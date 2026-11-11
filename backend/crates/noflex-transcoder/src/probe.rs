// SPDX-License-Identifier: AGPL-3.0-only

use std::str::FromStr;

use noflex_core::{
    AudioCodec, ChannelLayout, ColorPrimaries, DolbyVisionProfile, HdrTransfer, MatrixCoefficients,
    PixelFormat, SubtitleCodec, VideoCodec, VideoProfile,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Output of one ffprobe run. `video` is `None` for audio-only
/// or subtitle-only files; `audio` / `subtitle` collect every
/// track in source order with the original `index` for
/// caller-side track-list matching.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProbeReport {
    pub container: Option<String>,
    pub video: Option<VideoStreamProbe>,
    pub audio: Vec<AudioStreamProbe>,
    pub subtitle: Vec<SubtitleStreamProbe>,
    pub format_bit_rate: Option<u64>,
    /// Container duration (`format.duration`); absent for live inputs
    /// and some broken files.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// The container's chapters in start order (`-show_chapters`); empty
    /// when it has none.
    #[serde(default)]
    pub chapters: Vec<ChapterProbe>,
}

/// One chapter as the container lists it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterProbe {
    pub start_ms: i64,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoStreamProbe {
    pub codec: VideoCodec,
    pub profile: Option<VideoProfile>,
    pub level: Option<f32>,
    pub width: u32,
    pub height: u32,
    pub bit_depth: Option<u8>,
    pub pixel_format: Option<PixelFormat>,
    pub frame_rate: Option<f32>,
    pub color_space: Option<ColorPrimaries>,
    pub transfer: Option<HdrTransfer>,
    pub matrix: Option<MatrixCoefficients>,
    pub dovi_profile: Option<DolbyVisionProfile>,
    pub side_data: ProbeSideData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeSideData {
    pub max_cll: Option<u32>,
    pub max_fall: Option<u32>,
    pub mastering: Option<MasteringDisplay>,
}

/// SMPTE ST 2086 mastering-display metadata. CIE 1931 chroma
/// coordinates: 0.00002-unit resolution × 50000. Min luminance:
/// 0.0001 cd/m² × 10000. Max luminance: 1 cd/m².
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MasteringDisplay {
    pub primary_r: (u16, u16),
    pub primary_g: (u16, u16),
    pub primary_b: (u16, u16),
    pub white_point: (u16, u16),
    pub min_luminance: u32,
    pub max_luminance: u32,
}

/// `codec` is intentionally `String` — ffprobe returns many
/// variants (`aac`, `ac-3`, `eac3`, `truehd`, `flac`, `opus`,
/// `pcm_s*le`, `dts`, …), so the scanner records the raw tag; the typed enum
/// path is handled by audio decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStreamProbe {
    pub index: u32,
    pub codec: String,
    pub profile: Option<String>,
    pub bit_depth: Option<u8>,
    pub sample_rate: Option<u32>,
    pub channels: u8,
    pub channel_layout: Option<ChannelLayout>,
    pub language: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub title: Option<String>,
    pub audio_object_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleStreamProbe {
    pub index: u32,
    pub codec: SubtitleCodec,
    pub language: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub external: bool,
    pub title: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("ffprobe parse: {0}")]
    Parse(String),
    #[error("unsupported codec: {0}")]
    UnsupportedCodec(String),
    #[error("missing video stream")]
    MissingStream,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffprobe timed out after {secs}s")]
    Timeout { secs: u64 },
}

