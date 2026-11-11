// SPDX-License-Identifier: AGPL-3.0-only
//
// Assembles a release bundle: the server and everything it would otherwise need
// from the host, laid out as `noflex_core::bundle` looks for it.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{
    discover_plugins,
    download::{fetch, read_toml},
    ffmpeg,
};

#[derive(Debug, Deserialize)]
struct ModelPin {
    base_url: String,
    dir: String,
    file: Vec<ModelFile>,
}

#[derive(Debug, Deserialize)]
struct ModelFile {
    source: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct RuntimePin {
    base_url: String,
    target: std::collections::BTreeMap<String, RuntimeAsset>,
}

#[derive(Debug, Deserialize)]
struct RuntimeAsset {
    asset: String,
    sha256: String,
}

/// Downloads the pinned embedding model into `assets`.
pub fn model_fetch(root: &Path, assets: &Path) -> Result<()> {
    let pin: ModelPin = read_toml(&root.join("packaging/model.toml"))?;
    for file in &pin.file {
        let name = file
            .source
            .rsplit('/')
            .next()
            .context("a model file has an empty path")?;
        fetch(
            &format!("{}/{}", pin.base_url, file.source),
            &file.sha256,
            &assets.join(&pin.dir).join(name),
        )?;
    }
    Ok(())
}

/// Unpacks the pinned ONNX Runtime's shared libraries into `out/lib`, keeping its
/// license. The Windows archive also holds import libraries and half a gigabyte
/// of debug symbols, which nothing loads.
fn runtime_fetch(root: &Path, target: &str, cache: &Path, out: &Path) -> Result<()> {
    let pin: RuntimePin = read_toml(&root.join("packaging/onnxruntime.toml"))?;
    let asset = pin
        .target
        .get(target)
        .with_context(|| format!("no ONNX Runtime pinned for `{target}`"))?;
    let archive = cache.join(&asset.asset);
    fetch(
        &format!("{}/{}", pin.base_url, asset.asset),
        &asset.sha256,
        &archive,
    )?;
    let unpacked = cache.join("onnxruntime-unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked)?;
    let status = Command::new("tar")
        .arg("xf")
        .arg(&archive)
        .arg("--strip-components=1")
        .arg("-C")
        .arg(&unpacked)
        .status()
        .context("run tar")?;
    if !status.success() {
        bail!("could not unpack {}", archive.display());
    }
    std::fs::create_dir_all(out.join("lib"))?;
    for entry in std::fs::read_dir(unpacked.join("lib"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let shared = name.contains(".so") || name.ends_with(".dylib") || name.ends_with(".dll");
        if entry.file_type()?.is_dir() || !shared {
            continue;
        }
        let dest = out.join("lib").join(entry.file_name());
        let _ = std::fs::remove_file(&dest);
        run_cp(&entry.path(), &dest)?;
    }
    run_cp(&unpacked.join("LICENSE"), &out.join("LICENSE"))?;
    Ok(())
}

/// `cp -P`, because a library's symlinks have to stay symlinks.
fn run_cp(from: &Path, to: &Path) -> Result<()> {
    let status = Command::new("cp")
        .args(["-RP"])
        .arg(from)
        .arg(to)
        .status()
        .context("run cp")?;
    if !status.success() {
        bail!("could not copy {} to {}", from.display(), to.display());
    }
    Ok(())
}

/// Options for [`bundle`].
pub struct BundleOptions {
    pub target: String,
    /// Reuse a finished `ffmpeg-build` rather than compiling one.
    pub ffmpeg_dir: Option<PathBuf>,
    /// Embed this build of the web client rather than building one, so every
    /// platform's bundle serves the same one.
    pub web_dir: Option<PathBuf>,
    /// Cargo profile for the server. Release takes minutes of link-time
    /// optimization, which a layout check doesn't need.
    pub profile: String,
}

/// Builds `dist/noflex-<target>/` and its `.tar.zst`, or `.zip` for Windows. Linux
/// targets link through cargo-zigbuild against glibc 2.28, matching the ffmpeg
/// image, because a binary linked in the Nix shell would only run on a machine with
/// that Nix store. macOS and Windows targets need their own toolchains, and so
/// their own runners: see `release.yml` and `macos.yml` in `.github/workflows/`.
pub fn bundle(root: &Path, options: &BundleOptions) -> Result<()> {
    let target = options.target.as_str();
    let dist = root.join("dist");
    let cache = dist.join("cache");
    let out = dist.join(format!("noflex-{target}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    std::fs::create_dir_all(&cache)?;

    let web_dir = match &options.web_dir {
        Some(dir) => std::path::absolute(dir)?,
        None => {
            println!("==> building the web client");
            let web = root.join("web");
            run_in(&web, "npm", &["ci", "--no-audit", "--no-fund"])?;
            run_in(&web, "npm", &["run", "build"])?;
            web.join("build")
        }
    };

    println!("==> building noflex-server for {target}");
    let backend = root.join("backend");
    let linux = target.contains("linux");
    let cargo_target = if linux {
        format!("{target}.2.28")
    } else {
        target.to_string()
    };
    let subcommand = if linux { "zigbuild" } else { "build" };
    let status = Command::new("cargo")
        .current_dir(&backend)
        .args([subcommand, "-p", "noflex-server", "--target", &cargo_target])
        .args(["--profile", &options.profile])
        .env("NOFLEX_WEB_DIR", &web_dir)
        .status()
        .context("run cargo")?;
    if !status.success() {
        bail!("building noflex-server failed");
    }
    let profile_dir = match options.profile.as_str() {
        "dev" => "debug",
        other => other,
    };
    let exe = format!("noflex-server{}", exe_suffix(target));
    run_cp(
        &backend
            .join("target")
            .join(target)
            .join(profile_dir)
            .join(&exe),
        &out.join(&exe),
    )?;

    println!("==> building plugins");
    crate::plugin_build(&[])?;
    let plugins = out.join("plugins");
    std::fs::create_dir_all(&plugins)?;
    for plugin in discover_plugins()? {
        std::fs::copy(
            plugin.output_wasm()?,
            plugins.join(format!("{}.wasm", plugin.manifest.plugin_id)),
        )?;
    }

    let ffmpeg_dest = out.join("ffmpeg");
    let pin = ffmpeg::Pin::load(root)?;
    match &options.ffmpeg_dir {
        Some(dir) => run_cp(dir, &ffmpeg_dest)?,
        None => ffmpeg::build(root, &pin, target, &ffmpeg_dest)?,
    }
    if target == host_target() {
        ffmpeg::verify(&pin, target, &ffmpeg_dest)?;
    } else {
        println!("==> not verifying ffmpeg: {target} doesn't run here");
    }

    runtime_fetch(root, target, &cache, &out.join("onnxruntime"))?;
    // Fetched once into the cache, then copied: it's the slow part of a rebuild.
    let model_cache = cache.join("assets");
    model_fetch(root, &model_cache)?;
    run_cp(&model_cache, &out.join("assets"))?;

    if linux {
        let service = out.join("linux");
        std::fs::create_dir_all(&service)?;
        for file in LINUX_SERVICE {
            std::fs::copy(root.join("packaging/linux").join(file), service.join(file))?;
        }
    }

    std::fs::copy(root.join("LICENSE"), out.join("LICENSE"))?;
    std::fs::write(out.join("README.txt"), NOTICE)?;

    let mut tar = Command::new("tar");
    tar.current_dir(&dist);
    let archive = if target.contains("windows") {
        // Windows' own tar, which Command finds before Git's, writes zips.
        let archive = format!("noflex-{target}.zip");
        tar.args(["-a", "-cf", &archive]);
        archive
    } else {
        let archive = format!("noflex-{target}.tar.zst");
        // Not `--zstd`, which macOS's tar may lack.
        tar.args(["--use-compress-program=zstd", "-cf", &archive]);
        archive
    };
    let status = tar
        .arg(format!("noflex-{target}"))
        .status()
        .context("run tar")?;
    if !status.success() {
        bail!("could not archive the bundle");
    }
    println!("==> {}", dist.join(archive).display());
    Ok(())
}

/// What runs a Linux bundle as a system service, from `packaging/linux/`.
const LINUX_SERVICE: [&str; 3] = ["noflex.service", "noflex.sysusers", "config.toml"];

const NOTICE: &str = "\
NoFlex, AGPL-3.0-only: https://github.com/NoFlexMedia/noflex

Run ./noflex-server, then open http://127.0.0.1:8096. docs/PACKAGING.md in the
source explains what each hardware accelerator needs from this machine, and
docs/DEPLOYMENT.md how to run this as a service, with what's in linux/ on Linux.

Third-party software in this bundle, under its own license:

  ffmpeg/        FFmpeg, GPL-3.0 (ffmpeg/LICENSE.txt). Built from the commit and
                 build environment in packaging/ffmpeg/ffmpeg.toml.
  onnxruntime/   ONNX Runtime, MIT (onnxruntime/LICENSE).
  assets/        EmbeddingGemma 2, Apache-2.0, as packaged by the export named in
                 packaging/model.toml.
";

fn run_in(dir: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .current_dir(dir)
        .args(args)
        .status()
        .with_context(|| format!("run {program}"))?;
    if !status.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    Ok(())
}

fn exe_suffix(target: &str) -> &'static str {
    if target.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

fn host_target() -> String {
    let (arch, os) = (std::env::consts::ARCH, std::env::consts::OS);
    match os {
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        _ => format!("{arch}-unknown-linux-gnu"),
    }
}
