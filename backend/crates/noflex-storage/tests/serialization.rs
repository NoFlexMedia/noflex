// SPDX-License-Identifier: AGPL-3.0-only
//
// W3 round-trip serialization tests for the v2 Edition / AudioTrack /
// SubtitleTrack field set. These cover the new typed-enum columns
// independently of the Turso repo so a serde rename regression is
// caught before the conformance suite ever sees a malformed JSON.

use std::str::FromStr;

use noflex_core::{
    ChannelLayout, ColorPrimaries, DolbyVisionProfile, EditionId, HdrTransfer, MatrixCoefficients,
    PixelFormat, VideoCodec, VideoProfile, WorkId,
};
use noflex_storage::domain::{AudioTrack, Edition, SubtitleTrack};

fn full_edition() -> Edition {
    Edition {
        id: EditionId::new(),
        work_id: WorkId::new(),
        name: "Final Cut".to_string(),
        runtime_ms: Some(7_140_000),
        intro_start_ms: Some(125_000),
        intro_end_ms: Some(195_000),
        credits_start_ms: Some(7_060_000),
        chapters: vec![],
        codec_video: Some("h264".to_string()),
        codec_audio: Some("truehd".to_string()),
        width: Some(3840),
        height: Some(2160),
        bit_depth: Some(10),
        hdr_format: Some("hdr10".to_string()),
        audio_tracks: Some(vec![AudioTrack {
            language: Some("en".to_string()),
            codec: "truehd".to_string(),
            channels: 8,
            default: true,
            bit_depth: Some(24),
            sample_rate: Some(48_000),
            channel_layout: Some(ChannelLayout::Seven1),
            codec_profile: Some("TrueHD".to_string()),
            audio_object_type: Some("joc".to_string()),
            external: false,
            forced: false,
            title: Some("Main".to_string()),
        }]),
        subtitle_tracks: Some(vec![SubtitleTrack {
            language: Some("en".to_string()),
            codec: "pgs_image".to_string(),
            default: false,
            forced: false,
            external: true,
            codec_profile: Some("hdmv-pgs".to_string()),
            srt_compatible: false,
            image_count: Some(1_287),
            title: None,
            path: None,
            stream_index: None,
        }]),
        video_codec: Some(VideoCodec::H265),
        video_profile: Some(VideoProfile::HevcMain10),
        video_level: Some(51.0),
        pixel_format: Some(PixelFormat::Yuv420p10le),
        color_primaries: Some(ColorPrimaries::Bt2020),
        transfer_characteristics: Some(HdrTransfer::Pq),
        matrix_coefficients: Some(MatrixCoefficients::Bt2020Ncl),
        dovi_profile: Some(DolbyVisionProfile::Profile7),
        max_cll: Some(1_000),
        max_fall: Some(400),
        mastering_display_primary_r: Some((34_500, 16_000)),
        mastering_display_primary_g: Some((13_500, 34_500)),
        mastering_display_primary_b: Some((7_500, 3_000)),
        mastering_display_white_point: Some((15_600, 16_400)),
        mastering_display_min_luminance: Some(1),
        mastering_display_max_luminance: Some(1_000),
        frame_rate: Some(23.976),
    }
}

#[test]
fn edition_round_trips_through_json() {
    let original = full_edition();
    let json = serde_json::to_string(&original).expect("serialize");
    let back: Edition = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.id, original.id);
    assert_eq!(back.work_id, original.work_id);
    assert_eq!(back.name, original.name);
    assert_eq!(back.runtime_ms, original.runtime_ms);
    assert_eq!(back.codec_video, original.codec_video);
    assert_eq!(back.hdr_format, original.hdr_format);
    assert_eq!(back.video_codec, original.video_codec);
    assert_eq!(back.video_profile, original.video_profile);
    assert_eq!(back.video_level, original.video_level);
    assert_eq!(back.pixel_format, original.pixel_format);
    assert_eq!(back.color_primaries, original.color_primaries);
    assert_eq!(
        back.transfer_characteristics,
        original.transfer_characteristics
    );
    assert_eq!(back.matrix_coefficients, original.matrix_coefficients);
    assert_eq!(back.dovi_profile, original.dovi_profile);
    assert_eq!(back.max_cll, original.max_cll);
    assert_eq!(back.max_fall, original.max_fall);
    assert_eq!(
        back.mastering_display_primary_r,
        original.mastering_display_primary_r
    );
    assert_eq!(
        back.mastering_display_primary_g,
        original.mastering_display_primary_g
    );
    assert_eq!(
        back.mastering_display_primary_b,
        original.mastering_display_primary_b
    );
    assert_eq!(
        back.mastering_display_white_point,
        original.mastering_display_white_point
    );
    assert_eq!(
        back.mastering_display_min_luminance,
        original.mastering_display_min_luminance
    );
    assert_eq!(
        back.mastering_display_max_luminance,
        original.mastering_display_max_luminance
    );
    assert_eq!(back.frame_rate, original.frame_rate);
    let orig_tracks = back.audio_tracks.as_ref().expect("audio tracks");
    assert_eq!(orig_tracks.len(), 1);
    assert_eq!(orig_tracks[0].codec_profile.as_deref(), Some("TrueHD"));
    assert_eq!(orig_tracks[0].audio_object_type.as_deref(), Some("joc"));
    assert_eq!(orig_tracks[0].channel_layout, Some(ChannelLayout::Seven1));
    let orig_subs = back.subtitle_tracks.as_ref().expect("subtitle tracks");
    assert_eq!(orig_subs.len(), 1);
    assert!(orig_subs[0].external);
    assert_eq!(orig_subs[0].image_count, Some(1_287));
}

