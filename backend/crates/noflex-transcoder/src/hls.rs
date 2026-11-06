// SPDX-License-Identifier: AGPL-3.0-only
//
// Master-playlist rendering. ffmpeg writes the media playlists and
// segments; the master is ours because hls.js creates its MSE
// SourceBuffers from the CODECS attribute, so it must name exactly what
// the segments carry — a stream-copied HEVC rendition announced as
// `avc1` fails to play.

use std::fmt::Write;

use noflex_core::{AudioCodec, DolbyVisionProfile, VideoCodec, VideoProfile};

use crate::{
    decision::{AudioPlan, TranscodePlan, VideoPlan, VideoRange},
    probe::ProbeReport,
};

/// HLS `VIDEO-RANGE` attribute values (RFC 8216bis §4.4.6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlsVideoRange {
    Sdr,
    Pq,
    Hlg,
}

/// One `EXT-X-STREAM-INF` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlsVariant {
    pub bandwidth_bps: u64,
    pub resolution: Option<(u32, u32)>,
    /// RFC 6381 codec list, comma-separated.
    pub codecs: String,
    /// Dolby Vision signalling layered over a backward-compatible
    /// base codec (`dvh1.08.06/db1p`).
    pub supplemental_codecs: Option<String>,
    pub video_range: Option<HlsVideoRange>,
    pub path: String,
}

/// Output video geometry and rate for a re-encode, as the server's
/// ladder resolved it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodedVideo {
    pub width: u32,
    pub height: u32,
    pub bitrate_bps: u64,
}

/// Assumed bitrate when a copied source has none probed; only feeds
/// hls.js' ABR estimate, which has nothing to switch between.
const UNKNOWN_SOURCE_BPS: u64 = 20_000_000;

impl HlsVariant {
    /// The single variant a session's master playlist announces for
    /// `plan`. `encoded` is required when the plan re-encodes video;
    /// `audio_stream` indexes `probe.audio`.
    pub fn for_plan(
        probe: &ProbeReport,
        plan: &TranscodePlan,
        audio_stream: Option<usize>,
        encoded: Option<EncodedVideo>,
        path: impl Into<String>,
    ) -> Self {
        let mut codecs = Vec::new();
        let mut supplemental = None;
        let mut video_range = None;
        let mut resolution = None;
        let mut bandwidth = 0_u64;
        match (plan.video, probe.video.as_ref()) {
            (VideoPlan::Copy { codec, range }, Some(v)) => {
                // Copying a Dolby Vision stream to a client without it
                // presents the base layer, so only the base codec is
                // announced.
                if let Some(s) = video_codec_string(codec, v.profile, v.level, v.bit_depth) {
                    codecs.push(s);
                }
                if matches!(
                    range,
                    VideoRange::Dovi
                        | VideoRange::DoviWithHdr10
                        | VideoRange::DoviWithHlg
                        | VideoRange::DoviWithSdr
                ) && let Some(profile) = v.dovi_profile
                {
                    let dv = dovi_codec_string(codec, profile, v.level);
                    if range == VideoRange::Dovi {
                        // Profile 5 has no compatible base layer to fall back on.
                        codecs.clear();
                        codecs.push(dv);
                    } else {
                        supplemental = Some(dv);
                    }
                }
                video_range = Some(match range {
                    VideoRange::Sdr | VideoRange::DoviWithSdr => HlsVideoRange::Sdr,
                    VideoRange::Hlg | VideoRange::DoviWithHlg => HlsVideoRange::Hlg,
                    _ => HlsVideoRange::Pq,
                });
                resolution = Some((v.width, v.height));
                bandwidth = probe.format_bit_rate.unwrap_or(UNKNOWN_SOURCE_BPS);
            }
            (VideoPlan::Encode { codec, profile, .. }, _) => {
                let out = encoded.unwrap_or(EncodedVideo {
                    width: 1920,
                    height: 1080,
                    bitrate_bps: 8_000_000,
                });
                if let Some(s) = encoded_video_codec_string(codec, profile, out.height) {
                    codecs.push(s);
                }
                video_range = Some(HlsVideoRange::Sdr);
                resolution = Some((out.width, out.height));
                bandwidth = out.bitrate_bps;
            }
            _ => {}
        }
        let source_audio = audio_stream.and_then(|i| probe.audio.get(i));
        match plan.audio {
            AudioPlan::Copy { codec } => {
                let profile = source_audio.and_then(|a| a.profile.as_deref());
                codecs.extend(audio_codec_string(codec, profile).map(str::to_string));
                if plan.video == VideoPlan::None {
                    bandwidth = probe.format_bit_rate.unwrap_or(320_000);
                }
            }
            AudioPlan::Encode { codec, channels } => {
                codecs.extend(audio_codec_string(codec, None).map(str::to_string));
                bandwidth = bandwidth.saturating_add(audio_bitrate_bps(codec, channels));
            }
            AudioPlan::None => {}
        }
        Self {
            bandwidth_bps: bandwidth.max(1),
            resolution,
            codecs: codecs.join(","),
            supplemental_codecs: supplemental,
            video_range,
            path: path.into(),
        }
    }
}

