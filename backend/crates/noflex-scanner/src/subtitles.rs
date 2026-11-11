// SPDX-License-Identifier: AGPL-3.0-only
//
// Plex-convention sidecar subtitle discovery. Mirrors Plex's
// `.<lang>.<flag>.<flag>.<ext>` naming: a sidecar next to
// `Movie.mkv` named `Movie.en.sdh.srt` is parsed as an English
// hearing-impaired SubRip track, etc. The remainder parser is intentionally
// strict — any token it doesn't recognise causes the whole sidecar to be
// dropped, so torrent-junk filenames like `Movie.720p.srt` don't silently
// turn into a no-language track.

use std::{path::Path, sync::OnceLock};

use noflex_core::SubtitleCodec;
use noflex_storage::domain::SubtitleTrack;
use noflex_transcoder::probe::ProbeReport;
use regex::Regex;

const SIDECAR_EXTS: &[&str] = &["srt", "vtt", "ass", "ssa"];

/// Match the first remainder token as a language code: `en`, `eng`,
/// `pt-BR`, etc. Stored lowercase in the resulting [`SubtitleTrack`].
fn language_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[a-zA-Z]{2,3}(?:-[a-zA-Z]{2,4})?$").unwrap())
}

/// The container's own subtitle streams, in stream order, each tagged
/// with its position among them so the server can extract or burn it.
pub fn embedded_tracks(probe: &ProbeReport) -> Vec<SubtitleTrack> {
    probe
        .subtitle
        .iter()
        .filter(|s| !s.external)
        .enumerate()
        .map(|(position, s)| SubtitleTrack {
            language: s.language.clone(),
            codec: s.codec.as_str().to_string(),
            default: s.default,
            forced: s.forced,
            external: false,
            codec_profile: None,
            srt_compatible: !matches!(s.codec, SubtitleCodec::PgsImage | SubtitleCodec::DvbSub),
            image_count: None,
            title: s.title.clone(),
            path: None,
            stream_index: u32::try_from(position).ok(),
        })
        .collect()
}

