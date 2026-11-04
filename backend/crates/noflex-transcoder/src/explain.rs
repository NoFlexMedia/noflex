// SPDX-License-Identifier: AGPL-3.0-only
//
// The decision explanation surfaced on `PlaybackSession.decision` and
// the admin `activeStreams` query: what the source is, what the client
// receives, why, and which encoder does the work.

use async_graphql::{Enum, SimpleObject};
use noflex_core::{VideoCodec, VideoProfile};
use serde::{Deserialize, Serialize};

use crate::{
    decision::{AudioPlan, PlaybackDecision, PlaybackRoute, TranscodePlan, VideoPlan, VideoRange},
    encoder::Encoder,
    ffmpeg_argv::audio_encoder_flag,
    hls::{EncodedVideo, audio_bitrate_bps},
    probe::ProbeReport,
    reasons::{TranscodeReason, TranscodeReasons},
};

/// How a started session serves its bytes.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackRouteKind {
    Direct,
    Transcode,
    /// HLS over a version prepared ahead of time: no job to manage, and seeking
    /// is ordinary, with no restart.
    Optimized,
}

/// The per-stream shape of the work done.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackPlanKind {
    /// The file as stored.
    DirectPlay,
    /// Both streams copied into HLS segments.
    Remux,
    /// Video copied, audio re-encoded.
    TranscodeAudio,
    /// Video re-encoded, audio copied (or absent).
    TranscodeVideo,
    /// Both re-encoded.
    Transcode,
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackReason {
    pub code: TranscodeReason,
    pub description: String,
}

/// One side of the pipeline. Fields are `None` when unknown or when
/// the stream is absent.
#[derive(SimpleObject, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamSummary {
    /// ffprobe's container name for the source; `fmp4` / `mpegts`
    /// segments for an HLS target.
    pub container: Option<String>,
    pub video_codec: Option<VideoCodec>,
    pub video_profile: Option<VideoProfile>,
    /// ffprobe units (see `VideoCodecSupport.maxLevel`).
    pub video_level: Option<f32>,
    pub bit_depth: Option<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video_range: Option<VideoRange>,
    /// ffprobe's audio codec name (open-ended: `truehd`, `pcm_s24le`, …).
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u8>,
    pub bitrate_bps: Option<u64>,
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackDecisionInfo {
    pub route: PlaybackRouteKind,
    pub plan: PlaybackPlanKind,
    /// Empty only for direct play.
    pub reasons: Vec<PlaybackReason>,
    pub source: StreamSummary,
    pub target: StreamSummary,
    /// ffmpeg video encoder (`libx264`, `h264_vaapi`), `copy` when the
    /// video is stream-copied, `None` for direct play.
    pub encoder: Option<String>,
    /// ffmpeg audio encoder or `copy`; `None` for direct play or no audio.
    pub audio_encoder: Option<String>,
    /// Whether the video encoder runs on a GPU / media engine.
    pub hardware: bool,
    /// HDR is being tonemapped to SDR.
    pub tonemap: bool,
}

impl PlaybackDecisionInfo {
    /// Explain `decision` for `probe`. `encoded` carries the ladder's
    /// output geometry and `encoder` the video encoder, both consulted
    /// only when the plan re-encodes video.
    pub fn new(
        probe: &ProbeReport,
        decision: &PlaybackDecision,
        encoder: Encoder,
        encoded: Option<EncodedVideo>,
    ) -> Self {
        let source = source_summary(probe, decision.audio_stream);
        match &decision.route {
            PlaybackRoute::Direct => Self {
                route: PlaybackRouteKind::Direct,
                plan: PlaybackPlanKind::DirectPlay,
                reasons: Vec::new(),
                target: source.clone(),
                source,
                encoder: None,
                audio_encoder: None,
                hardware: false,
                tonemap: false,
            },
            PlaybackRoute::Transcoding { plan, reasons } => {
                Self::transcode(source, probe, plan, *reasons, encoder, encoded)
            }
        }
    }