/// Bitrate the encoder targets for `channels` of `codec`; shared with
/// the argv builder so the playlist's BANDWIDTH matches the output.
pub fn audio_bitrate_bps(codec: AudioCodec, channels: u8) -> u64 {
    let per_channel = match codec {
        AudioCodec::Opus => 48_000,
        AudioCodec::Flac | AudioCodec::Pcm => 700_000,
        _ => 64_000,
    };
    (per_channel * u64::from(channels.max(1))).min(640_000)
}

/// RFC 6381 string for a stream as probed. `level` is in ffprobe's
/// units (see `VideoCodecSupport::max_level`); a missing or negative
/// level falls back to a generous default. `None` for codecs HLS can't
/// carry.
pub fn video_codec_string(
    codec: VideoCodec,
    profile: Option<VideoProfile>,
    level: Option<f32>,
    bit_depth: Option<u8>,
) -> Option<String> {
    let level = level.filter(|l| *l > 0.0).map(|l| l.round() as u32);
    let depth = bit_depth.unwrap_or(8);
    Some(match codec {
        VideoCodec::H264 => {
            // avc1.PPCCLL: profile_idc, constraint flags, level_idc.
            let (profile_idc, constraints) = match profile {
                Some(VideoProfile::H264ConstrainedBaseline) => (0x42, 0x40),
                Some(VideoProfile::H264Baseline) => (0x42, 0x00),
                Some(VideoProfile::H264Main) => (0x4D, 0x00),
                Some(VideoProfile::H264High10) => (0x6E, 0x00),
                _ if depth > 8 => (0x6E, 0x00),
                _ => (0x64, 0x00),
            };
            format!(
                "avc1.{profile_idc:02X}{constraints:02X}{:02X}",
                level.unwrap_or(40).min(255)
            )
        }
        VideoCodec::H265 => {
            // hvc1.<profile_idc>.<compat flags, reversed hex>.L<level_idc>.B0
            let (profile_idc, compat) = match profile {
                Some(VideoProfile::HevcMain10) => (2, "4"),
                Some(VideoProfile::HevcHigh | VideoProfile::HevcHigh10) => (4, "10"),
                _ if depth > 8 => (2, "4"),
                _ => (1, "6"),
            };
            format!("hvc1.{profile_idc}.{compat}.L{}.B0", level.unwrap_or(120))
        }
        VideoCodec::Av1 => {
            // av01.<profile>.<seq_level_idx><tier>.<bit depth>
            let p = match profile {
                Some(VideoProfile::Av1High) => 1,
                Some(VideoProfile::Av1Professional) => 2,
                _ => 0,
            };
            format!("av01.{p}.{:02}M.{depth:02}", level.unwrap_or(8))
        }
        VideoCodec::Vp9 => {
            let p = match profile {
                Some(VideoProfile::Vp9Profile1) => 1,
                Some(VideoProfile::Vp9Profile2) => 2,
                Some(VideoProfile::Vp9Profile3) => 3,
                _ if depth > 8 => 2,
                _ => 0,
            };
            format!("vp09.{p:02}.{:02}.{depth:02}", level.unwrap_or(41))
        }
        _ => return None,
    })
}

/// Codec string for our own 8-bit SDR encodes; the level is sized to
/// the output height since encoders choose it themselves.
pub fn encoded_video_codec_string(
    codec: VideoCodec,
    profile: VideoProfile,
    height: u32,
) -> Option<String> {
    let (h264, hevc, av1) = match height {
        0..=720 => (31, 93, 5),
        721..=1080 => (40, 120, 8),
        1081..=1440 => (50, 150, 12),
        _ => (51, 153, 13),
    };
    let level = match codec {
        VideoCodec::H264 => h264,
        VideoCodec::H265 => hevc,
        VideoCodec::Av1 => av1,
        _ => return None,
    };
    video_codec_string(codec, Some(profile), Some(level as f32), Some(8))
}