impl ProbeReport {
    /// Parse the output of
    /// `ffprobe -v error -show_streams -show_format -show_chapters -of json`.
    ///
    /// The single source of truth for ffprobe's free-form
    /// string codec names ("h264", "hevc", "av1", "vp9", "vp8",
    /// "mpeg2video", "prores", "theora", plus audio "aac",
    /// "ac-3", "eac3", "truehd", "flac", "opus", "mp3", "vorbis",
    /// "pcm_*" and subtitle "subrip", "hdmv_pgs_subtitle",
    /// "mov_text", "ass", "ssa", "dvb_subtitle", "webvtt") →
    /// typed enums in `noflex-core`.
    pub fn from_ffprobe_json(json: serde_json::Value) -> Result<Self, ProbeError> {
        let streams = json
            .get("streams")
            .and_then(|s| s.as_array())
            .ok_or_else(|| ProbeError::Parse("missing `streams` array".to_string()))?;

        let mut video: Option<VideoStreamProbe> = None;
        let mut audio: Vec<AudioStreamProbe> = Vec::new();
        let mut subtitle: Vec<SubtitleStreamProbe> = Vec::new();

        for raw in streams {
            let codec_type = raw.get("codec_type").and_then(|v| v.as_str()).unwrap_or("");
            match codec_type {
                // Only the first video stream is retained; multi-angle drops.
                "video" if video.is_none() => {
                    video = Some(parse_video_stream(raw)?);
                }
                "audio" => audio.push(parse_audio_stream(raw)?),
                "subtitle" => subtitle.push(parse_subtitle_stream(raw)?),
                _ => {}
            }
        }

        let format_obj = json.get("format");
        let container = format_obj
            .and_then(|f| f.get("format_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.split(',').next().unwrap_or(s).to_string());
        let format_bit_rate = format_obj
            .and_then(|f| f.get("bit_rate"))
            .and_then(parse_int_as_u64);

        let duration_ms = format_obj
            .and_then(|f| f.get("duration"))
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|secs| secs.is_finite() && *secs > 0.0)
            .map(|secs| (secs * 1000.0).round() as u64);

        let mut chapters: Vec<ChapterProbe> = json
            .get("chapters")
            .and_then(|c| c.as_array())
            .map(|list| list.iter().filter_map(parse_chapter).collect())
            .unwrap_or_default();
        chapters.sort_by_key(|c| c.start_ms);

        Ok(ProbeReport {
            container,
            video,
            audio,
            subtitle,
            format_bit_rate,
            duration_ms,
            chapters,
        })
    }
}

/// A chapter with a usable `start_time`; its `title` tag when it has one.
fn parse_chapter(raw: &serde_json::Value) -> Option<ChapterProbe> {
    let start_ms = raw
        .get("start_time")
        .and_then(|v| v.as_str())
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|secs| secs.is_finite() && *secs >= 0.0)
        .map(|secs| (secs * 1000.0).round() as i64)?;
    let title = raw
        .get("tags")
        .and_then(|t| t.get("title"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    Some(ChapterProbe { start_ms, title })
}

fn parse_video_stream(raw: &serde_json::Value) -> Result<VideoStreamProbe, ProbeError> {
    let codec_name = raw
        .get("codec_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ProbeError::Parse("video stream missing `codec_name`".to_string()))?;
    let codec = map_video_codec(codec_name)
        .ok_or_else(|| ProbeError::UnsupportedCodec(codec_name.to_string()))?;

    let profile = raw
        .get("profile")
        .and_then(|v| v.as_str())
        .and_then(|p| map_video_profile_for_codec(codec, p));

    let level = raw.get("level").and_then(parse_level);

    let width = raw
        .get("width")
        .and_then(parse_int_as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| ProbeError::Parse("video stream missing/invalid `width`".to_string()))?;
    let height = raw
        .get("height")
        .and_then(parse_int_as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| ProbeError::Parse("video stream missing/invalid `height`".to_string()))?;

    let pix_fmt = raw.get("pix_fmt").and_then(|v| v.as_str());
    let pixel_format = pix_fmt.and_then(|p| p.parse::<PixelFormat>().ok());
    let bit_depth = pix_fmt.and_then(bit_depth_from_pix_fmt);

    let frame_rate = raw
        .get("r_frame_rate")
        .and_then(|v| v.as_str())
        .and_then(parse_frame_rate);

    let color_space = raw
        .get("color_primaries")
        .and_then(|v| v.as_str())
        .and_then(map_color_primaries);
    let transfer = raw
        .get("color_transfer")
        .and_then(|v| v.as_str())
        .and_then(map_hdr_transfer);
    let matrix = raw
        .get("color_space")
        .and_then(|v| v.as_str())
        .and_then(map_matrix_coefficients);

    let mut side_data = ProbeSideData::default();
    let mut dovi_profile: Option<DolbyVisionProfile> = None;
    if let Some(list) = raw.get("side_data_list").and_then(|v| v.as_array()) {
        for entry in list {
            let kind = entry
                .get("side_data_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match kind {
                "Mastering display metadata" => {
                    if let Some(m) = parse_mastering_display(entry) {
                        side_data.mastering = Some(m);
                    }
                }
                "Content light level metadata" => {
                    if let Some(max_cll) = entry
                        .get("max_content_light_level")
                        .and_then(parse_int_as_u64)
                        .and_then(|v| u32::try_from(v).ok())
                    {
                        side_data.max_cll = Some(max_cll);
                    }
                    if let Some(max_fall) = entry
                        .get("max_frame_average_light_level")
                        .and_then(parse_int_as_u64)
                        .and_then(|v| u32::try_from(v).ok())
                    {
                        side_data.max_fall = Some(max_fall);
                    }
                }
                "Dolby Vision configuration" | "Dolby Vision RPU" => {
                    if let Some(p) = entry
                        .get("dv_profile")
                        .and_then(parse_int_as_u64)
                        .and_then(|v| u8::try_from(v).ok())
                        .and_then(|n| DolbyVisionProfile::try_from(n).ok())
                    {
                        dovi_profile = Some(p);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(VideoStreamProbe {
        codec,
        profile,
        level,
        width,
        height,
        bit_depth,
        pixel_format,
        frame_rate,
        color_space,
        transfer,
        matrix,
        dovi_profile,
        side_data,
    })
}

fn parse_audio_stream(raw: &serde_json::Value) -> Result<AudioStreamProbe, ProbeError> {
    let index = raw
        .get("index")
        .and_then(parse_int_as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    let codec = raw
        .get("codec_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ProbeError::Parse("audio stream missing `codec_name`".to_string()))?
        .to_string();
    let profile = raw
        .get("profile")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let bit_depth = raw
        .get("bits_per_sample")
        .and_then(parse_int_as_u64)
        .and_then(|v| u8::try_from(v).ok());
    let sample_rate = raw
        .get("sample_rate")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u32>().ok());
    let channels = raw
        .get("channels")
        .and_then(parse_int_as_u64)
        .and_then(|v| u8::try_from(v).ok())
        .unwrap_or(2);
    let channel_layout = raw
        .get("channel_layout")
        .and_then(|v| v.as_str())
        .and_then(map_channel_layout);
    let language = raw
        .get("tags")
        .and_then(|t| t.get("language"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let default = raw
        .get("disposition")
        .and_then(|d| d.get("default"))
        .and_then(|v| v.as_i64())
        .map(|n| n != 0)
        .unwrap_or(false);
    let forced = raw
        .get("disposition")
        .and_then(|d| d.get("forced"))
        .and_then(|v| v.as_i64())
        .map(|n| n != 0)
        .unwrap_or(false);
    let title = raw
        .get("tags")
        .and_then(|t| t.get("title"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let audio_object_type = raw
        .get("tags")
        .and_then(|t| t.get("audio_object_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(AudioStreamProbe {
        index,
        codec,
        profile,
        bit_depth,
        sample_rate,
        channels,
        channel_layout,
        language,
        default,
        forced,
        title,
        audio_object_type,
    })
}

fn parse_subtitle_stream(raw: &serde_json::Value) -> Result<SubtitleStreamProbe, ProbeError> {
    let index = raw
        .get("index")
        .and_then(parse_int_as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    let codec_name = raw
        .get("codec_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ProbeError::Parse("subtitle stream missing `codec_name`".to_string()))?;
    let codec = map_subtitle_codec(codec_name)
        .ok_or_else(|| ProbeError::UnsupportedCodec(codec_name.to_string()))?;
    let language = raw
        .get("tags")
        .and_then(|t| t.get("language"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let default = raw
        .get("disposition")
        .and_then(|d| d.get("default"))
        .and_then(|v| v.as_i64())
        .map(|n| n != 0)
        .unwrap_or(false);
    let forced = raw
        .get("disposition")
        .and_then(|d| d.get("forced"))
        .and_then(|v| v.as_i64())
        .map(|n| n != 0)
        .unwrap_or(false);
    // ffprobe doesn't distinguish muxed-vs-sidecar; the
    // scanner's filename walker resolves that separately.
    let external = false;
    let title = raw
        .get("tags")
        .and_then(|t| t.get("title"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(SubtitleStreamProbe {
        index,
        codec,
        language,
        default,
        forced,
        external,
        title,
    })
}

fn parse_mastering_display(entry: &serde_json::Value) -> Option<MasteringDisplay> {
    let raw = entry
        .get("mastering_display_primaries")
        .and_then(|v| v.as_str())?;
    // SMPTE ST 2086 string: "R(x1,y1)G(x2,y2)B(x3,y3)WP(x4,y4)"
    // e.g. "R(0.6800,0.3200)G(0.2650,0.6900)B(0.1500,0.0600)WP(0.3127,0.3290)"
    let r = extract_chroma_pair(raw, "R(")?;
    let g = extract_chroma_pair(raw, "G(")?;
    let b = extract_chroma_pair(raw, "B(")?;
    let wp = extract_chroma_pair(raw, "WP(")?;
    let min_lum = entry
        .get("mastering_display_minimum_luminance")
        .and_then(parse_f64)?;
    let max_lum = entry
        .get("mastering_display_maximum_luminance")
        .and_then(parse_f64)?;
    Some(MasteringDisplay {
        primary_r: scale_chroma(r),
        primary_g: scale_chroma(g),
        primary_b: scale_chroma(b),
        white_point: scale_chroma(wp),
        min_luminance: scale_min_luminance(min_lum),
        max_luminance: scale_max_luminance(max_lum),
    })
}

fn extract_chroma_pair(s: &str, marker: &str) -> Option<(f64, f64)> {
    let after = s.find(marker)? + marker.len();
    let rest = &s[after..];
    let end = rest.find(')')?;
    let pair = &rest[..end];
    let (xs, ys) = pair.split_once(',')?;
    Some((xs.trim().parse().ok()?, ys.trim().parse().ok()?))
}

fn scale_chroma(p: (f64, f64)) -> (u16, u16) {
    let x = (p.0 * 50_000.0).round().clamp(0.0, 50_000.0) as u16;
    let y = (p.1 * 50_000.0).round().clamp(0.0, 50_000.0) as u16;
    (x, y)
}

fn scale_min_luminance(v: f64) -> u32 {
    // 0.0001 cd/m² resolution; SMPTE ST 2086 floor.
    (v * 10_000.0).round().max(0.0) as u32
}

fn scale_max_luminance(v: f64) -> u32 {
    v.round().max(0.0) as u32
}

fn parse_level(v: &serde_json::Value) -> Option<f32> {
    if let Some(n) = v.as_f64() {
        return Some(n as f32);
    }
    // H.264: "4.0", "4.1"; HEVC: "120", "150" (× 30).
    if let Some(s) = v.as_str() {
        return s.parse::<f32>().ok();
    }
    None
}

fn parse_frame_rate(s: &str) -> Option<f32> {
    // ffprobe emits "24000/1001", "25/1", "60/1", etc.
    let (num, den) = s.split_once('/')?;
    let n: f32 = num.parse().ok()?;
    let d: f32 = den.parse().ok()?;
    if d == 0.0 {
        return None;
    }
    Some(n / d)
}

fn parse_int_as_u64(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(n) = v.as_i64() {
        return u64::try_from(n).ok();
    }
    if let Some(s) = v.as_str() {
        return s.parse::<u64>().ok();
    }
    None
}

fn parse_f64(v: &serde_json::Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        return s.parse::<f64>().ok();
    }
    None
}

fn bit_depth_from_pix_fmt(pix_fmt: &str) -> Option<u8> {
    // yuv420p (8) / yuv420p10le (10) / yuv422p12le (12) / gray10le (10) …
    // Bit depth is the digit run that ends the string, immediately
    // followed by `le` or `be` (the FFmpeg endianness suffix). Bare
    // 8-bit forms have no such suffix.
    let lower = pix_fmt.to_ascii_lowercase();
    for suffix in ["le", "be"] {
        if let Some(stripped) = lower.strip_suffix(suffix)
            && let Some(digits) = stripped.rsplit(|c: char| !c.is_ascii_digit()).next()
            && !digits.is_empty()
        {
            return digits.parse::<u8>().ok();
        }
    }
    Some(8)
}

pub fn map_video_codec(name: &str) -> Option<VideoCodec> {
    Some(match name {
        "h264" => VideoCodec::H264,
        "hevc" | "h265" => VideoCodec::H265,
        "av1" => VideoCodec::Av1,
        "vp9" => VideoCodec::Vp9,
        "vp8" => VideoCodec::Vp8,
        "mpeg2video" => VideoCodec::Mpeg2,
        "prores" => VideoCodec::Prores,
        "theora" => VideoCodec::Theora,
        _ => return None,
    })
}

fn map_video_profile(name: &str) -> Option<VideoProfile> {
    Some(match name {
        "Baseline" => VideoProfile::H264Baseline,
        "Constrained Baseline" => VideoProfile::H264ConstrainedBaseline,
        "High 10" => VideoProfile::H264High10,
        "Main 10" | "Main 10+" => VideoProfile::HevcMain10,
        "Main Still Picture" => VideoProfile::HevcMain,
        "Format Range Extensions" => VideoProfile::HevcHigh,
        "High Throughput" => VideoProfile::HevcHigh10,
        "Professional" => VideoProfile::Av1Professional,
        "Profile 0" => VideoProfile::Vp9Profile0,
        "Profile 1" => VideoProfile::Vp9Profile1,
        "Profile 2" => VideoProfile::Vp9Profile2,
        "Profile 3" => VideoProfile::Vp9Profile3,
        _ => return None,
    })
}

/// Disambiguated profile mapping. ffprobe emits the `profile`
/// string without saying which spec it belongs to, so a bare
/// `"Main"` is routed to H.264, HEVC, AV1, or VP9 by the codec
/// name.
pub fn map_video_profile_for_codec(codec: VideoCodec, name: &str) -> Option<VideoProfile> {
    match name {
        "Main" => Some(match codec {
            VideoCodec::H264 => VideoProfile::H264Main,
            VideoCodec::H265 => VideoProfile::HevcMain,
            VideoCodec::Av1 => VideoProfile::Av1Main,
            VideoCodec::Vp9 => VideoProfile::Vp9Profile0,
            _ => return None,
        }),
        "High" => Some(match codec {
            VideoCodec::H264 => VideoProfile::H264High,
            VideoCodec::Av1 => VideoProfile::Av1High,
            _ => return map_video_profile(name),
        }),
        other => map_video_profile(other),
    }
}

/// ffprobe's open-ended audio codec names onto the closed enum the
/// capability model speaks. `None` for codecs no client declares
/// (DTS-HD MA still maps to `Dts`; ALAC, WMA, … don't map).
pub fn map_audio_codec(name: &str) -> Option<AudioCodec> {
    Some(match name {
        "aac" => AudioCodec::Aac,
        "ac3" | "ac-3" => AudioCodec::Ac3,
        "eac3" | "ec-3" => AudioCodec::Eac3,
        "mp3" | "mp3float" => AudioCodec::Mp3,
        "opus" => AudioCodec::Opus,
        "vorbis" => AudioCodec::Vorbis,
        "flac" => AudioCodec::Flac,
        "truehd" | "mlp" => AudioCodec::Truehd,
        "dts" => AudioCodec::Dts,
        pcm if pcm.starts_with("pcm_") => AudioCodec::Pcm,
        _ => return None,
    })
}

fn map_color_primaries(name: &str) -> Option<ColorPrimaries> {
    ColorPrimaries::from_str(name).ok()
}

fn map_hdr_transfer(name: &str) -> Option<HdrTransfer> {
    // ffprobe emits the spec-defined transfer names; the
    // enum's serde form is the short label.
    let normalised = match name {
        "smpte2084" => "pq",
        "arib-std-b67" => "hlg",
        "bt709" | "smpte170m" | "smpte240m" | "bt601" | "iec61966-2-1" => "sdr",
        other => other,
    };
    HdrTransfer::from_str(normalised).ok()
}

fn map_channel_layout(name: &str) -> Option<ChannelLayout> {
    // ffprobe emits the consumer-facing surround form
    // ("stereo", "5.1", "7.1"); the enum's serde form is the
    // short channel count label.
    let normalised = match name {
        "stereo" | "2.0" => "stereo",
        "2.1" => "3.0",
        "4.1" => "5.0",
        "5.1" | "5.1(side)" => "6.0",
        "6.1" | "7.0" | "7.1(wide)" | "7.1(wide-side)" => "7.1",
        "octagonal" => "8.0",
        "joc" | "atmos" | "ambisonic" => "object_based",
        other => other,
    };
    ChannelLayout::from_str(normalised).ok()
}

fn map_matrix_coefficients(name: &str) -> Option<MatrixCoefficients> {
    // ffprobe emits `bt2020nc` (NCL) / `bt2020c` (CL) /
    // `chroma-derived-nc` / `chroma-derived-c`; the enum's
    // serde form is `bt2020_ncl` / `bt2020_cl` /
    // `chroma_derived_nc` / `chroma_derived_cl`.
    let normalised = match name {
        "bt2020nc" => "bt2020_ncl",
        "bt2020c" => "bt2020_cl",
        "chroma-derived-nc" => "chroma_derived_nc",
        "chroma-derived-c" => "chroma_derived_cl",
        other => other,
    };
    MatrixCoefficients::from_str(normalised).ok()
}

fn map_subtitle_codec(name: &str) -> Option<SubtitleCodec> {
    Some(match name {
        "subrip" | "srt" => SubtitleCodec::SubRip,
        "hdmv_pgs_subtitle" | "pgs" => SubtitleCodec::PgsImage,
        "mov_text" => SubtitleCodec::MovText,
        "ass" => SubtitleCodec::Ass,
        "ssa" => SubtitleCodec::Ssa,
        "dvb_subtitle" | "dvbsub" => SubtitleCodec::DvbSub,
        "webvtt" => SubtitleCodec::WebVtt,
        "ttml" => SubtitleCodec::Ttml,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: serde_json::Value) -> Result<ProbeReport, ProbeError> {
        ProbeReport::from_ffprobe_json(json)
    }

    #[test]
    fn missing_streams_array_is_parse_error() {
        let v = serde_json::json!({});
        let err = parse(v).expect_err("parse should fail");
        assert!(matches!(err, ProbeError::Parse(_)));
    }

    #[test]
    fn audio_only_file_has_no_video_stream() {
        let v = serde_json::json!({
            "streams": [
                {"codec_type": "audio", "codec_name": "aac", "channels": 2, "index": 1}
            ]
        });
        let r = parse(v).expect("parse");
        assert!(r.video.is_none());
        assert_eq!(r.audio.len(), 1);
        assert_eq!(r.audio[0].codec, "aac");
    }

    #[test]
    fn unknown_video_codec_is_unsupported() {
        let v = serde_json::json!({
            "streams": [
                {"codec_type": "video", "codec_name": "xxyzz", "width": 1280, "height": 720, "index": 0}
            ]
        });
        let err = parse(v).expect_err("parse should fail");
        assert!(matches!(err, ProbeError::UnsupportedCodec(s) if s == "xxyzz"));
    }

    #[test]
    fn h264_high_with_aac_stereo() {
        let v = serde_json::json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "profile": "High",
                    "level": 40,
                    "width": 1920,
                    "height": 1080,
                    "pix_fmt": "yuv420p",
                    "r_frame_rate": "24000/1001",
                    "color_primaries": "bt709",
                    "color_transfer": "bt709",
                    "color_space": "bt709",
                    "index": 0
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac",
                    "profile": "LC",
                    "channels": 2,
                    "channel_layout": "stereo",
                    "sample_rate": "48000",
                    "bits_per_sample": 16,
                    "index": 1,
                    "disposition": {"default": 1, "forced": 0},
                    "tags": {"language": "eng"}
                }
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "bit_rate": "5000000", "duration": "5400.250000"}
        });
        let r = parse(v).expect("parse");
        let v0 = r.video.as_ref().expect("video");
        assert_eq!(v0.codec, VideoCodec::H264);
        assert_eq!(v0.profile, Some(VideoProfile::H264High));
        assert_eq!(v0.level, Some(40.0));
        assert_eq!(v0.width, 1920);
        assert_eq!(v0.height, 1080);
        assert_eq!(v0.pixel_format, Some(PixelFormat::Yuv420p));
        assert_eq!(v0.bit_depth, Some(8));
        assert_eq!(v0.frame_rate, Some(23.976025));
        assert_eq!(v0.color_space, Some(ColorPrimaries::Bt709));
        assert_eq!(v0.transfer, Some(HdrTransfer::Sdr));
        assert_eq!(r.container.as_deref(), Some("mov"));
        assert_eq!(r.format_bit_rate, Some(5_000_000));
        assert_eq!(r.duration_ms, Some(5_400_250));
        assert_eq!(r.audio.len(), 1);
        assert_eq!(r.audio[0].codec, "aac");
        assert_eq!(r.audio[0].channels, 2);
        assert_eq!(r.audio[0].channel_layout, Some(ChannelLayout::Stereo));
        assert!(r.audio[0].default);
        assert!(!r.audio[0].forced);
        assert_eq!(r.audio[0].language.as_deref(), Some("eng"));
    }

    #[test]
    fn chapters_come_in_start_order_with_their_titles() {
        let v = serde_json::json!({
            "streams": [],
            "chapters": [
                {"id": 2, "start_time": "1290.500000", "tags": {"title": "End Credits"}},
                {"id": 0, "start_time": "0.000000", "tags": {"title": " Opening "}},
                {"id": 1, "start_time": "85.042000", "tags": {"title": ""}},
                {"id": 3, "start_time": "N/A"}
            ]
        });
        let r = parse(v).expect("parse");
        assert_eq!(
            r.chapters,
            [
                ChapterProbe {
                    start_ms: 0,
                    title: Some("Opening".into())
                },
                ChapterProbe {
                    start_ms: 85_042,
                    title: None
                },
                ChapterProbe {
                    start_ms: 1_290_500,
                    title: Some("End Credits".into())
                },
            ]
        );
    }

    #[test]
    fn av1_main_with_opus_stereo_in_webm() {
        let v = serde_json::json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "av1",
                    "profile": "Main",
                    "width": 1280,
                    "height": 720,
                    "pix_fmt": "yuv420p",
                    "r_frame_rate": "25/1",
                    "index": 0
                },
                {
                    "codec_type": "audio",
                    "codec_name": "opus",
                    "channels": 2,
                    "channel_layout": "stereo",
                    "index": 1
                }
            ],
            "format": {"format_name": "matroska,webm"}
        });
        let r = parse(v).expect("parse");
        let v0 = r.video.as_ref().expect("video");
        assert_eq!(v0.codec, VideoCodec::Av1);
        assert_eq!(v0.profile, Some(VideoProfile::Av1Main));
        assert_eq!(r.container.as_deref(), Some("matroska"));
        assert_eq!(r.audio[0].codec, "opus");
    }

    #[test]
    fn hevc_ac3_mkv_sdr() {
        let v = serde_json::json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "hevc",
                    "profile": "Main",
                    "width": 1920,
                    "height": 1080,
                    "pix_fmt": "yuv420p",
                    "index": 0
                },
                {
                    "codec_type": "audio",
                    "codec_name": "ac3",
                    "channels": 6,
                    "channel_layout": "5.1",
                    "index": 1
                }
            ],
            "format": {"format_name": "matroska,webm"}
        });
        let r = parse(v).expect("parse");
        let v0 = r.video.as_ref().expect("video");
        assert_eq!(v0.codec, VideoCodec::H265);
        assert_eq!(v0.profile, Some(VideoProfile::HevcMain));
        assert_eq!(r.audio[0].codec, "ac3");
        assert_eq!(r.audio[0].channels, 6);
        assert_eq!(r.audio[0].channel_layout, Some(ChannelLayout::Six));
    }

    #[test]
    fn hevc_main10_hdr10_parses_mastering_and_cll() {
        let v = serde_json::json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "hevc",
                    "profile": "Main 10",
                    "width": 3840,
                    "height": 2160,
                    "pix_fmt": "yuv420p10le",
                    "color_primaries": "bt2020",
                    "color_transfer": "smpte2084",
                    "color_space": "bt2020nc",
                    "r_frame_rate": "24/1",
                    "index": 0,
                    "side_data_list": [
                        {
                            "side_data_type": "Mastering display metadata",
                            "mastering_display_primaries":
                                "R(0.6800,0.3200)G(0.2650,0.6900)B(0.1500,0.0600)WP(0.3127,0.3290)",
                            "mastering_display_minimum_luminance": "0.0050",
                            "mastering_display_maximum_luminance": "1000"
                        },
                        {
                            "side_data_type": "Content light level metadata",
                            "max_content_light_level": 1000,
                            "max_frame_average_light_level": 400
                        }
                    ]
                }
            ],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2"}
        });
        let r = parse(v).expect("parse");
        let v0 = r.video.as_ref().expect("video");
        assert_eq!(v0.codec, VideoCodec::H265);
        assert_eq!(v0.profile, Some(VideoProfile::HevcMain10));
        assert_eq!(v0.pixel_format, Some(PixelFormat::Yuv420p10le));
        assert_eq!(v0.bit_depth, Some(10));
        assert_eq!(v0.color_space, Some(ColorPrimaries::Bt2020));
        assert_eq!(v0.transfer, Some(HdrTransfer::Pq));
        assert_eq!(v0.matrix, Some(MatrixCoefficients::Bt2020Ncl));
        assert_eq!(v0.frame_rate, Some(24.0));
        assert_eq!(v0.side_data.max_cll, Some(1000));
        assert_eq!(v0.side_data.max_fall, Some(400));
        let m = v0.side_data.mastering.as_ref().expect("mastering");
        assert_eq!(m.primary_r, (34000, 16000));
        assert_eq!(m.primary_g, (13250, 34500));
        assert_eq!(m.primary_b, (7500, 3000));
        assert_eq!(m.white_point, (15635, 16450));
        assert_eq!(m.min_luminance, 50);
        assert_eq!(m.max_luminance, 1000);
    }

    #[test]
    fn hevc_dolby_vision_profile_8() {
        let v = serde_json::json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "hevc",
                    "profile": "Main 10",
                    "width": 1920,
                    "height": 1080,
                    "pix_fmt": "yuv420p10le",
                    "color_transfer": "smpte2084",
                    "index": 0,
                    "side_data_list": [
                        {
                            "side_data_type": "Dolby Vision configuration",
                            "dv_version_major": 1,
                            "dv_version_minor": 0,
                            "dv_profile": 8,
                            "dv_level": 9
                        }
                    ]
                }
            ],
            "format": {"format_name": "mp4"}
        });
        let r = parse(v).expect("parse");
        let v0 = r.video.as_ref().expect("video");
        assert_eq!(v0.dovi_profile, Some(DolbyVisionProfile::Profile8));
        assert_eq!(v0.transfer, Some(HdrTransfer::Pq));
    }

    #[test]
    fn subtitle_pgs_track_decodes() {
        let v = serde_json::json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1280,
                    "height": 720,
                    "index": 0
                },
                {
                    "codec_type": "subtitle",
                    "codec_name": "hdmv_pgs_subtitle",
                    "index": 2,
                    "disposition": {"default": 0, "forced": 0},
                    "tags": {"language": "eng"}
                }
            ]
        });
        let r = parse(v).expect("parse");
        assert_eq!(r.subtitle.len(), 1);
        assert_eq!(r.subtitle[0].codec, SubtitleCodec::PgsImage);
        assert_eq!(r.subtitle[0].index, 2);
        assert!(!r.subtitle[0].default);
        assert!(!r.subtitle[0].forced);
    }

    #[test]
    fn unknown_subtitle_codec_is_unsupported() {
        let v = serde_json::json!({
            "streams": [
                {
                    "codec_type": "subtitle",
                    "codec_name": "not-a-codec",
                    "index": 2
                }
            ]
        });
        let err = parse(v).expect_err("parse should fail");
        assert!(matches!(err, ProbeError::UnsupportedCodec(s) if s == "not-a-codec"));
    }

    #[test]
    fn parse_frame_rate_handles_fractional_and_zero() {
        // 24000/1001 in f32 is the closest representable; the
        // exact f64 result is 23.976023976…; f32 rounds to 23.976025.
        assert_eq!(parse_frame_rate("24000/1001").unwrap(), 23.976025);
        assert_eq!(parse_frame_rate("25/1").unwrap(), 25.0);
        assert!(parse_frame_rate("0/0").is_none());
        assert!(parse_frame_rate("nope").is_none());
    }

    #[test]
    fn bit_depth_from_pix_fmt_extraction() {
        assert_eq!(bit_depth_from_pix_fmt("yuv420p"), Some(8));
        assert_eq!(bit_depth_from_pix_fmt("yuv420p10le"), Some(10));
        assert_eq!(bit_depth_from_pix_fmt("yuv420p12le"), Some(12));
        assert_eq!(bit_depth_from_pix_fmt("yuv422p10le"), Some(10));
        assert_eq!(bit_depth_from_pix_fmt("yuv444p"), Some(8));
    }

    #[test]
    fn chroma_extraction_handles_canonical_smpte_string() {
        let s = "R(0.6800,0.3200)G(0.2650,0.6900)B(0.1500,0.0600)WP(0.3127,0.3290)";
        assert_eq!(extract_chroma_pair(s, "R(").unwrap(), (0.6800, 0.3200));
        assert_eq!(extract_chroma_pair(s, "WP(").unwrap(), (0.3127, 0.3290));
        assert!(extract_chroma_pair(s, "Z(").is_none());
    }

    #[test]
    fn map_video_profile_for_codec_disambiguates_main() {
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::H264, "Main"),
            Some(VideoProfile::H264Main)
        );
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::H265, "Main"),
            Some(VideoProfile::HevcMain)
        );
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::Av1, "Main"),
            Some(VideoProfile::Av1Main)
        );
    }

    #[test]
    fn map_video_profile_for_codec_disambiguates_high() {
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::H264, "High"),
            Some(VideoProfile::H264High)
        );
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::Av1, "High"),
            Some(VideoProfile::Av1High)
        );
    }

    #[test]
    fn map_video_profile_for_codec_names_vp9_profiles() {
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::Vp9, "Profile 2"),
            Some(VideoProfile::Vp9Profile2)
        );
    }

    #[test]
    fn map_audio_codec_covers_ffprobe_names() {
        assert_eq!(map_audio_codec("eac3"), Some(AudioCodec::Eac3));
        assert_eq!(map_audio_codec("pcm_s24le"), Some(AudioCodec::Pcm));
        assert_eq!(map_audio_codec("alac"), None);
    }

    #[test]
    fn map_video_profile_for_codec_keeps_unique_strings() {
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::H265, "Main 10"),
            Some(VideoProfile::HevcMain10)
        );
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::H264, "High 10"),
            Some(VideoProfile::H264High10)
        );
        assert_eq!(
            map_video_profile_for_codec(VideoCodec::H265, "Format Range Extensions"),
            Some(VideoProfile::HevcHigh)
        );
    }
}