    /// A live channel: always transcoded to `plan`, source unprobed.
    pub fn live(plan: &TranscodePlan, encoder: Encoder, encoded: Option<EncodedVideo>) -> Self {
        Self::transcode(
            StreamSummary::default(),
            &ProbeReport::default(),
            plan,
            TranscodeReasons::LiveSource,
            encoder,
            encoded,
        )
    }

    fn transcode(
        source: StreamSummary,
        probe: &ProbeReport,
        plan: &TranscodePlan,
        reasons: TranscodeReasons,
        encoder: Encoder,
        encoded: Option<EncodedVideo>,
    ) -> Self {
        let mut target = StreamSummary {
            container: Some(plan.segment_container.as_str().to_string()),
            ..Default::default()
        };
        let mut video_encoder = None;
        let mut hardware = false;
        let mut tonemap = false;
        match plan.video {
            VideoPlan::None => {}
            VideoPlan::Copy { range, .. } => {
                target.video_codec = source.video_codec;
                target.video_profile = source.video_profile;
                target.video_level = source.video_level;
                target.bit_depth = source.bit_depth;
                target.width = source.width;
                target.height = source.height;
                target.video_range = Some(range);
                target.bitrate_bps = probe.format_bit_rate;
                video_encoder = Some("copy".to_string());
            }
            VideoPlan::Encode {
                codec,
                tonemap: tm,
                profile,
                ..
            } => {
                target.video_codec = Some(codec);
                target.video_profile = Some(profile);
                target.bit_depth = Some(8);
                target.video_range = Some(VideoRange::Sdr);
                if let Some(out) = encoded {
                    target.width = Some(out.width);
                    target.height = Some(out.height);
                    target.bitrate_bps = Some(out.bitrate_bps);
                }
                video_encoder = Some(encoder.video_flag().to_string());
                hardware = encoder.is_hardware();
                tonemap = tm;
            }
        }
        let audio_encoder = match plan.audio {
            AudioPlan::None => None,
            AudioPlan::Copy { .. } => {
                target.audio_codec = source.audio_codec.clone();
                target.audio_channels = source.audio_channels;
                Some("copy".to_string())
            }
            AudioPlan::Encode { codec, channels } => {
                target.audio_codec = Some(codec.as_str().to_string());
                target.audio_channels = Some(channels);
                if plan.video == VideoPlan::None {
                    target.bitrate_bps = Some(audio_bitrate_bps(codec, channels));
                }
                Some(audio_encoder_flag(codec).to_string())
            }
        };
        let plan_kind = match (plan.copies_video(), plan.copies_audio()) {
            (true, true) => PlaybackPlanKind::Remux,
            (true, false) => PlaybackPlanKind::TranscodeAudio,
            (false, true) => PlaybackPlanKind::TranscodeVideo,
            (false, false) => PlaybackPlanKind::Transcode,
        };
        Self {
            route: PlaybackRouteKind::Transcode,
            plan: plan_kind,
            reasons: reasons
                .codes()
                .into_iter()
                .map(|code| PlaybackReason {
                    code,
                    description: code.description().to_string(),
                })
                .collect(),
            source,
            target,
            encoder: video_encoder,
            audio_encoder,
            hardware,
            tonemap,
        }
    }
}

fn source_summary(probe: &ProbeReport, audio_stream: Option<usize>) -> StreamSummary {
    let video = probe.video.as_ref();
    let audio = audio_stream.and_then(|i| probe.audio.get(i));
    StreamSummary {
        container: probe.container.clone(),
        video_codec: video.map(|v| v.codec),
        video_profile: video.and_then(|v| v.profile),
        video_level: video.and_then(|v| v.level).filter(|l| *l > 0.0),
        bit_depth: video.and_then(|v| v.bit_depth),
        width: video.map(|v| v.width),
        height: video.map(|v| v.height),
        video_range: video.map(VideoRange::of),
        audio_codec: audio.map(|a| a.codec.clone()),
        audio_channels: audio.map(|a| a.channels),
        bitrate_bps: probe.format_bit_rate,
    }
}