/// RFC 6381 audio string. `aac_profile` is ffprobe's profile name for
/// AAC (`HE-AAC`, `HE-AACv2`, `LC`). `None` for codecs with no HLS
/// signalling.
pub fn audio_codec_string(codec: AudioCodec, aac_profile: Option<&str>) -> Option<&'static str> {
    Some(match codec {
        AudioCodec::Aac => match aac_profile {
            Some("HE-AACv2") => "mp4a.40.29",
            Some("HE-AAC") => "mp4a.40.5",
            _ => "mp4a.40.2",
        },
        AudioCodec::Mp3 => "mp4a.40.34",
        AudioCodec::Ac3 => "ac-3",
        AudioCodec::Eac3 => "ec-3",
        AudioCodec::Opus => "opus",
        AudioCodec::Flac => "fLaC",
        _ => return None,
    })
}

/// Dolby Vision codec string (`dvh1.08.06`, `dav1.10.06`, `dvav.09.05`)
/// with the DoVi level derived from the base layer's level.
pub fn dovi_codec_string(
    codec: VideoCodec,
    profile: DolbyVisionProfile,
    level: Option<f32>,
) -> String {
    let level = level.filter(|l| *l > 0.0).map(|l| l.round() as u32);
    let (fourcc, dv_profile) = match codec {
        VideoCodec::Av1 => ("dav1", 10),
        VideoCodec::H264 => ("dvav", 9),
        _ => ("dvh1", profile.as_u8()),
    };
    format!(
        "{fourcc}.{dv_profile:02}.{:02}",
        dovi_level(codec, level.unwrap_or(0))
    )
}

/// Map a base-layer level onto the lowest Dolby Vision level whose max
/// pixel rate covers it. The tables are mirrored in
/// `lib/ts/codecSupport.ts` for probing; a level beyond every row
/// maps to the top DoVi level 13.
pub fn dovi_level(codec: VideoCodec, level: u32) -> u8 {
    // (max base level, DoVi level); compared with <=.
    const HEVC: &[(u32, u8)] = &[
        // HEVC level_idc = level × 30.
        (90, 1),   // 3.0  16_588_800  ->  22_118_400
        (93, 3),   // 3.1  33_177_600  ->  49_766_400
        (120, 4),  // 4.0  66_846_720  -> 124_416_000
        (123, 6),  // 4.1 133_693_440  -> 199_065_600
        (150, 7),  // 5.0 267_386_880  -> 248_832_000
        (153, 10), // 5.1 534_773_760  -> 995_328_000
        (180, 11), // 6.0
    ];
    const AV1: &[(u32, u8)] = &[
        // seq_level_idx.
        (4, 1),
        (5, 3),
        (8, 4),
        (9, 6),
        (12, 7),
        (13, 10),
        (16, 11),
    ];
    const H264: &[(u32, u8)] = &[
        // level × 10, Max PPS at 16×16 macroblocks.
        (30, 1),
        (31, 2),
        (41, 5),
        (50, 6),
        (51, 8),
        (52, 10),
        (60, 12),
    ];
    let table = match codec {
        VideoCodec::Av1 => AV1,
        VideoCodec::H264 => H264,
        _ => HEVC,
    };
    table
        .iter()
        .find(|(max, _)| level <= *max)
        .map_or(13, |(_, dv)| *dv)
}

pub fn render_master(variants: &[HlsVariant]) -> String {
    let mut out = String::with_capacity(64 + variants.len() * 192);
    out.push_str("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    for v in variants {
        let mut attrs = format!("BANDWIDTH={}", v.bandwidth_bps);
        if let Some((w, h)) = v.resolution {
            let _ = write!(attrs, ",RESOLUTION={w}x{h}");
        }
        if !v.codecs.is_empty() {
            let _ = write!(attrs, ",CODECS=\"{}\"", v.codecs);
        }
        if let Some(s) = &v.supplemental_codecs {
            let _ = write!(attrs, ",SUPPLEMENTAL-CODECS=\"{s}\"");
        }
        if let Some(range) = v.video_range {
            let name = match range {
                HlsVideoRange::Sdr => "SDR",
                HlsVideoRange::Pq => "PQ",
                HlsVideoRange::Hlg => "HLG",
            };
            let _ = write!(attrs, ",VIDEO-RANGE={name}");
        }
        let _ = writeln!(out, "#EXT-X-STREAM-INF:{attrs}\n{}", v.path);
    }
    out
}

