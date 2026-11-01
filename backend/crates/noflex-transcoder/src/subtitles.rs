// SPDX-License-Identifier: AGPL-3.0-only
//
// Text subtitle conversion for out-of-band delivery. Browsers render
// only WebVTT through `<track>`, so SubRip and ASS/SSA sidecars are
// converted on the fly instead of being burned in. ASS styling and
// positioning are dropped; burn-in remains for clients that want them.

use noflex_core::SubtitleCodec;

use crate::ffmpeg_argv::TranscodeInput;

/// Whether [`to_webvtt`] can convert `codec`.
pub fn converts_to_webvtt(codec: SubtitleCodec) -> bool {
    matches!(
        codec,
        SubtitleCodec::WebVtt
            | SubtitleCodec::Srt
            | SubtitleCodec::SubRip
            | SubtitleCodec::Ass
            | SubtitleCodec::Ssa
    )
}

/// Whether a muxed-in track of `codec` can be extracted to WebVTT by
/// ffmpeg. Bitmap formats can only be burned in.
pub fn extracts_to_webvtt(codec: SubtitleCodec) -> bool {
    !matches!(
        codec,
        SubtitleCodec::PgsImage | SubtitleCodec::DvbSub | SubtitleCodec::Ttml
    )
}

/// argv (without the binary) that writes the input's `stream`-th
/// subtitle stream to stdout as WebVTT.
pub fn extract_webvtt_argv(input: &TranscodeInput, stream: u32) -> Vec<String> {
    [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        input.protocol_whitelist(),
        "-i",
        &input.ffmpeg_arg(),
        "-map",
        &format!("0:s:{stream}"),
        "-c:s",
        "webvtt",
        "-f",
        "webvtt",
        "pipe:1",
    ]
    .map(str::to_string)
    .to_vec()
}

/// Decode a sidecar's bytes: UTF-8 (BOM stripped) when valid, else
/// Latin-1, which is what most legacy non-UTF-8 SubRip files are close
/// enough to.
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// Convert subtitle text in `codec` to WebVTT. `None` for codecs
/// [`converts_to_webvtt`] rejects.
pub fn to_webvtt(codec: SubtitleCodec, text: &str) -> Option<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Some(match codec {
        SubtitleCodec::WebVtt => {
            if text.trim_start().starts_with("WEBVTT") {
                text
            } else {
                format!("WEBVTT\n\n{text}")
            }
        }
        SubtitleCodec::Srt | SubtitleCodec::SubRip => srt_to_vtt(&text),
        SubtitleCodec::Ass | SubtitleCodec::Ssa => ass_to_vtt(&text),
        _ => return None,
    })
}

fn srt_to_vtt(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str("WEBVTT\n\n");
    for line in text.lines() {
        if line.contains("-->") {
            // Only the timing line uses `,` for milliseconds.
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(&strip_braced(line));
        }
        out.push('\n');
    }
    out
}

