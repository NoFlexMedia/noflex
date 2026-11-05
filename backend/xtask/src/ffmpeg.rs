// SPDX-License-Identifier: AGPL-3.0-only
//
// Builds and checks the ffmpeg a release bundle ships, as pinned in
// `packaging/ffmpeg/ffmpeg.toml`.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Pin {
    ffmpeg_commit: String,
    nvenc_sdk: String,
    target: BTreeMap<String, TargetPin>,
    required: Required,
}

#[derive(Debug, Deserialize)]
struct TargetPin {
    /// The target in `packaging/ffmpeg/pipeline`. None builds natively; see
    /// `build-macos.sh`.
    pipeline: Option<String>,
    #[serde(default)]
    encoders: Vec<String>,
    #[serde(default)]
    filters: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Required {
    encoders: Vec<String>,
    filters: Vec<String>,
    muxers: Vec<String>,
}

impl Pin {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("packaging/ffmpeg/ffmpeg.toml");
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    fn target(&self, target: &str) -> Result<&TargetPin> {
        self.target.get(target).with_context(|| {
            let known = self.target.keys().cloned().collect::<Vec<_>>().join(", ");
            format!("no ffmpeg pinned for `{target}` (have: {known})")
        })
    }
}

/// Compile the pinned ffmpeg for `target` into `out`, as `bin/`, `lib/` and
/// `LICENSE.txt`. Targets with a pipeline need Docker or Podman, and an hour or more
/// the first time, while it compiles every dependency; the image is then reused.
/// The others build natively, so must be built on the target.
pub fn build(root: &Path, pin: &Pin, target: &str, out: &Path) -> Result<()> {
    println!("==> building ffmpeg {} for {target}", pin.ffmpeg_commit);
    let scripts = root.join("packaging/ffmpeg");
    let mut command = match &pin.target(target)?.pipeline {
        Some(pipeline) => {
            let images = Command::new(scripts.join("build-images.sh"))
                .arg(pipeline)
                .output()
                .context("run packaging/ffmpeg/build-images.sh")?;
            std::io::Write::write_all(&mut std::io::stderr(), &images.stderr)?;
            if !images.status.success() {
                std::io::Write::write_all(&mut std::io::stderr(), &images.stdout)?;
                bail!("building the ffmpeg image failed for {target}");
            }
            // The script's last line of output names the image.
            let stdout = String::from_utf8_lossy(&images.stdout);
            let image = stdout
                .lines()
                .next_back()
                .context("build-images.sh named no image")?;
            let mut command = Command::new(scripts.join("build.sh"));
            command.args([image, &pin.ffmpeg_commit, &pin.nvenc_sdk]);
            command
        }
        None => {
            let mut command = Command::new(scripts.join("build-macos.sh"));
            command.arg(&pin.ffmpeg_commit);
            command
        }
    };
    let status = command.arg(out).status().context("run the ffmpeg build")?;
    if !status.success() {
        bail!("ffmpeg build failed for {target}");
    }
    Ok(())
}

/// Check the ffmpeg in `dir` has everything `target` requires. Runs the binary,
/// so `target` must be one this host can execute.
pub fn verify(pin: &Pin, target: &str, dir: &Path) -> Result<()> {
    let target_pin = pin.target(target)?;
    let ffmpeg = dir
        .join("bin")
        .join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
    let listing = |flag: &str| -> Result<BTreeSet<String>> {
        let output = Command::new(&ffmpeg)
            .args(["-hide_banner", flag])
            .output()
            .with_context(|| format!("run {}", ffmpeg.display()))?;
        if !output.status.success() {
            bail!("{} {flag} failed", ffmpeg.display());
        }
        Ok(listed_names(&String::from_utf8_lossy(&output.stdout)))
    };

    let mut missing = Vec::new();
    let mut check = |kind: &str, have: &BTreeSet<String>, want: &[&Vec<String>]| {
        for name in want.iter().flat_map(|list| list.iter()) {
            if !have.contains(name) {
                missing.push(format!("{kind} {name}"));
            }
        }
    };
    let required = &pin.required;
    check(
        "encoder",
        &listing("-encoders")?,
        &[&required.encoders, &target_pin.encoders],
    );
    check(
        "filter",
        &listing("-filters")?,
        &[&required.filters, &target_pin.filters],
    );
    check("muxer", &listing("-muxers")?, &[&required.muxers]);
    if !missing.is_empty() {
        bail!("{} lacks {}", ffmpeg.display(), missing.join(", "));
    }
    println!("==> {} has everything {target} requires", ffmpeg.display());
    Ok(())
}

/// The names in `ffmpeg -encoders`-style output, whose rows are flags, a name and
/// a description. The second word of every line is enough: headers and legends
/// contribute only names that nothing asks for.
fn listed_names(listing: &str) -> BTreeSet<String> {
    listing
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_names_reads_every_listing_shape() {
        let names = listed_names(
            " V....D libx264              libx264 H.264 / AVC\n\
             \x20T.. scale             V->V       Scale the input video size.\n\
             \x20 E hls             Apple HTTP Live Streaming\n",
        );
        assert!(names.contains("libx264") && names.contains("scale") && names.contains("hls"));
    }

    #[test]
    fn pin_covers_what_the_transcoder_can_ask_for() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let pin = Pin::load(&root).expect("ffmpeg.toml parses");
        let linux = pin
            .target("x86_64-unknown-linux-gnu")
            .expect("linux target");
        assert_eq!(linux.pipeline.as_deref(), Some("linux64"));
        let mac = pin.target("aarch64-apple-darwin").expect("macOS target");
        assert_eq!(mac.pipeline, None, "VideoToolbox needs a native build");
    }
}