#[test]
fn edition_round_trips_when_new_fields_are_omitted() {
    // Legacy JSON omits the new fields. `#[serde(default)]` on
    // every one of them must make this deserialize cleanly into
    // `None`.
    let legacy = serde_json::json!({
        "id": "00000000-0000-7000-8000-000000000000",
        "work_id": "00000000-0000-7000-8000-000000000001",
        "name": "Legacy Cut",
        "runtime_ms": 7000000_i64,
        "chapters": [],
    });
    let ed: Edition = serde_json::from_value(legacy).expect("legacy deserialize");
    assert_eq!(ed.name, "Legacy Cut");
    assert_eq!(ed.video_codec, None);
    assert_eq!(ed.video_profile, None);
    assert_eq!(ed.video_level, None);
    assert_eq!(ed.pixel_format, None);
    assert_eq!(ed.color_primaries, None);
    assert_eq!(ed.transfer_characteristics, None);
    assert_eq!(ed.matrix_coefficients, None);
    assert_eq!(ed.dovi_profile, None);
    assert_eq!(ed.max_cll, None);
    assert_eq!(ed.max_fall, None);
    assert_eq!(ed.mastering_display_primary_r, None);
    assert_eq!(ed.mastering_display_white_point, None);
    assert_eq!(ed.mastering_display_min_luminance, None);
    assert_eq!(ed.mastering_display_max_luminance, None);
    assert_eq!(ed.frame_rate, None);
}

