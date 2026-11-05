// SPDX-License-Identifier: AGPL-3.0-only

//! Where a release bundle keeps what it ships beside the server binary. A build
//! run from source or a Nix store has none of it, and every lookup falls through
//! to the host's.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// The directory holding the running executable, when it can be determined.
fn exe_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        std::env::current_exe()
            .ok()?
            .canonicalize()
            .ok()?
            .parent()
            .map(Path::to_path_buf)
    })
    .as_deref()
}

/// `name` (`"plugins"`, `"assets"`) beside the executable, if the bundle has it.
pub fn dir(name: &str) -> Option<PathBuf> {
    exe_dir()
        .map(|dir| dir.join(name))
        .filter(|dir| dir.is_dir())
}

/// How to invoke an ffmpeg program (`"ffmpeg"`, `"ffprobe"`): the path in
/// `override_env` if set, else the bundled copy, else the bare name for `PATH` to
/// resolve. The result may not exist; running it is what reports that.
pub fn ffmpeg_tool(name: &str, override_env: &str) -> PathBuf {
    resolve_tool(
        name,
        std::env::var_os(override_env).map(PathBuf::from),
        exe_dir(),
    )
}

fn resolve_tool(name: &str, overridden: Option<PathBuf>, exe_dir: Option<&Path>) -> PathBuf {
    overridden
        .or_else(|| {
            let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
            let bundled = exe_dir?.join("ffmpeg").join("bin").join(file);
            bundled.is_file().then_some(bundled)
        })
        .unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_prefers_the_override_then_the_bundle_then_path() {
        let exe_dir = tempfile::tempdir().expect("tempdir");
        let dir = Some(exe_dir.path());

        assert_eq!(resolve_tool("ffmpeg", None, dir), PathBuf::from("ffmpeg"));

        let bin = exe_dir.path().join("ffmpeg/bin");
        std::fs::create_dir_all(&bin).expect("bin dir");
        let bundled = bin.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&bundled, b"").expect("bundled tool");
        assert_eq!(resolve_tool("ffmpeg", None, dir), bundled);
        assert_eq!(
            resolve_tool("ffprobe", None, dir),
            PathBuf::from("ffprobe"),
            "ffprobe isn't in this bundle"
        );

        let chosen = PathBuf::from("/opt/ffmpeg");
        assert_eq!(resolve_tool("ffmpeg", Some(chosen.clone()), dir), chosen);
    }
}
