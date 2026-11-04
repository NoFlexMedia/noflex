// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{
    AudioCodec, AudioObjectType, ChannelLayout, ColorPrimaries, DolbyVisionProfile, HdrTransfer,
    MatrixCoefficients, PixelFormat, SubtitleCodec, VideoCodec, VideoProfile,
};
use noflex_transcoder::{
    AudioCodecSupport, AudioPlan, ClientCapabilities, DecideOptions, DirectPlayProfile, HlsProfile,
    HlsSegmentContainer, PlaybackRoute, TranscodeReasons, VideoCodecSupport, VideoPlan, VideoRange,
    decide,
    probe::{AudioStreamProbe, ProbeReport, SubtitleStreamProbe, VideoStreamProbe},
};
use proptest::prelude::*;

fn arb_video_codec() -> impl Strategy<Value = VideoCodec> {
    prop_oneof![
        Just(VideoCodec::H264),
        Just(VideoCodec::H265),
        Just(VideoCodec::Av1),
        Just(VideoCodec::Vp9),
        Just(VideoCodec::Vp8),
        Just(VideoCodec::Mpeg2),
        Just(VideoCodec::Prores),
        Just(VideoCodec::Theora),
    ]
}

fn arb_video_profile() -> impl Strategy<Value = Option<VideoProfile>> {
    prop_oneof![
        Just(None),
        Just(Some(VideoProfile::H264Baseline)),
        Just(Some(VideoProfile::H264Main)),
        Just(Some(VideoProfile::H264High)),
        Just(Some(VideoProfile::H264High10)),
        Just(Some(VideoProfile::HevcMain)),
        Just(Some(VideoProfile::HevcMain10)),
        Just(Some(VideoProfile::HevcHigh)),
        Just(Some(VideoProfile::Av1Main)),
        Just(Some(VideoProfile::Av1High)),
    ]
}

fn arb_pixel_format() -> impl Strategy<Value = Option<PixelFormat>> {
    prop_oneof![
        Just(None),
        Just(Some(PixelFormat::Yuv420p)),
        Just(Some(PixelFormat::Yuv420p10le)),
        Just(Some(PixelFormat::Yuv422p)),
        Just(Some(PixelFormat::Yuv444p)),
    ]
}

fn arb_hdr_transfer() -> impl Strategy<Value = HdrTransfer> {
    prop_oneof![
        Just(HdrTransfer::Sdr),
        Just(HdrTransfer::Pq),
        Just(HdrTransfer::Hlg),
    ]
}

fn arb_bit_depth() -> impl Strategy<Value = Option<u8>> {
    prop_oneof![Just(None), Just(Some(8)), Just(Some(10)), Just(Some(12)),]
}

fn arb_height() -> impl Strategy<Value = u32> {
    prop_oneof![Just(480u32), Just(720), Just(1080), Just(2160),]
}

fn arb_level() -> impl Strategy<Value = Option<f32>> {
    prop_oneof![
        Just(None),
        Just(Some(-99.0f32)),
        Just(Some(8.0f32)),
        Just(Some(40.0f32)),
        Just(Some(51.0f32)),
        Just(Some(120.0f32)),
        Just(Some(153.0f32)),
    ]
}

fn arb_dovi() -> impl Strategy<Value = Option<DolbyVisionProfile>> {
    prop_oneof![
        Just(None),
        Just(Some(DolbyVisionProfile::Profile5)),
        Just(Some(DolbyVisionProfile::Profile7)),
        Just(Some(DolbyVisionProfile::Profile8)),
        Just(Some(DolbyVisionProfile::Profile9)),
    ]
}

fn arb_audio_codec() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("aac".to_string()),
        Just("opus".to_string()),
        Just("ac3".to_string()),
        Just("eac3".to_string()),
        Just("flac".to_string()),
        Just("truehd".to_string()),
    ]
}

fn arb_client_audio_codec() -> impl Strategy<Value = AudioCodec> {
    prop_oneof![
        Just(AudioCodec::Aac),
        Just(AudioCodec::Opus),
        Just(AudioCodec::Ac3),
        Just(AudioCodec::Eac3),
        Just(AudioCodec::Flac),
        Just(AudioCodec::Mp3),
    ]
}

