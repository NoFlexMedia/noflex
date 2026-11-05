// SPDX-License-Identifier: AGPL-3.0-only
//
// Round-trip tests for the typed media enums, so a serde rename regression is
// caught before the conformance suite ever sees malformed JSON.

use std::str::FromStr;

use noflex_core::{
    ChannelLayout, ColorPrimaries, DolbyVisionProfile, HdrTransfer, MatrixCoefficients,
    PixelFormat, VideoCodec, VideoProfile,
};
use noflex_storage::domain::SubtitleTrack;
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