/// Drop `{...}` blocks: ASS override tags, and the `{\an8}` some SubRip
/// files carry.
fn strip_braced(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut depth = 0_u32;
    for ch in line.chars() {
        match ch {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

fn ass_to_vtt(text: &str) -> String {
    let mut in_events = false;
    let mut fields: Vec<String> = Vec::new();
    let mut cues: Vec<(u64, u64, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(format) = line.strip_prefix("Format:") {
            fields = format
                .split(',')
                .map(|f| f.trim().to_ascii_lowercase())
                .collect();
            continue;
        }
        let Some(body) = line.strip_prefix("Dialogue:") else {
            continue;
        };
        if fields.is_empty() {
            continue;
        }
        // Text is the last field and may itself contain commas.
        let values: Vec<&str> = body.splitn(fields.len(), ',').collect();
        let get = |name: &str| {
            fields
                .iter()
                .position(|f| f == name)
                .and_then(|i| values.get(i))
                .map(|v| v.trim())
        };
        let (Some(start), Some(end), Some(raw)) = (
            get("start").and_then(parse_ass_time),
            get("end").and_then(parse_ass_time),
            get("text"),
        ) else {
            continue;
        };
        let cue = strip_braced(raw)
            .replace("\\N", "\n")
            .replace("\\n", "\n")
            .replace("\\h", " ")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let cue = cue.trim();
        if !cue.is_empty() && end > start {
            cues.push((start, end, cue.to_string()));
        }
    }
    cues.sort_by_key(|(start, end, _)| (*start, *end));
    let mut out = String::from("WEBVTT\n\n");
    for (start, end, cue) in cues {
        out.push_str(&format!(
            "{} --> {}\n{cue}\n\n",
            vtt_time(start),
            vtt_time(end)
        ));
    }
    out
}

/// `H:MM:SS.cc` → milliseconds.
fn parse_ass_time(s: &str) -> Option<u64> {
    let mut parts = s.split(':');
    let h: u64 = parts.next()?.trim().parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let seconds = parts.next()?;
    let (sec, frac) = seconds.split_once('.').unwrap_or((seconds, ""));
    let sec: u64 = sec.parse().ok()?;
    let frac_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac.get(..3)?.parse().ok()?,
    };
    Some(((h * 60 + m) * 60 + sec) * 1000 + frac_ms)
}

fn vtt_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn extract_argv_maps_the_subtitle_stream_with_file_only_input() {
        let input = crate::ffmpeg_argv::TranscodeInput::File("/m/a:b.mkv".into());
        let argv = super::extract_webvtt_argv(&input, 2);
        let at = |flag: &str| argv[argv.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(at("-protocol_whitelist"), "file");
        assert_eq!(at("-i"), "file:/m/a:b.mkv");
        assert_eq!(at("-map"), "0:s:2");
        assert_eq!(argv.last().map(String::as_str), Some("pipe:1"));
    }

    use super::*;

    #[test]
    fn srt_timings_and_header() {
        let srt = "\u{feff}1\r\n00:00:01,500 --> 00:00:03,000\r\n{\\an8}Hello, world\r\n\r\n";
        let vtt = to_webvtt(SubtitleCodec::SubRip, &decode_text(srt.as_bytes())).unwrap();
        assert_eq!(
            vtt,
            "WEBVTT\n\n1\n00:00:01.500 --> 00:00:03.000\nHello, world\n\n"
        );
    }

    #[test]
    fn vtt_passes_through_and_gets_header_if_missing() {
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.000\nhi\n";
        assert_eq!(to_webvtt(SubtitleCodec::WebVtt, vtt).unwrap(), vtt);
        assert!(
            to_webvtt(SubtitleCodec::WebVtt, "00:01.000 --> 00:02.000\nhi\n")
                .unwrap()
                .starts_with("WEBVTT\n\n")
        );
    }

    #[test]
    fn ass_dialogue_becomes_sorted_cues() {
        let ass = "[Script Info]\nTitle: x\n\n[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:05.20,0:00:06.00,Default,,0,0,0,,{\\i1}Second{\\i0}, line\\Nbreak <b>\n\
            Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,First\n\
            Comment: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,ignored\n";
        let vtt = to_webvtt(SubtitleCodec::Ass, ass).unwrap();
        assert_eq!(
            vtt,
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nFirst\n\n\
             00:00:05.200 --> 00:00:06.000\nSecond, line\nbreak &lt;b&gt;\n\n"
        );
    }

    #[test]
    fn latin1_fallback_and_image_codecs_rejected() {
        assert_eq!(decode_text(b"caf\xe9"), "café");
        assert!(to_webvtt(SubtitleCodec::PgsImage, "").is_none());
        assert!(!converts_to_webvtt(SubtitleCodec::DvbSub));
        assert!(converts_to_webvtt(SubtitleCodec::Ssa));
    }

    #[test]
    fn ass_time_parsing() {
        assert_eq!(parse_ass_time("1:02:03.45"), Some(3_723_450));
        assert_eq!(parse_ass_time("0:00:00.5"), Some(500));
        assert_eq!(parse_ass_time("bogus"), None);
        assert_eq!(vtt_time(3_723_450), "01:02:03.450");
    }
}
