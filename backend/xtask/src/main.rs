// SPDX-License-Identifier: AGPL-3.0-only
//
// `cargo xtask <command>` task runner. Lives outside the main workspace
// build path (added as a workspace member so it's discoverable, but its
// commands invoke nested cargo processes against `plugins/<name>/`).
//
// Available commands:
//   - `plugin-build [<name>...]` — compile first-party plugins to
//     `wasm32-wasip2` components and embed the `noflex:manifest` custom
//     section. With no arguments, builds every plugin under `plugins/`.
//   - `plugin-dev <name>` — same as plugin-build, then symlinks the
//     output into `$XDG_CONFIG_HOME/noflex/plugins/` so the running
//     server picks it up via hot-reload.
//   - `ffmpeg-build <target> <out>` — compile the pinned ffmpeg for a target.
//   - `ffmpeg-verify <target> <dir>` — check an ffmpeg has what we require.
//   - `model-fetch <assets_dir>` — download the pinned embedding model.
//   - `bundle <target>` — assemble a release bundle under `dist/`.
//   - `third-party check|sync|source <out>` — the third-party code the apps
//     ship, as `packaging/third-party/apps.toml` records it.

mod bundle;
mod download;
mod ffmpeg;
mod third_party;

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

// Sync wasip2 stack — stable rustc target, no `-Z build-std` needed.
const WASI_TARGET: &str = "wasm32-wasip2";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("plugin-build") => plugin_build(&args[1..]),
        Some("plugin-dev") => plugin_dev(&args[1..]),
        Some("ffmpeg-build") => ffmpeg_command(&args[1..], ffmpeg::build),
        Some("ffmpeg-verify") => ffmpeg_command(&args[1..], |_, pin, target, dir| {
            ffmpeg::verify(pin, target, dir)
        }),
        Some("model-fetch") => model_fetch(&args[1..]),
        Some("bundle") => bundle_command(&args[1..]),
        Some("third-party") => third_party_command(&args[1..]),
        Some(other) => {
            eprintln!("xtask: unknown command `{other}`");
            print_help();
            return ExitCode::from(2);
        }
        None => {
            print_help();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    eprintln!(
        r#"xtask — NoFlex repo-local task runner

USAGE:
    cargo xtask <command> [args...]

COMMANDS:
    plugin-build [<name>...]   Build first-party WASM plugins. With no
                               arguments builds everything under plugins/.
    plugin-dev <name>          Build a plugin and symlink it into the
                               user plugin dir for hot-reload pickup.
    ffmpeg-build <target> <out>
                               Compile the ffmpeg pinned in
                               packaging/ffmpeg/ffmpeg.toml. Needs Docker or
                               Podman.
    ffmpeg-verify <target> <dir>
                               Check the ffmpeg in <dir> has every encoder,
                               filter and muxer that target requires.
    model-fetch <assets_dir>   Download the embedding model pinned in
                               packaging/model.toml, checking its hashes.
    bundle <target> [--ffmpeg <dir>] [--web <dir>] [--profile <profile>]
                               Assemble dist/noflex-<target>/ and its archive:
                               the server, plugins, ffmpeg, ONNX Runtime and the
                               model. --ffmpeg reuses a finished ffmpeg-build,
                               --web a built web client.
    third-party check          Check packaging/third-party/apps.toml against
                               the app's media_kit and the license assets.
    third-party sync           Write the apps' license assets from it.
    third-party source <out>   Gather the pinned sources it names into <out>.

Requires the `wasm32-wasip2` rustc target and `wasm-tools` on PATH (both
provided by `nix develop`)."#
    );
}

fn third_party_command(args: &[String]) -> Result<()> {
    let root = checkout_root()?;
    match args {
        [command] if command == "check" => third_party::check(&root),
        [command] if command == "sync" => third_party::sync(&root),
        [command, out] if command == "source" => third_party::source(&root, Path::new(out)),
        _ => bail!("expected check, sync, or source <out>"),
    }
}

