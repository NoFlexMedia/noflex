// SPDX-License-Identifier: AGPL-3.0-only
//
// Identify the ffmpeg/ffprobe binaries NoFlex will actually invoke.
// "ffmpeg not found" and "wrong ffmpeg on PATH" are the most common
// transcoding support questions, so the admin view shows the resolved
// path, version line and build flags rather than a yes/no.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use noflex_graphql::admin_status::BinaryInfo;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Probe `command` (a bare name searched on `PATH`, or a path).
/// Never fails; problems land in [`BinaryInfo::error`].
pub async fn probe_binary(command: &Path, override_env: &str) -> BinaryInfo {
    let mut info = BinaryInfo {
        command: command.display().to_string(),
        override_env: override_env.to_owned(),
        ..BinaryInfo::default()
    };
    let Some(path) = resolve(command) else {
        info.error = Some(format!("`{}` not found on PATH", command.display()));
        return info;
    };
    info.path = Some(path.display().to_string());
    match run_capture(&path, &["-hide_banner", "-version"]).await {
        Ok(out) => {
            let (version, flags) = parse_version(&out);
            info.version = version;
            info.build_flags = flags;
        }
        Err(e) => info.error = Some(e),
    }
    info
}

/// `ffmpeg -decoders` names; empty when the probe fails.
pub async fn decoder_names(ffmpeg: &Path) -> Vec<String> {
    run_capture(ffmpeg, &["-hide_banner", "-decoders"])
        .await
        .map(|out| noflex_transcoder::hwdetect::parse_encoder_names(&out))
        .unwrap_or_default()
}

/// Resolve a command the way `Command::new` will: paths as-is, bare
/// names through `PATH`.
pub fn resolve(command: &Path) -> Option<PathBuf> {
    if command.components().count() > 1 || command.is_absolute() {
        return command.is_file().then(|| command.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

async fn run_capture(bin: &Path, args: &[&str]) -> Result<String, String> {
    let child = tokio::process::Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(PROBE_TIMEOUT, child)
        .await
        .map_err(|_| format!("`{}` timed out after {PROBE_TIMEOUT:?}", bin.display()))?
        .map_err(|e| format!("running `{}`: {e}", bin.display()))?;
    if !output.status.success() {
        return Err(format!("`{}` exited with {}", bin.display(), output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// First line of `-version`, plus the `--enable-*` flags from its
/// `configuration:` line.
pub fn parse_version(out: &str) -> (Option<String>, Vec<String>) {
    let version = out
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned);
    let flags = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("configuration:"))
        .map(|cfg| {
            cfg.split_whitespace()
                .filter(|f| f.starts_with("--enable-"))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    (version, flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_line_and_enable_flags() {
        let out = "ffmpeg version 7.1 Copyright (c) 2000-2024 the FFmpeg developers\n\
                   built with gcc 14\n\
                   configuration: --prefix=/nix --enable-gpl --enable-libx264 --disable-doc --enable-vaapi\n\
                   libavutil      59. 39.100 / 59. 39.100\n";
        let (version, flags) = parse_version(out);
        assert_eq!(
            version.as_deref(),
            Some("ffmpeg version 7.1 Copyright (c) 2000-2024 the FFmpeg developers")
        );
        assert_eq!(
            flags,
            ["--enable-gpl", "--enable-libx264", "--enable-vaapi"]
        );
    }

    #[test]
    fn empty_output_has_no_version() {
        assert_eq!(parse_version(""), (None, Vec::new()));
    }

    #[tokio::test]
    async fn missing_binary_reports_error_not_panic() {
        let info = probe_binary(Path::new("noflex-definitely-missing-ffmpeg"), "FFMPEG_BIN").await;
        assert!(info.path.is_none());
        assert!(info.error.is_some());
        assert_eq!(info.override_env, "FFMPEG_BIN");
    }

    #[test]
    fn resolve_finds_bare_names_on_path() {
        assert!(resolve(Path::new("sh")).is_some());
        assert!(resolve(Path::new("/nonexistent/ffmpeg")).is_none());
    }
}
