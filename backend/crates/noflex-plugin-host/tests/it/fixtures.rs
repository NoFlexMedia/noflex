// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::PathBuf, process::Command};

/// The component for the fixture guest in `tests/fixtures/<name>/`,
/// building it first if the artifact is missing.
///
/// Returns `None` when the build fails — typically no `wasm32-wasip2`
/// toolchain — so the caller can skip instead of failing.
pub fn wasm(name: &str) -> Option<PathBuf> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let wasm = workspace.join(format!(
        "target/wasm32-wasip2/release/noflex_fixture_{}.wasm",
        name.replace('-', "_")
    ));
    if wasm.is_file() {
        return Some(wasm);
    }
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .arg("--package")
        .arg(format!("noflex-fixture-{name}"))
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .status()
        .ok()?;
    (status.success() && wasm.is_file()).then_some(wasm)
}