/// Runs an ffmpeg subcommand taking `<target> <dir>`.
fn ffmpeg_command(
    args: &[String],
    run: impl FnOnce(&Path, &ffmpeg::Pin, &str, &Path) -> Result<()>,
) -> Result<()> {
    let [target, dir] = args else {
        bail!("expected <target> <dir>");
    };
    let root = checkout_root()?;
    run(&root, &ffmpeg::Pin::load(&root)?, target, Path::new(dir))
}

fn model_fetch(args: &[String]) -> Result<()> {
    let [assets] = args else {
        bail!("expected <assets_dir>");
    };
    bundle::model_fetch(&checkout_root()?, Path::new(assets))
}

fn bundle_command(args: &[String]) -> Result<()> {
    let Some((target, rest)) = args.split_first() else {
        bail!("expected <target>");
    };
    let mut options = bundle::BundleOptions {
        target: target.clone(),
        ffmpeg_dir: None,
        web_dir: None,
        profile: "release".to_string(),
    };
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        let value = rest
            .next()
            .with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--ffmpeg" => options.ffmpeg_dir = Some(PathBuf::from(value)),
            "--web" => options.web_dir = Some(PathBuf::from(value)),
            "--profile" => options.profile = value.clone(),
            other => bail!("unknown option `{other}`"),
        }
    }
    bundle::bundle(&checkout_root()?, &options)
}

/// The directory holding `packaging/`, one level above the Cargo workspace that
/// [`repo_root`] finds.
fn checkout_root() -> Result<PathBuf> {
    Ok(repo_root()?
        .parent()
        .context("the Cargo workspace has no parent")?
        .to_path_buf())
}

fn plugin_build(names: &[String]) -> Result<()> {
    let plugins = discover_plugins()?;
    let targets: Vec<&PluginDef> = if names.is_empty() {
        plugins.iter().collect()
    } else {
        names
            .iter()
            .map(|n| {
                plugins
                    .iter()
                    .find(|p| &p.crate_name == n || &p.manifest.plugin_id == n)
                    .ok_or_else(|| anyhow::anyhow!("no plugin named `{n}` under plugins/"))
            })
            .collect::<Result<Vec<_>>>()?
    };

    for plugin in targets {
        println!("==> building {} ({})", plugin.crate_name, WASI_TARGET);
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "--target",
                WASI_TARGET,
                "--manifest-path",
            ])
            .arg(&plugin.manifest_path)
            .status()
            .with_context(|| format!("invoke cargo for {}", plugin.crate_name))?;
        if !status.success() {
            bail!("cargo build failed for {}", plugin.crate_name);
        }

        // The manifest is embedded by the plugin source itself via
        // `#[link_section = "noflex:manifest"]` on a static byte array
        // backed by `manifest.toml`. No post-build step is needed.
        let wasm = plugin.output_wasm()?;
        println!("   {}", wasm.display());
    }

    Ok(())
}

fn plugin_dev(names: &[String]) -> Result<()> {
    if names.is_empty() {
        bail!("plugin-dev requires a plugin name");
    }
    plugin_build(names)?;

    let plugins = discover_plugins()?;
    let user_dir = user_plugin_dir()?;
    std::fs::create_dir_all(&user_dir)
        .with_context(|| format!("create user plugin dir {}", user_dir.display()))?;
    for name in names {
        let plugin = plugins
            .iter()
            .find(|p| &p.crate_name == name || &p.manifest.plugin_id == name)
            .ok_or_else(|| anyhow::anyhow!("no plugin named `{name}` under plugins/"))?;
        let wasm = plugin.output_wasm()?;
        let dest = user_dir.join(format!("{}.wasm", plugin.manifest.plugin_id));
        let _ = std::fs::remove_file(&dest);
        std::os::unix::fs::symlink(&wasm, &dest)
            .with_context(|| format!("symlink {} -> {}", wasm.display(), dest.display()))?;
        println!("==> linked {} -> {}", dest.display(), wasm.display());
    }
    Ok(())
}