#[cfg(test)]
mod tests {
    use noflex_core::{ChannelLayout, HdrTransfer, PixelFormat};

    use super::*;
    use crate::{
        decision::HlsSegmentContainer,
        probe::{AudioStreamProbe, VideoStreamProbe},
    };

    fn probe(codec: VideoCodec, profile: VideoProfile, level: f32, depth: u8) -> ProbeReport {
        ProbeReport {
            container: Some("matroska".into()),
            video: Some(VideoStreamProbe {
                codec,
                profile: Some(profile),
                level: Some(level),
                width: 3840,
                height: 2160,
                bit_depth: Some(depth),
                pixel_format: Some(PixelFormat::Yuv420p10le),
                frame_rate: Some(24.0),
                color_space: None,
                transfer: Some(HdrTransfer::Pq),
                matrix: None,
                dovi_profile: None,
                side_data: Default::default(),
            }),
            audio: vec![AudioStreamProbe {
                index: 1,
                codec: "eac3".into(),
                profile: None,
                bit_depth: None,
                sample_rate: Some(48_000),
                channels: 6,
                channel_layout: Some(ChannelLayout::Six),
                language: None,
                default: true,
                forced: false,
                title: None,
                audio_object_type: None,
            }],
            subtitle: vec![],
            format_bit_rate: Some(25_000_000),
            duration_ms: None,
        }
    }

    #[test]
    fn h264_strings_use_level_idc_hex() {
        assert_eq!(
            video_codec_string(
                VideoCodec::H264,
                Some(VideoProfile::H264High),
                Some(40.0),
                Some(8)
            )
            .unwrap(),
            "avc1.640028"
        );
        assert_eq!(
            video_codec_string(
                VideoCodec::H264,
                Some(VideoProfile::H264Main),
                Some(31.0),
                Some(8)
            )
            .unwrap(),
            "avc1.4D001F"
        );
        assert_eq!(
            video_codec_string(
                VideoCodec::H264,
                Some(VideoProfile::H264ConstrainedBaseline),
                Some(30.0),
                None
            )
            .unwrap(),
            "avc1.42401E"
        );
        assert_eq!(
            video_codec_string(VideoCodec::H264, None, None, Some(10)).unwrap(),
            "avc1.6E0028"
        );
    }

    #[test]
    fn hevc_strings_use_profile_compat_and_level_idc() {
        assert_eq!(
            video_codec_string(
                VideoCodec::H265,
                Some(VideoProfile::HevcMain10),
                Some(153.0),
                Some(10)
            )
            .unwrap(),
            "hvc1.2.4.L153.B0"
        );
        assert_eq!(
            video_codec_string(
                VideoCodec::H265,
                Some(VideoProfile::HevcMain),
                Some(120.0),
                Some(8)
            )
            .unwrap(),
            "hvc1.1.6.L120.B0"
        );
    }

    #[test]
    fn av1_and_vp9_strings() {
        assert_eq!(
            video_codec_string(
                VideoCodec::Av1,
                Some(VideoProfile::Av1Main),
                Some(12.0),
                Some(10)
            )
            .unwrap(),
            "av01.0.12M.10"
        );
        assert_eq!(
            video_codec_string(
                VideoCodec::Vp9,
                Some(VideoProfile::Vp9Profile2),
                Some(-99.0),
                Some(10)
            )
            .unwrap(),
            "vp09.02.41.10"
        );
        assert!(video_codec_string(VideoCodec::Mpeg2, None, None, None).is_none());
    }

    #[test]
    fn audio_strings() {
        assert_eq!(
            audio_codec_string(AudioCodec::Aac, Some("LC")),
            Some("mp4a.40.2")
        );
        assert_eq!(
            audio_codec_string(AudioCodec::Aac, Some("HE-AAC")),
            Some("mp4a.40.5")
        );
        assert_eq!(audio_codec_string(AudioCodec::Eac3, None), Some("ec-3"));
        assert_eq!(audio_codec_string(AudioCodec::Flac, None), Some("fLaC"));
        assert_eq!(audio_codec_string(AudioCodec::Truehd, None), None);
    }

