// SPDX-License-Identifier: AGPL-3.0-only

//! Runtime hardware-acceleration probe. We ask the ffmpeg binary
//! what it can do (`-hwaccels`, `-encoders`) and pair that with a
//! `/dev/dri` scan for a VAAPI render node. The result is a
//! plain-data snapshot the server resolves into a single
//! [`HwBackend`](crate::encoder::HwBackend) at startup — detection
//! never blocks or fails startup, so a misconfigured ffmpeg just
//! means we fall back to software.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

/// Snapshot of what the local ffmpeg can actually do. Built by
/// [`detect`]; consumed by the server's backend-resolution logic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HwDetect {
    /// Lines from `ffmpeg -hwaccels` (e.g. `vaapi`, `qsv`).
    pub hwaccels: Vec<String>,
    /// Encoder names from `ffmpeg -encoders` (e.g. `h264_nvenc`).
    pub encoders: Vec<String>,
    /// Resolved VAAPI render node, if any.
    pub vaapi_device: Option<PathBuf>,
}

impl HwDetect {
    pub fn has_vaapi(&self) -> bool {
        self.hwaccels.iter().any(|h| h == "vaapi") && self.vaapi_device.is_some()
    }

    pub fn has_qsv(&self) -> bool {
        self.hwaccels.iter().any(|h| h == "qsv")
    }

    pub fn has_videotoolbox(&self) -> bool {
        self.hwaccels.iter().any(|h| h == "videotoolbox")
    }

    pub fn has_nvenc(&self) -> bool {
        self.encoders.iter().any(|e| e == "h264_nvenc")
    }

    pub fn has_mf(&self) -> bool {
        self.encoders.iter().any(|e| e == "h264_mf")
    }
}

/// Probe the local ffmpeg for hardware-acceleration support. Every
/// failure mode (missing binary, nonzero exit, unparseable output)
/// collapses to an empty snapshot — hardware detection must never
/// prevent startup.
pub async fn detect(ffmpeg: &Path) -> HwDetect {
    let hwaccels = run_capture(ffmpeg, &["-hide_banner", "-hwaccels"])
        .await
        .as_deref()
        .map(parse_hwaccels)
        .unwrap_or_default();
    let encoders = run_capture(ffmpeg, &["-hide_banner", "-encoders"])
        .await
        .as_deref()
        .map(parse_encoder_names)
        .unwrap_or_default();
    HwDetect {
        hwaccels,
        encoders,
        vaapi_device: find_vaapi_device(),
    }
}

/// Run ffmpeg with `args`, returning captured stdout on success.
async fn run_capture(ffmpeg: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new(ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Locate a usable VAAPI render node: `/dev/dri/renderD128` when
/// present, otherwise the lowest-numbered `renderD*` under
/// `/dev/dri`. `None` when there's no DRI device at all.
fn find_vaapi_device() -> Option<PathBuf> {
    let primary = Path::new("/dev/dri/renderD128");
    if primary.exists() {
        return Some(primary.to_path_buf());
    }
    // renderD128 was absent; scan /dev/dri for the lowest-numbered
    // render node that does exist.
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in std::fs::read_dir("/dev/dri").ok()?.flatten() {
        let Some(num) = entry
            .file_name()
            .to_string_lossy()
            .strip_prefix("renderD")
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _)| num < *b) {
            best = Some((num, entry.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// Parse `ffmpeg -hwaccels` output: every trimmed, non-blank line
/// after the `Hardware acceleration methods:` header.
pub fn parse_hwaccels(out: &str) -> Vec<String> {
    let mut started = false;
    let mut accels = Vec::new();
    for line in out.lines() {
        if !started {
            if line.contains("Hardware acceleration methods:") {
                started = true;
            }
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            accels.push(trimmed.to_string());
        }
    }
    accels
}

/// Parse `ffmpeg -encoders` output: the second whitespace token of
/// each line is the encoder name (the first is the flag column).
/// Header, separator, and legend (`<flags> = <desc>`) lines are
/// skipped.
pub fn parse_encoder_names(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _flags = parts.next()?;
            let name = parts.next()?;
            // Legend lines are "<flags> = <meaning>"; the name
            // token there is literally "=".
            (name != "=").then(|| name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HWACCELS_SAMPLE: &str = "\
Hardware acceleration methods:
cuda
vaapi
qsv
dxva2
vdpau
videotoolbox
";

    const ENCODERS_SAMPLE: &str = "\
Encoders:
 V..... = Video encoder
 A..... = Audio encoder
 S..... = Subtitle encoder
 ....E. = Marked as 'encode-only' (cannot decode)
 -------
 V....D a64multi             Multicolor charset for C64 (codec a64_multi)
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 A....D aac                  AAC (Advanced Audio Coding)
 V....D h264_vaapi           H.264/AVC (VAAPI) (codec h264)
 V....D h264_mf              Microsoft Media Foundation H.264 (codec h264)
";

    #[test]
    fn parse_hwaccels_skips_header_and_blanks() {
        let accels = parse_hwaccels(HWACCELS_SAMPLE);
        assert_eq!(
            accels,
            ["cuda", "vaapi", "qsv", "dxva2", "vdpau", "videotoolbox"]
        );
    }

    #[test]
    fn parse_hwaccels_no_header_is_empty() {
        assert!(parse_hwaccels("nothing useful\nhere\n").is_empty());
    }

    #[test]
    fn parse_encoder_names_extracts_second_token() {
        let names = parse_encoder_names(ENCODERS_SAMPLE);
        assert!(names.contains(&"h264_nvenc".to_string()));
        assert!(names.contains(&"h264_vaapi".to_string()));
        assert!(names.contains(&"h264_mf".to_string()));
        assert!(names.contains(&"aac".to_string()));
        // Legend lines (token "=") are skipped.
        assert!(!names.iter().any(|n| n == "="));
    }

    #[test]
    fn parse_encoder_names_skips_header_and_separator() {
        // Header + separator only: no encoder names.
        assert!(parse_encoder_names("Encoders:\n -------\n").is_empty());
    }

    #[test]
    fn has_predicates_reflect_parsed_fields() {
        let det = HwDetect {
            hwaccels: parse_hwaccels(HWACCELS_SAMPLE),
            encoders: parse_encoder_names(ENCODERS_SAMPLE),
            vaapi_device: Some(PathBuf::from("/dev/dri/renderD128")),
        };
        assert!(det.has_vaapi());
        assert!(det.has_qsv());
        assert!(det.has_videotoolbox());
        assert!(det.has_nvenc());
        assert!(det.has_mf());
    }

    #[test]
    fn has_vaapi_requires_device() {
        // vaapi method present but no render node: not usable.
        let det = HwDetect {
            hwaccels: vec!["vaapi".to_string()],
            encoders: vec![],
            vaapi_device: None,
        };
        assert!(!det.has_vaapi());
    }

    #[tokio::test]
    async fn detect_missing_binary_yields_empty_lists() {
        // vaapi_device is host-dependent, so we only assert the
        // ffmpeg-parsed fields collapse to empty on a missing binary.
        let det = detect(Path::new("/nonexistent/noflex-ffmpeg-probe")).await;
        assert!(det.hwaccels.is_empty());
        assert!(det.encoders.is_empty());
    }
}