#[test]
fn enums_serialize_to_snake_case_and_parse_back() {
    for v in [
        VideoCodec::H264,
        VideoCodec::H265,
        VideoCodec::Av1,
        VideoCodec::Vp9,
        VideoCodec::Vp8,
        VideoCodec::Mpeg2,
        VideoCodec::Prores,
        VideoCodec::Theora,
    ] {
        let s = serde_json::to_string(&v).expect("video codec serialize");
        assert!(s.starts_with('"') && s.ends_with('"'), "got {s}");
        let body = &s[1..s.len() - 1];
        assert_eq!(body, v.as_str());
        let back: VideoCodec = serde_json::from_str(&s).expect("video codec deserialize");
        assert_eq!(back, v);
    }

    for v in [
        VideoProfile::H264Baseline,
        VideoProfile::H264ConstrainedBaseline,
        VideoProfile::H264Main,
        VideoProfile::H264High,
        VideoProfile::H264High10,
        VideoProfile::HevcMain,
        VideoProfile::HevcMain10,
        VideoProfile::HevcHigh,
        VideoProfile::HevcHigh10,
        VideoProfile::Av1Main,
        VideoProfile::Av1High,
        VideoProfile::Av1Professional,
        VideoProfile::Vp9Profile0,
        VideoProfile::Vp9Profile1,
        VideoProfile::Vp9Profile2,
        VideoProfile::Vp9Profile3,
    ] {
        let s = serde_json::to_string(&v).expect("video profile serialize");
        let back: VideoProfile = serde_json::from_str(&s).expect("video profile deserialize");
        assert_eq!(back, v);
    }

    for v in [
        PixelFormat::Yuv420p,
        PixelFormat::Yuv420p10le,
        PixelFormat::Yuv422p,
        PixelFormat::Yuv422p10le,
        PixelFormat::Yuv444p,
        PixelFormat::Yuv444p10le,
        PixelFormat::Gray,
        PixelFormat::Yuv420p12le,
        PixelFormat::Yuv422p12le,
        PixelFormat::Yuv444p12le,
    ] {
        let s = serde_json::to_string(&v).expect("pixel format serialize");
        let back: PixelFormat = serde_json::from_str(&s).expect("pixel format deserialize");
        assert_eq!(back, v);
    }

    for v in [
        HdrTransfer::Sdr,
        HdrTransfer::Pq,
        HdrTransfer::Hlg,
        HdrTransfer::SLog3,
        HdrTransfer::SLog2,
        HdrTransfer::LogC,
        HdrTransfer::Bt2100,
        HdrTransfer::Bt1886,
    ] {
        let s = serde_json::to_string(&v).expect("hdr transfer serialize");
        let back: HdrTransfer = serde_json::from_str(&s).expect("hdr transfer deserialize");
        assert_eq!(back, v);
    }

    for v in [
        ColorPrimaries::Bt709,
        ColorPrimaries::Bt2020,
        ColorPrimaries::Bt601Ntsc,
        ColorPrimaries::Bt601Pal,
        ColorPrimaries::SmpteC,
        ColorPrimaries::DciP3,
        ColorPrimaries::DisplayP3,
        ColorPrimaries::Bt470M,
        ColorPrimaries::Bt470Bg,
        ColorPrimaries::Xyz,
        ColorPrimaries::Film,
    ] {
        let s = serde_json::to_string(&v).expect("color primaries serialize");
        let back: ColorPrimaries = serde_json::from_str(&s).expect("color primaries deserialize");
        assert_eq!(back, v);
    }

    for v in [
        MatrixCoefficients::Bt709,
        MatrixCoefficients::Bt470Bg,
        MatrixCoefficients::Bt601Ntsc,
        MatrixCoefficients::Bt601Pal,
        MatrixCoefficients::Smpte240M,
        MatrixCoefficients::YCgCo,
        MatrixCoefficients::Bt2020Ncl,
        MatrixCoefficients::Bt2020Cl,
        MatrixCoefficients::Smpte2085,
        MatrixCoefficients::ChromaDerivedNc,
        MatrixCoefficients::ChromaDerivedCl,
        MatrixCoefficients::Ictcp,
    ] {
        let s = serde_json::to_string(&v).expect("matrix coefficients serialize");
        let back: MatrixCoefficients =
            serde_json::from_str(&s).expect("matrix coefficients deserialize");
        assert_eq!(back, v);
    }

    for v in [
        DolbyVisionProfile::Profile5,
        DolbyVisionProfile::Profile7,
        DolbyVisionProfile::Profile8,
        DolbyVisionProfile::Profile9,
    ] {
        let s = serde_json::to_string(&v).expect("dovi profile serialize");
        let back: DolbyVisionProfile = serde_json::from_str(&s).expect("dovi profile deserialize");
        assert_eq!(back, v);
        let n: u8 = v.into();
        let round = DolbyVisionProfile::try_from(n).expect("dovi try_from");
        assert_eq!(round, v);
    }

    for v in [
        ChannelLayout::Mono,
        ChannelLayout::Stereo,
        ChannelLayout::Three,
        ChannelLayout::Four,
        ChannelLayout::Five,
        ChannelLayout::Six,
        ChannelLayout::Seven1,
        ChannelLayout::Eight,
        ChannelLayout::ObjectBased,
    ] {
        let s = serde_json::to_string(&v).expect("channel layout serialize");
        let back: ChannelLayout = serde_json::from_str(&s).expect("channel layout deserialize");
        assert_eq!(back, v);
    }

    for v in [
        noflex_core::SubtitleCodec::WebVtt,
        noflex_core::SubtitleCodec::Ttml,
        noflex_core::SubtitleCodec::Srt,
        noflex_core::SubtitleCodec::SubRip,
        noflex_core::SubtitleCodec::Ass,
        noflex_core::SubtitleCodec::Ssa,
        noflex_core::SubtitleCodec::PgsImage,
        noflex_core::SubtitleCodec::DvbSub,
        noflex_core::SubtitleCodec::MovText,
    ] {
        let s = serde_json::to_string(&v).expect("subtitle codec serialize");
        let back: noflex_core::SubtitleCodec =
            serde_json::from_str(&s).expect("subtitle codec deserialize");
        assert_eq!(back, v);
    }
}