#[cfg(test)]
mod tests {
    use noflex_core::{AudioCodec, HdrTransfer, PixelFormat};

    use super::*;
    use crate::{
        decision::{ClientCapabilities, DecideOptions, decide},
        probe::{AudioStreamProbe, VideoStreamProbe},
    };

    fn hevc_truehd() -> ProbeReport {
        ProbeReport {
            container: Some("matroska".into()),
            video: Some(VideoStreamProbe {
                codec: VideoCodec::H265,
                profile: Some(VideoProfile::HevcMain10),
                level: Some(153.0),
                width: 3840,
                height: 2160,
                bit_depth: Some(10),
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
                codec: "truehd".into(),
                profile: None,
                bit_depth: None,
                sample_rate: None,
                channels: 8,
                channel_layout: None,
                language: None,
                default: true,
                forced: false,
                title: None,
                audio_object_type: None,
            }],
            subtitle: vec![],
            format_bit_rate: Some(50_000_000),
            duration_ms: None,
        }
    }

    #[test]
    fn full_transcode_explains_source_target_and_encoder() {
        let p = hevc_truehd();
        let d = decide(&p, &ClientCapabilities::web(), &DecideOptions::default());
        let out = EncodedVideo {
            width: 1920,
            height: 1080,
            bitrate_bps: 8_000_000,
        };
        let info = PlaybackDecisionInfo::new(&p, &d, Encoder::Vaapi264, Some(out));
        assert_eq!(info.route, PlaybackRouteKind::Transcode);
        assert_eq!(info.plan, PlaybackPlanKind::Transcode);
        assert!(info.tonemap);
        assert!(info.hardware);
        assert_eq!(info.encoder.as_deref(), Some("h264_vaapi"));
        assert_eq!(info.audio_encoder.as_deref(), Some("aac"));
        assert_eq!(info.source.video_range, Some(VideoRange::Hdr10));
        assert_eq!(info.source.audio_codec.as_deref(), Some("truehd"));
        assert_eq!(info.target.video_codec, Some(VideoCodec::H264));
        assert_eq!(info.target.height, Some(1080));
        assert_eq!(
            info.target.audio_codec.as_deref(),
            Some(AudioCodec::Aac.as_str())
        );
        assert_eq!(info.target.audio_channels, Some(6));
        assert_eq!(info.target.container.as_deref(), Some("fmp4"));
        let codes: Vec<_> = info.reasons.iter().map(|r| r.code).collect();
        assert!(codes.contains(&TranscodeReason::VideoCodecNotSupported));
        assert!(codes.contains(&TranscodeReason::AudioCodecNotSupported));
        assert!(info.reasons.iter().all(|r| !r.description.is_empty()));
    }

    #[test]
    fn direct_play_mirrors_source() {
        let mut p = hevc_truehd();
        p.container = Some("mov".into());
        let v = p.video.as_mut().unwrap();
        v.codec = VideoCodec::H264;
        v.profile = Some(VideoProfile::H264High);
        v.level = Some(40.0);
        v.bit_depth = Some(8);
        v.pixel_format = Some(PixelFormat::Yuv420p);
        v.transfer = None;
        v.height = 1080;
        v.width = 1920;
        p.audio[0].codec = "aac".into();
        p.audio[0].channels = 2;
        let d = decide(&p, &ClientCapabilities::web(), &DecideOptions::default());
        let info = PlaybackDecisionInfo::new(&p, &d, Encoder::Software264, None);
        assert_eq!(info.plan, PlaybackPlanKind::DirectPlay);
        assert!(info.reasons.is_empty());
        assert_eq!(info.source, info.target);
        assert_eq!(info.encoder, None);
        assert!(!info.hardware);
    }
}