fn arb_range() -> impl Strategy<Value = VideoRange> {
    prop_oneof![
        Just(VideoRange::Sdr),
        Just(VideoRange::Hdr10),
        Just(VideoRange::Hlg),
        Just(VideoRange::Dovi),
        Just(VideoRange::DoviWithHdr10),
        Just(VideoRange::DoviWithSdr),
    ]
}

fn arb_container() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("mp4".to_string()),
        Just("mkv".to_string()),
        Just("webm".to_string()),
        Just("ts".to_string()),
    ]
}

fn arb_audio_channels() -> impl Strategy<Value = u8> {
    prop_oneof![Just(1u8), Just(2u8), Just(6u8), Just(8u8),]
}

fn arb_audio_object() -> impl Strategy<Value = Option<AudioObjectType>> {
    prop_oneof![
        Just(None),
        Just(Some(AudioObjectType::None)),
        Just(Some(AudioObjectType::Joc)),
        Just(Some(AudioObjectType::Adm)),
        Just(Some(AudioObjectType::MpegH)),
    ]
}

fn arb_subtitle_codec() -> impl Strategy<Value = SubtitleCodec> {
    prop_oneof![
        Just(SubtitleCodec::SubRip),
        Just(SubtitleCodec::PgsImage),
        Just(SubtitleCodec::MovText),
        Just(SubtitleCodec::Ass),
        Just(SubtitleCodec::WebVtt),
    ]
}

fn arb_video_stream() -> impl Strategy<Value = VideoStreamProbe> {
    (
        arb_video_codec(),
        arb_video_profile(),
        arb_level(),
        arb_height(),
        arb_bit_depth(),
        arb_pixel_format(),
        arb_hdr_transfer(),
        arb_dovi(),
    )
        .prop_map(
            |(codec, profile, level, height, bit_depth, pixel_format, transfer, dovi)| {
                VideoStreamProbe {
                    codec,
                    profile,
                    level,
                    width: height * 16 / 9,
                    height,
                    bit_depth,
                    pixel_format,
                    frame_rate: Some(24.0),
                    color_space: Some(ColorPrimaries::Bt709),
                    transfer: Some(transfer),
                    matrix: Some(MatrixCoefficients::Bt709),
                    dovi_profile: dovi,
                    side_data: Default::default(),
                }
            },
        )
}

fn arb_audio_stream() -> impl Strategy<Value = AudioStreamProbe> {
    (
        arb_audio_codec(),
        arb_audio_channels(),
        0u32..3u32,
        arb_audio_object(),
    )
        .prop_map(|(codec, channels, idx, audio_object_type)| {
            let object_str = audio_object_type
                .and_then(|o| {
                    if o == AudioObjectType::None {
                        None
                    } else {
                        Some(o.as_str())
                    }
                })
                .map(str::to_string);
            AudioStreamProbe {
                index: idx,
                codec,
                profile: Some("LC".to_string()),
                bit_depth: Some(16),
                sample_rate: Some(48_000),
                channels,
                channel_layout: match channels {
                    1 => Some(ChannelLayout::Mono),
                    2 => Some(ChannelLayout::Stereo),
                    6 => Some(ChannelLayout::Six),
                    _ => Some(ChannelLayout::Seven1),
                },
                language: Some("eng".to_string()),
                default: true,
                forced: false,
                title: None,
                audio_object_type: object_str,
            }
        })
}

fn arb_subtitle_stream() -> impl Strategy<Value = SubtitleStreamProbe> {
    (arb_subtitle_codec(), 2u32..5u32).prop_map(|(codec, idx)| SubtitleStreamProbe {
        index: idx,
        codec,
        language: Some("eng".to_string()),
        default: true,
        forced: false,
        external: false,
        title: None,
    })
}

fn arb_probe_report() -> impl Strategy<Value = ProbeReport> {
    (
        prop::option::of(prop_oneof![
            Just("mov".to_string()),
            Just("matroska".to_string()),
            Just("mpegts".to_string()),
            "[a-z]{1,8}"
        ]),
        prop::option::of(arb_video_stream()),
        prop::collection::vec(arb_audio_stream(), 0..3),
        prop::collection::vec(arb_subtitle_stream(), 0..2),
        prop::option::of(0u64..50_000_000u64),
    )
        .prop_map(
            |(container, video, audio, subtitle, format_bit_rate)| ProbeReport {
                container,
                video,
                audio,
                subtitle,
                format_bit_rate,
                duration_ms: None,
            },
        )
}