#[test]
fn enum_from_str_matches_as_str() {
    assert_eq!(VideoCodec::H264.as_str(), "h264");
    assert_eq!(VideoProfile::HevcMain10.as_str(), "hevc_main10");
    assert_eq!(PixelFormat::Yuv420p.as_str(), "yuv420p");
    assert_eq!(HdrTransfer::Pq.as_str(), "pq");
    assert_eq!(ColorPrimaries::Bt2020.as_str(), "bt2020");
    assert_eq!(MatrixCoefficients::Bt2020Ncl.as_str(), "bt2020_ncl");
    assert_eq!(DolbyVisionProfile::Profile7.as_str(), "profile7");
    assert_eq!(ChannelLayout::Seven1.as_str(), "7.1");
    assert_eq!(noflex_core::SubtitleCodec::PgsImage.as_str(), "pgs_image");

    assert_eq!(VideoCodec::from_str("h265").unwrap(), VideoCodec::H265);
    assert_eq!(
        VideoProfile::from_str("av1_professional").unwrap(),
        VideoProfile::Av1Professional
    );
    assert_eq!(
        PixelFormat::from_str("yuv420p10le").unwrap(),
        PixelFormat::Yuv420p10le
    );
    assert_eq!(HdrTransfer::from_str("hlg").unwrap(), HdrTransfer::Hlg);
    assert_eq!(
        ColorPrimaries::from_str("dci_p3").unwrap(),
        ColorPrimaries::DciP3
    );
    assert_eq!(
        MatrixCoefficients::from_str("chroma_derived_nc").unwrap(),
        MatrixCoefficients::ChromaDerivedNc
    );
    assert_eq!(
        DolbyVisionProfile::from_str("profile8").unwrap(),
        DolbyVisionProfile::Profile8
    );
    assert_eq!(
        ChannelLayout::from_str("object_based").unwrap(),
        ChannelLayout::ObjectBased
    );
    assert_eq!(
        noflex_core::SubtitleCodec::from_str("mov_text").unwrap(),
        noflex_core::SubtitleCodec::MovText
    );
}

#[test]
fn enum_from_str_rejects_garbage() {
    assert!(VideoCodec::from_str("avc1").is_err());
    assert!(VideoProfile::from_str("baseline").is_err());
    assert!(PixelFormat::from_str("rgb24").is_err());
    assert!(HdrTransfer::from_str("gamma2.2").is_err());
    assert!(ColorPrimaries::from_str("p3").is_err());
    assert!(MatrixCoefficients::from_str("bt709").is_ok());
    assert!(MatrixCoefficients::from_str("rgb").is_err());
    assert!(DolbyVisionProfile::from_str("profile6").is_err());
    assert!(ChannelLayout::from_str("2.0").is_err());
    assert!(noflex_core::SubtitleCodec::from_str("idx").is_err());
}

#[test]
fn dolby_vision_profile_u8_round_trip() {
    use std::convert::TryFrom;
    for v in [
        DolbyVisionProfile::Profile5,
        DolbyVisionProfile::Profile7,
        DolbyVisionProfile::Profile8,
        DolbyVisionProfile::Profile9,
    ] {
        let n: u8 = v.into();
        let back = DolbyVisionProfile::try_from(n).expect("try_from");
        assert_eq!(back, v);
    }
    assert!(DolbyVisionProfile::try_from(0_u8).is_err());
    assert!(DolbyVisionProfile::try_from(6_u8).is_err());
    assert!(DolbyVisionProfile::try_from(255_u8).is_err());
}

#[test]
fn audio_track_with_atmos_round_trips() {
    let t = AudioTrack {
        language: Some("en".to_string()),
        codec: "ac-3".to_string(),
        channels: 6,
        default: true,
        bit_depth: Some(16),
        sample_rate: Some(48_000),
        channel_layout: Some(ChannelLayout::ObjectBased),
        codec_profile: Some("E-AC-3 JOC".to_string()),
        audio_object_type: Some("joc".to_string()),
        external: false,
        forced: false,
        title: None,
    };
    let json = serde_json::to_string(&t).expect("audio serialize");
    let back: AudioTrack = serde_json::from_str(&json).expect("audio deserialize");
    assert_eq!(back, t);
}

#[test]
fn subtitle_track_with_pgs_round_trips() {
    let t = SubtitleTrack {
        language: Some("en".to_string()),
        codec: "pgs_image".to_string(),
        default: false,
        forced: false,
        external: true,
        codec_profile: Some("hdmv-pgs".to_string()),
        srt_compatible: false,
        image_count: Some(1_200),
        title: None,
        path: None,
        stream_index: None,
    };
    let json = serde_json::to_string(&t).expect("subtitle serialize");
    let back: SubtitleTrack = serde_json::from_str(&json).expect("subtitle deserialize");
    assert_eq!(back, t);
}