fn user_plugin_dir() -> Result<PathBuf> {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("noflex").join("plugins"));
    }
    let home = env::var("HOME").context("HOME unset; cannot resolve user plugin dir")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("noflex")
        .join("plugins"))
}

#[derive(Debug)]
struct PluginDef {
    crate_name: String,
    manifest_path: PathBuf,
    target_dir_relative: PathBuf,
    manifest: PluginManifestSnippet,
}

impl PluginDef {
    fn output_wasm(&self) -> Result<PathBuf> {
        // Per-plugin cargo invocations write to `<plugin>/target/<triple>/release/<name>.wasm`.
        let parent = self
            .manifest_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("plugin manifest has no parent"))?;
        let wasm = parent.join(&self.target_dir_relative);
        if !wasm.is_file() {
            bail!(
                "expected wasm at {} after build — did the cargo step succeed?",
                wasm.display()
            );
        }
        Ok(wasm)
    }
}

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
struct PluginManifestSnippet {
    plugin_id: String,
    display_name: String,
    #[serde(default)]
    hooks: Vec<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    http_egress_hosts: Vec<String>,
    #[serde(default)]
    secrets_read: Vec<String>,
    #[serde(default)]
    priority: toml::value::Table,
    #[serde(default)]
    deadlines_secs: toml::value::Table,
    #[serde(default)]
    memory_limit_mb: Option<u32>,
    // Set by xtask from the crate's `[package].version`.
    #[serde(default = "default_version")]
    version: String,
    // The host's expected API version. Stamped in here so plugins built
    // against an older noflex are rejected at load time.
    #[serde(default = "default_api_version")]
    noflex_api_version: String,
    // Schema version of the manifest itself. Bumped when fields shift.
    #[serde(default = "default_manifest_version")]
    manifest_version: u32,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
}

fn default_version() -> String {
    "0.0.0".into()
}
fn default_api_version() -> String {
    "0.2".into()
}
fn default_manifest_version() -> u32 {
    1
}

fn discover_plugins() -> Result<Vec<PluginDef>> {
    let plugins_dir = repo_root()?.join("plugins");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&plugins_dir)
        .with_context(|| format!("read plugins dir {}", plugins_dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let cargo_toml = path.join("Cargo.toml");
        if !cargo_toml.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&cargo_toml)
            .with_context(|| format!("read {}", cargo_toml.display()))?;
        let parsed: CrateToml =
            toml::from_str(&raw).with_context(|| format!("parse {}", cargo_toml.display()))?;
        let crate_name = parsed.package.name.clone();
        let mut manifest = parsed
            .package
            .metadata
            .and_then(|m| m.noflex_plugin)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} is missing [package.metadata.noflex-plugin]",
                    cargo_toml.display()
                )
            })?;
        manifest.version = parsed.package.version.clone();
        let target_subpath: PathBuf = format!(
            "target/{}/release/{}.wasm",
            WASI_TARGET,
            crate_name.replace('-', "_")
        )
        .into();
        out.push(PluginDef {
            crate_name,
            manifest_path: cargo_toml,
            target_dir_relative: target_subpath,
            manifest,
        });
    }
    Ok(out)
}

fn repo_root() -> Result<PathBuf> {
    // xtask runs from the repo root via `cargo xtask`, so CARGO_MANIFEST_DIR
    // points at xtask/. Step up one.
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR unset (run via `cargo xtask`)")?;
    PathBuf::from(manifest_dir)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no parent"))
}

#[derive(Debug, Deserialize)]
struct CrateToml {
    package: CratePackage,
}

#[derive(Debug, Deserialize)]
struct CratePackage {
    name: String,
    version: String,
    #[serde(default)]
    metadata: Option<CrateMetadata>,
}

#[derive(Debug, Deserialize)]
struct CrateMetadata {
    #[serde(rename = "noflex-plugin")]
    noflex_plugin: Option<PluginManifestSnippet>,
}