    #[test]
    fn dovi_levels_follow_the_tables() {
        assert_eq!(dovi_level(VideoCodec::H265, 90), 1);
        assert_eq!(dovi_level(VideoCodec::H265, 120), 4);
        assert_eq!(dovi_level(VideoCodec::H265, 153), 10);
        assert_eq!(dovi_level(VideoCodec::H265, 186), 13);
        assert_eq!(dovi_level(VideoCodec::Av1, 13), 10);
        assert_eq!(dovi_level(VideoCodec::H264, 40), 5);
        assert_eq!(
            dovi_codec_string(VideoCodec::H265, DolbyVisionProfile::Profile8, Some(153.0)),
            "dvh1.08.10"
        );
    }

    #[test]
    fn copied_hevc_hdr_variant_announces_source_codecs_and_pq() {
        let p = probe(VideoCodec::H265, VideoProfile::HevcMain10, 153.0, 10);
        let plan = TranscodePlan {
            segment_container: HlsSegmentContainer::Fmp4,
            video: VideoPlan::Copy {
                codec: VideoCodec::H265,
                range: VideoRange::Hdr10,
            },
            audio: AudioPlan::Copy {
                codec: AudioCodec::Eac3,
            },
        };
        let v = HlsVariant::for_plan(&p, &plan, Some(0), None, "index.m3u8");
        assert_eq!(v.codecs, "hvc1.2.4.L153.B0,ec-3");
        assert_eq!(v.video_range, Some(HlsVideoRange::Pq));
        assert_eq!(v.resolution, Some((3840, 2160)));
        assert_eq!(v.bandwidth_bps, 25_000_000);
        let m = render_master(&[v]);
        assert!(m.contains(
            "#EXT-X-STREAM-INF:BANDWIDTH=25000000,RESOLUTION=3840x2160,CODECS=\"hvc1.2.4.L153.B0,ec-3\",VIDEO-RANGE=PQ\nindex.m3u8\n"
        ), "{m}");
    }

    #[test]
    fn dovi_with_hdr10_copy_announces_dovi_as_supplemental() {
        let mut p = probe(VideoCodec::H265, VideoProfile::HevcMain10, 153.0, 10);
        p.video.as_mut().unwrap().dovi_profile = Some(DolbyVisionProfile::Profile8);
        let mut plan = TranscodePlan {
            segment_container: HlsSegmentContainer::Fmp4,
            video: VideoPlan::Copy {
                codec: VideoCodec::H265,
                range: VideoRange::DoviWithHdr10,
            },
            audio: AudioPlan::None,
        };
        let v = HlsVariant::for_plan(&p, &plan, None, None, "index.m3u8");
        assert_eq!(v.supplemental_codecs.as_deref(), Some("dvh1.08.10"));
        assert_eq!(v.codecs, "hvc1.2.4.L153.B0");
        plan.video = VideoPlan::Copy {
            codec: VideoCodec::H265,
            range: VideoRange::Hdr10,
        };
        let v = HlsVariant::for_plan(&p, &plan, None, None, "index.m3u8");
        assert_eq!(v.supplemental_codecs, None);
    }

    #[test]
    fn encoded_variant_announces_ladder_output() {
        let p = probe(VideoCodec::H265, VideoProfile::HevcMain10, 153.0, 10);
        let plan = TranscodePlan {
            segment_container: HlsSegmentContainer::Ts,
            video: VideoPlan::Encode {
                codec: VideoCodec::H264,
                width: 1920,
                height: 1080,
                tonemap: true,
                profile: VideoProfile::H264High,
            },
            audio: AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: 2,
            },
        };
        let encoded = EncodedVideo {
            width: 1920,
            height: 1080,
            bitrate_bps: 8_000_000,
        };
        let v = HlsVariant::for_plan(&p, &plan, Some(0), Some(encoded), "index.m3u8");
        assert_eq!(v.codecs, "avc1.640028,mp4a.40.2");
        assert_eq!(v.bandwidth_bps, 8_128_000);
        assert_eq!(v.video_range, Some(HlsVideoRange::Sdr));
    }

    #[test]
    fn empty_master_is_well_formed() {
        let m = render_master(&[]);
        assert_eq!(
            m,
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n"
        );
    }
}