fn arb_direct_profile() -> impl Strategy<Value = DirectPlayProfile> {
    (
        arb_container(),
        prop::collection::vec(arb_video_codec(), 0..4),
        prop::collection::vec(arb_client_audio_codec(), 0..4),
    )
        .prop_map(
            |(container, video_codecs, audio_codecs)| DirectPlayProfile {
                container,
                video_codecs,
                audio_codecs,
            },
        )
}

fn arb_hls_profile() -> impl Strategy<Value = HlsProfile> {
    (
        prop_oneof![
            Just(HlsSegmentContainer::Fmp4),
            Just(HlsSegmentContainer::Ts)
        ],
        prop::collection::vec(arb_video_codec(), 0..4),
        prop::collection::vec(arb_client_audio_codec(), 0..4),
    )
        .prop_map(
            |(segment_container, video_codecs, audio_codecs)| HlsProfile {
                segment_container,
                video_codecs,
                audio_codecs,
            },
        )
}

fn arb_video_support() -> impl Strategy<Value = VideoCodecSupport> {
    (
        arb_video_codec(),
        prop::collection::vec(arb_video_profile().prop_filter_map("some", |p| p), 0..3),
        arb_level(),
        arb_bit_depth(),
        prop::option::of(arb_height()),
        prop::collection::vec(arb_range(), 0..3),
    )
        .prop_map(
            |(codec, profiles, max_level, max_bit_depth, max_height, video_ranges)| {
                VideoCodecSupport {
                    codec,
                    profiles,
                    max_level,
                    max_bit_depth,
                    max_width: max_height.map(|h| h * 16 / 9),
                    max_height,
                    max_pixels: max_height.map(|h| u64::from(h) * u64::from(h) * 16 / 9),
                    video_ranges,
                }
            },
        )
}

fn arb_client_caps() -> impl Strategy<Value = ClientCapabilities> {
    (
        prop::collection::vec(arb_direct_profile(), 0..3),
        prop::collection::vec(arb_hls_profile(), 0..3),
        prop::collection::vec(arb_video_support(), 0..3),
        prop::collection::vec(
            (arb_client_audio_codec(), prop::option::of(1u8..9u8)).prop_map(
                |(codec, max_channels)| AudioCodecSupport {
                    codec,
                    max_channels,
                },
            ),
            0..3,
        ),
        prop::option::of(arb_height()),
        prop::option::of(1_000_000u64..40_000_000u64),
    )
        .prop_map(
            |(direct_play, hls, video_codecs, audio_codecs, max_height, max_bitrate_bps)| {
                ClientCapabilities {
                    direct_play,
                    hls,
                    video_codecs,
                    audio_codecs,
                    subtitle_codecs: vec![SubtitleCodec::WebVtt],
                    max_height,
                    max_bitrate_bps,
                }
            },
        )
}

