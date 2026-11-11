// SPDX-License-Identifier: AGPL-3.0-only
//
// Pins and the files they name: a pin is a TOML file under `packaging/`, and
// everything it names is fetched by hash.

use std::{io::Read, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Puts `url` at `dest` unless a file with the right hash is already there, and
/// refuses anything else: these are unsigned files from third parties.
pub fn fetch(url: &str, want: &str, dest: &Path) -> Result<()> {
    if dest.is_file() && sha256(dest)? == want {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    println!("==> fetching {url}");
    let partial = dest.with_extension("partial");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&partial)
        .arg(url)
        .status()
        .context("run curl")?;
    if !status.success() {
        bail!("download failed: {url}");
    }
    let got = sha256(&partial)?;
    if got != want {
        std::fs::remove_file(&partial)?;
        bail!("{url} hashes to {got}, expected {want}");
    }
    std::fs::rename(&partial, dest)?;
    Ok(())
}