/// Scan the video's parent directory for Plex-style sidecar subtitle files.
///
/// Best-effort: any IO failure (missing parent, permission denied, …) returns
/// an empty vector so the surrounding scan pipeline keeps going. Results are
/// sorted by absolute path for deterministic scan output.
pub(crate) fn discover_sidecars(video_path: &Path) -> Vec<SubtitleTrack> {
    let Some(parent) = video_path.parent() else {
        return Vec::new();
    };
    let Some(video_stem) = video_path.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };

    let entries = match std::fs::read_dir(parent) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        let ext_lower = ext.to_ascii_lowercase();
        if !SIDECAR_EXTS.contains(&ext_lower.as_str()) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        // Sidecar must either share the video's exact stem
        // (`Movie.srt`) or extend it with a `.` separator
        // (`Movie.en.srt`). Anything else is a different filename.
        let remainder = if stem == video_stem {
            ""
        } else if let Some(rest) = stem.strip_prefix(video_stem) {
            let Some(rest) = rest.strip_prefix('.') else {
                continue;
            };
            rest
        } else {
            continue;
        };

        let Some(codec) = codec_for_ext(&ext_lower) else {
            continue;
        };

        let parts: Vec<&str> = if remainder.is_empty() {
            Vec::new()
        } else {
            remainder.split('.').collect()
        };

        let Some(parsed) = parse_remainder(&parts) else {
            continue;
        };

        let abs_path = path
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string))
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        out.push(SubtitleTrack {
            language: parsed.language,
            codec: codec.as_str().to_string(),
            default: parsed.default,
            forced: parsed.forced,
            external: true,
            codec_profile: None,
            srt_compatible: matches!(codec, SubtitleCodec::SubRip | SubtitleCodec::WebVtt),
            image_count: None,
            title: parsed.title,
            path: Some(abs_path),
            stream_index: None,
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn codec_for_ext(ext: &str) -> Option<SubtitleCodec> {
    match ext {
        "srt" => Some(SubtitleCodec::SubRip),
        "vtt" => Some(SubtitleCodec::WebVtt),
        "ass" => Some(SubtitleCodec::Ass),
        "ssa" => Some(SubtitleCodec::Ssa),
        _ => None,
    }
}

struct ParsedRemainder {
    language: Option<String>,
    default: bool,
    forced: bool,
    title: Option<String>,
}

/// Tokenise the remainder after `<stem>.` into `(language, flags)`.
/// Any unknown flag aborts the parse and the caller drops the file.
fn parse_remainder(parts: &[&str]) -> Option<ParsedRemainder> {
    let (language, rest) = match parts.split_first() {
        Some((first, rest)) if language_re().is_match(first) => {
            (Some(first.to_ascii_lowercase()), rest)
        }
        // First token didn't look like a language; treat every token as a
        // flag and drop the file if any of them is unrecognised.
        Some(_) => return parse_flags_only(parts),
        None => (None, &[][..]),
    };

    let mut default = false;
    let mut forced = false;
    let mut hearing_impaired = false;
    for part in rest {
        match part.to_ascii_lowercase().as_str() {
            "default" => default = true,
            "forced" => forced = true,
            "sdh" | "cc" | "hi" => hearing_impaired = true,
            _ => return None,
        }
    }

    Some(ParsedRemainder {
        language,
        default,
        forced,
        title: if hearing_impaired {
            Some("SDH".to_string())
        } else {
            None
        },
    })
}

fn parse_flags_only(parts: &[&str]) -> Option<ParsedRemainder> {
    let mut default = false;
    let mut forced = false;
    let mut hearing_impaired = false;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "default" => default = true,
            "forced" => forced = true,
            "sdh" | "cc" | "hi" => hearing_impaired = true,
            _ => return None,
        }
    }
    Some(ParsedRemainder {
        language: None,
        default,
        forced,
        title: if hearing_impaired {
            Some("SDH".to_string())
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use noflex_transcoder::probe::SubtitleStreamProbe;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn embedded_tracks_number_subtitle_streams_from_zero() {
        let stream = |index, codec| SubtitleStreamProbe {
            index,
            codec,
            language: Some("eng".into()),
            default: false,
            forced: false,
            external: false,
            title: None,
        };
        let probe = ProbeReport {
            container: Some("matroska".into()),
            video: None,
            audio: Vec::new(),
            subtitle: vec![
                stream(2, SubtitleCodec::PgsImage),
                stream(3, SubtitleCodec::SubRip),
            ],
            format_bit_rate: None,
            duration_ms: None,
            chapters: Vec::new(),
        };
        let tracks = embedded_tracks(&probe);
        assert_eq!(
            tracks.iter().map(|t| t.stream_index).collect::<Vec<_>>(),
            [Some(0), Some(1)]
        );
        assert!(!tracks[0].srt_compatible && tracks[1].srt_compatible);
        assert!(tracks.iter().all(|t| !t.external && t.path.is_none()));
    }

    /// Write `name` into `dir` with empty contents so read_dir sees it.
    fn make(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"").expect("write sidecar");
        path
    }

    fn abs(path: &Path) -> String {
        path.canonicalize()
            .expect("canonicalize")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn bare_no_language_no_flags() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let sidecar = make(dir.path(), "Movie.srt");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.language, None);
        assert_eq!(t.codec, "sub_rip");
        assert!(!t.default);
        assert!(!t.forced);
        assert!(t.external);
        assert!(t.srt_compatible);
        assert_eq!(t.codec_profile, None);
        assert_eq!(t.image_count, None);
        assert_eq!(t.title, None);
        assert_eq!(t.path.as_deref(), Some(abs(&sidecar).as_str()));
    }

    #[test]
    fn english_language_only() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let sidecar = make(dir.path(), "Movie.en.srt");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.codec, "sub_rip");
        assert!(!t.forced);
        assert!(!t.default);
        assert_eq!(t.title, None);
        assert_eq!(t.path.as_deref(), Some(abs(&sidecar).as_str()));
    }

    #[test]
    fn english_forced_flag() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let sidecar = make(dir.path(), "Movie.en.forced.srt");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.codec, "sub_rip");
        assert!(t.forced);
        assert!(!t.default);
        assert_eq!(t.title, None);
        assert_eq!(t.path.as_deref(), Some(abs(&sidecar).as_str()));
    }

    #[test]
    fn english_sdh_vtt() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let sidecar = make(dir.path(), "Movie.en.sdh.vtt");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.codec, "web_vtt");
        assert!(t.srt_compatible);
        assert!(!t.forced);
        assert_eq!(t.title.as_deref(), Some("SDH"));
        assert_eq!(t.path.as_deref(), Some(abs(&sidecar).as_str()));
    }

    #[test]
    fn portuguese_brazilian_in_ass() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let sidecar = make(dir.path(), "Movie.pt-BR.ass");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.language.as_deref(), Some("pt-br"));
        assert_eq!(t.codec, "ass");
        assert!(!t.srt_compatible);
        assert!(!t.forced);
        assert!(!t.default);
        assert_eq!(t.title, None);
        assert_eq!(t.path.as_deref(), Some(abs(&sidecar).as_str()));
    }

    #[test]
    fn quality_token_skips_whole_file() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        make(dir.path(), "Movie.720p.srt");

        assert!(discover_sidecars(&video).is_empty());
    }

    #[test]
    fn unrelated_stem_skipped() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        make(dir.path(), "Other.srt");
        make(dir.path(), "Other.en.srt");

        assert!(discover_sidecars(&video).is_empty());
    }

    #[test]
    fn non_subtitle_extension_skipped() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        make(dir.path(), "Movie.nfo");

        assert!(discover_sidecars(&video).is_empty());
    }

    #[test]
    fn unknown_flag_after_language_skips_whole_file() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        make(dir.path(), "Movie.en.1080p.srt");

        // "1080p" isn't a recognised flag — the file is rejected wholesale
        // rather than turned into an English track with junk.
        assert!(discover_sidecars(&video).is_empty());
    }

    #[test]
    fn default_and_cc_flag_combination() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let sidecar = make(dir.path(), "Movie.en.default.cc.srt");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.language.as_deref(), Some("en"));
        assert!(t.default);
        assert!(!t.forced);
        assert_eq!(t.title.as_deref(), Some("SDH"));
        assert_eq!(t.path.as_deref(), Some(abs(&sidecar).as_str()));
    }

    #[test]
    fn ssa_extension_maps_to_ssa_codec() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let sidecar = make(dir.path(), "Movie.ssa");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.codec, "ssa");
        assert!(!t.srt_compatible);
        assert_eq!(t.path.as_deref(), Some(abs(&sidecar).as_str()));
    }

    #[test]
    fn sorted_by_path() {
        let dir = tempdir().unwrap();
        let video = make(dir.path(), "Movie.mkv");
        let earlier = make(dir.path(), "Movie.aa.srt");
        let later = make(dir.path(), "Movie.bb.srt");

        let tracks = discover_sidecars(&video);
        assert_eq!(tracks.len(), 2);
        assert_eq!(
            tracks[0].path.as_deref(),
            Some(abs(&earlier).as_str()),
            "lexicographic order: aa < bb"
        );
        assert_eq!(tracks[1].path.as_deref(), Some(abs(&later).as_str()));
    }
}