fn opts() -> DecideOptions {
    DecideOptions::default()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn transcoding_always_explains_itself(probe in arb_probe_report(), client in arb_client_caps()) {
        if let PlaybackRoute::Transcoding { reasons, .. } = decide(&probe, &client, &opts()).route {
            prop_assert!(!reasons.is_empty());
            prop_assert_eq!(reasons.describe().len(), reasons.codes().len());
        }
    }

    #[test]
    fn decision_is_deterministic(probe in arb_probe_report(), client in arb_client_caps()) {
        prop_assert_eq!(decide(&probe, &client, &opts()), decide(&probe, &client, &opts()));
    }

    #[test]
    fn subtitles_never_change_the_decision(mut probe in arb_probe_report(), client in arb_client_caps()) {
        let with = decide(&probe, &client, &opts());
        probe.subtitle.clear();
        prop_assert_eq!(with, decide(&probe, &client, &opts()));
    }

    #[test]
    fn burn_in_never_direct_plays_or_copies_video(probe in arb_probe_report(), client in arb_client_caps()) {
        let burn = DecideOptions { burn_subtitle: true, ..opts() };
        match decide(&probe, &client, &burn).route {
            PlaybackRoute::Direct => prop_assert!(false, "burn-in direct played"),
            PlaybackRoute::Transcoding { plan, reasons } => {
                if probe.video.is_some() {
                    let encoded = matches!(plan.video, VideoPlan::Encode { .. });
                    prop_assert!(encoded);
                    prop_assert!(reasons.contains(TranscodeReasons::SubtitleBurnIn));
                }
            }
        }
    }

    #[test]
    fn copies_only_what_the_segments_carry(probe in arb_probe_report(), client in arb_client_caps()) {
        if let PlaybackRoute::Transcoding { plan, .. } = decide(&probe, &client, &opts()).route {
            match plan.video {
                VideoPlan::Copy { codec, .. } | VideoPlan::Encode { codec, .. } => {
                    prop_assert!(plan.segment_container.carries_video(codec));
                }
                VideoPlan::None => prop_assert!(probe.video.is_none()),
            }
            match plan.audio {
                AudioPlan::Copy { codec } | AudioPlan::Encode { codec, .. } => {
                    prop_assert!(plan.segment_container.carries_audio(codec));
                }
                AudioPlan::None => prop_assert!(probe.audio.is_empty()),
            }
        }
    }

    #[test]
    fn copied_video_is_presentable(probe in arb_probe_report(), client in arb_client_caps()) {
        if let PlaybackRoute::Transcoding { plan, .. } = decide(&probe, &client, &opts()).route
            && let (VideoPlan::Copy { codec, range }, Some(v)) = (plan.video, probe.video.as_ref())
        {
            prop_assert_eq!(codec, v.codec);
            let ranges = client.video_support(codec).map(|s| s.video_ranges.clone()).unwrap_or_default();
            prop_assert_eq!(VideoRange::of(v).delivered(&ranges), Some(range));
            if let Some(max) = client.max_height {
                prop_assert!(v.height <= max);
            }
        }
    }

    #[test]
    fn hdr_source_on_sdr_only_client_is_tonemapped(mut probe in arb_probe_report(), mut client in arb_client_caps()) {
        let Some(v) = probe.video.as_mut() else { return Ok(()) };
        v.transfer = Some(HdrTransfer::Pq);
        v.dovi_profile = None;
        for s in &mut client.video_codecs {
            s.video_ranges.clear();
        }
        match decide(&probe, &client, &opts()).route {
            PlaybackRoute::Direct => prop_assert!(false, "HDR direct played on SDR client"),
            PlaybackRoute::Transcoding { plan, .. } => {
                let tonemapped = matches!(plan.video, VideoPlan::Encode { tonemap: true, .. });
                prop_assert!(tonemapped);
            }
        }
    }

    #[test]
    fn bitrate_ceiling_is_respected(probe in arb_probe_report(), client in arb_client_caps()) {
        let over = matches!((client.max_bitrate_bps, probe.format_bit_rate), (Some(max), Some(bps)) if bps > max);
        if !over {
            return Ok(());
        }
        match decide(&probe, &client, &opts()).route {
            PlaybackRoute::Direct => prop_assert!(false, "over-bitrate source direct played"),
            PlaybackRoute::Transcoding { plan, reasons } => {
                if probe.video.is_some() {
                    let encoded = matches!(plan.video, VideoPlan::Encode { .. });
                    prop_assert!(encoded);
                    prop_assert!(reasons.contains(TranscodeReasons::ContainerBitrateExceedsLimit));
                }
            }
        }
    }

    #[test]
    fn encode_size_never_exceeds_source_or_limits(probe in arb_probe_report(), client in arb_client_caps()) {
        if let PlaybackRoute::Transcoding { plan, .. } = decide(&probe, &client, &opts()).route
            && let (VideoPlan::Encode { codec, width, height, .. }, Some(v)) = (plan.video, probe.video.as_ref())
        {
            prop_assert!(height <= v.height);
            prop_assert!(width <= v.width);
            if let Some(max) = client.max_height {
                prop_assert!(height <= max);
            }
            if let Some(support) = client.video_support(codec) {
                prop_assert!(support.fits(width, height), "{width}x{height} over {support:?}");
            }
        }
    }
}
