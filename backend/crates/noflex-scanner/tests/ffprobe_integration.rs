// SPDX-License-Identifier: AGPL-3.0-only
//
// W4-T16: ffprobe decoder integration tests.
//
// Each test inlines a real-shape `ffprobe -v error -show_streams
// -show_format -of json` payload as a `serde_json::json!({…})`
// literal (no file I/O, no real ffmpeg) and asserts the typed
// `ProbeReport` decode. The five canonical fixture shapes are SDR H.264 +
// AAC, SDR HEVC + AC-3, SDR AV1 + Opus, HDR10 HEVC Main10, and Dolby Vision
// P8 HEVC. A sixth
// multi-track variant exercises subtitle + multi-audio layouts.

use noflex_core::{
    ChannelLayout, ColorPrimaries, DolbyVisionProfile, HdrTransfer, MatrixCoefficients,
    PixelFormat, SubtitleCodec, VideoCodec, VideoProfile,
};
use noflex_transcoder::probe::ProbeReport;

#[test]
fn h264_aac_mp4_sdr() {
    let v = serde_json::json!({
        "streams": [
            {
                "index": 0,
                "codec_name": "h264",
                "codec_type": "video",
                "profile": "High",
                "level": 40,
                "width": 1920,
                "height": 1080,
                "pix_fmt": "yuv420p",
                "r_frame_rate": "24000/1001",
                "color_primaries": "bt709",
                "color_transfer": "bt709",
                "color_space": "bt709"
            },
            {
                "index": 1,
                "codec_name": "aac",
                "codec_type": "audio",
                "profile": "LC",
                "channels": 2,
                "channel_layout": "stereo",
                "sample_rate": "48000",
                "bits_per_sample": 16,
                "disposition": {"default": 1, "forced": 0},
                "tags": {"language": "eng"}
            }
        ],
        "format": {
            "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
            "bit_rate": "5012378",
            "duration": "7150.500000"
        }
    });
    let r = ProbeReport::from_ffprobe_json(v).expect("decode");
    assert_eq!(r.container.as_deref(), Some("mov"));
    assert_eq!(r.format_bit_rate, Some(5_012_378));
    let video = r.video.as_ref().expect("video");
    assert_eq!(video.codec, VideoCodec::H264);
    assert_eq!(video.profile, Some(VideoProfile::H264High));
    assert_eq!(video.level, Some(40.0));
    assert_eq!(video.width, 1920);
    assert_eq!(video.height, 1080);
    assert_eq!(video.pixel_format, Some(PixelFormat::Yuv420p));
    assert_eq!(video.bit_depth, Some(8));
    assert_eq!(video.frame_rate, Some(23.976025));
    assert_eq!(video.color_space, Some(ColorPrimaries::Bt709));
    assert_eq!(video.transfer, Some(HdrTransfer::Sdr));
    assert_eq!(video.matrix, Some(MatrixCoefficients::Bt709));
    assert!(video.dovi_profile.is_none());
    assert!(video.side_data.mastering.is_none());
    assert!(video.side_data.max_cll.is_none());
    assert!(video.side_data.max_fall.is_none());
    assert_eq!(r.audio.len(), 1);
    let a = &r.audio[0];
    assert_eq!(a.index, 1);
    assert_eq!(a.codec, "aac");
    assert_eq!(a.profile.as_deref(), Some("LC"));
    assert_eq!(a.bit_depth, Some(16));
    assert_eq!(a.sample_rate, Some(48_000));
    assert_eq!(a.channels, 2);
    assert_eq!(a.channel_layout, Some(ChannelLayout::Stereo));
    assert_eq!(a.language.as_deref(), Some("eng"));
    assert!(a.default);
    assert!(!a.forced);
    assert!(r.subtitle.is_empty());
}

#[test]
fn hevc_ac3_mkv_sdr() {
    let v = serde_json::json!({
        "streams": [
            {
                "index": 0,
                "codec_name": "hevc",
                "codec_type": "video",
                "profile": "Main",
                "level": 120,
                "width": 1920,
                "height": 1080,
                "pix_fmt": "yuv420p",
                "r_frame_rate": "24000/1001",
                "color_primaries": "bt709",
                "color_transfer": "bt709",
                "color_space": "bt709"
            },
            {
                "index": 1,
                "codec_name": "ac3",
                "codec_type": "audio",
                "profile": "AC-3",
                "channels": 6,
                "channel_layout": "5.1",
                "sample_rate": "48000",
                "bits_per_sample": 16,
                "disposition": {"default": 1, "forced": 0},
                "tags": {"language": "eng"}
            }
        ],
        "format": {
            "format_name": "matroska,webm",
            "bit_rate": "8123456"
        }
    });
    let r = ProbeReport::from_ffprobe_json(v).expect("decode");
    assert_eq!(r.container.as_deref(), Some("matroska"));
    let video = r.video.as_ref().expect("video");
    assert_eq!(video.codec, VideoCodec::H265);
    assert_eq!(video.profile, Some(VideoProfile::HevcMain));
    assert_eq!(video.level, Some(120.0));
    assert_eq!(video.width, 1920);
    assert_eq!(video.height, 1080);
    assert_eq!(video.pixel_format, Some(PixelFormat::Yuv420p));
    assert_eq!(video.bit_depth, Some(8));
    assert_eq!(video.frame_rate, Some(23.976025));
    assert_eq!(video.transfer, Some(HdrTransfer::Sdr));
    let a = &r.audio[0];
    assert_eq!(a.codec, "ac3");
    assert_eq!(a.profile.as_deref(), Some("AC-3"));
    assert_eq!(a.channels, 6);
    assert_eq!(a.channel_layout, Some(ChannelLayout::Six));
}

#[test]
fn av1_opus_webm_sdr() {
    let v = serde_json::json!({
        "streams": [
            {
                "index": 0,
                "codec_name": "av1",
                "codec_type": "video",
                "profile": "Main",
                "level": 31,
                "width": 1280,
                "height": 720,
                "pix_fmt": "yuv420p",
                "r_frame_rate": "25/1",
                "color_primaries": "bt709",
                "color_transfer": "bt709",
                "color_space": "bt709"
            },
            {
                "index": 1,
                "codec_name": "opus",
                "codec_type": "audio",
                "channels": 2,
                "channel_layout": "stereo",
                "sample_rate": "48000",
                "disposition": {"default": 1, "forced": 0}
            }
        ],
        "format": {
            "format_name": "matroska,webm",
            "bit_rate": "1500000"
        }
    });
    let r = ProbeReport::from_ffprobe_json(v).expect("decode");
    assert_eq!(r.container.as_deref(), Some("matroska"));
    let video = r.video.as_ref().expect("video");
    assert_eq!(video.codec, VideoCodec::Av1);
    assert_eq!(video.profile, Some(VideoProfile::Av1Main));
    assert_eq!(video.level, Some(31.0));
    assert_eq!(video.width, 1280);
    assert_eq!(video.height, 720);
    assert_eq!(video.pixel_format, Some(PixelFormat::Yuv420p));
    assert_eq!(video.frame_rate, Some(25.0));
    assert_eq!(video.transfer, Some(HdrTransfer::Sdr));
    let a = &r.audio[0];
    assert_eq!(a.codec, "opus");
    assert_eq!(a.channels, 2);
    assert_eq!(a.channel_layout, Some(ChannelLayout::Stereo));
}

#[test]
fn hevc_main10_hdr10_mp4() {
    let v = serde_json::json!({
        "streams": [
            {
                "index": 0,
                "codec_name": "hevc",
                "codec_type": "video",
                "profile": "Main 10",
                "level": 183,
                "width": 3840,
                "height": 2160,
                "pix_fmt": "yuv420p10le",
                "r_frame_rate": "24/1",
                "color_primaries": "bt2020",
                "color_transfer": "smpte2084",
                "color_space": "bt2020nc",
                "side_data_list": [
                    {
                        "side_data_type": "Mastering display metadata",
                        "red_x": "0.6800",
                        "red_y": "0.3200",
                        "green_x": "0.2650",
                        "green_y": "0.6900",
                        "blue_x": "0.1500",
                        "blue_y": "0.0600",
                        "white_point_x": "0.3127",
                        "white_point_y": "0.3290",
                        "min_luminance": "0.0050",
                        "max_luminance": "1000",
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
            },
            {
                "index": 1,
                "codec_name": "eac3",
                "codec_type": "audio",
                "profile": "E-AC-3 JOC",
                "channels": 8,
                "channel_layout": "7.1",
                "sample_rate": "48000",
                "disposition": {"default": 1, "forced": 0},
                "tags": {
                    "language": "eng",
                    "title": "English (Atmos)",
                    "audio_object_type": "joc"
                }
            }
        ],
        "format": {
            "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
            "bit_rate": "54000000"
        }
    });
    let r = ProbeReport::from_ffprobe_json(v).expect("decode");
    let video = r.video.as_ref().expect("video");
    assert_eq!(video.codec, VideoCodec::H265);
    assert_eq!(video.profile, Some(VideoProfile::HevcMain10));
    assert_eq!(video.level, Some(183.0));
    assert_eq!(video.width, 3840);
    assert_eq!(video.height, 2160);
    assert_eq!(video.pixel_format, Some(PixelFormat::Yuv420p10le));
    assert_eq!(video.bit_depth, Some(10));
    assert_eq!(video.frame_rate, Some(24.0));
    assert_eq!(video.color_space, Some(ColorPrimaries::Bt2020));
    assert_eq!(video.transfer, Some(HdrTransfer::Pq));
    assert_eq!(video.matrix, Some(MatrixCoefficients::Bt2020Ncl));
    assert!(video.dovi_profile.is_none());
    assert_eq!(video.side_data.max_cll, Some(1000));
    assert_eq!(video.side_data.max_fall, Some(400));
    let m = video.side_data.mastering.as_ref().expect("mastering");
    assert_eq!(m.primary_r, (34000, 16000));
    assert_eq!(m.primary_g, (13250, 34500));
    assert_eq!(m.primary_b, (7500, 3000));
    assert_eq!(m.white_point, (15635, 16450));
    assert_eq!(m.min_luminance, 50);
    assert_eq!(m.max_luminance, 1000);
    let a = &r.audio[0];
    assert_eq!(a.codec, "eac3");
    assert_eq!(a.profile.as_deref(), Some("E-AC-3 JOC"));
    assert_eq!(a.channels, 8);
    assert_eq!(a.channel_layout, Some(ChannelLayout::Seven1));
    assert_eq!(a.audio_object_type.as_deref(), Some("joc"));
    assert_eq!(a.title.as_deref(), Some("English (Atmos)"));
}

#[test]
fn hevc_dolby_vision_profile_8_mp4() {
    let v = serde_json::json!({
        "streams": [
            {
                "index": 0,
                "codec_name": "hevc",
                "codec_type": "video",
                "profile": "Main 10",
                "level": 153,
                "width": 1920,
                "height": 1080,
                "pix_fmt": "yuv420p10le",
                "r_frame_rate": "24/1",
                "color_primaries": "bt2020",
                "color_transfer": "smpte2084",
                "color_space": "bt2020nc",
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
        "format": {
            "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
            "bit_rate": "18000000"
        }
    });
    let r = ProbeReport::from_ffprobe_json(v).expect("decode");
    let video = r.video.as_ref().expect("video");
    assert_eq!(video.codec, VideoCodec::H265);
    assert_eq!(video.profile, Some(VideoProfile::HevcMain10));
    assert_eq!(video.pixel_format, Some(PixelFormat::Yuv420p10le));
    assert_eq!(video.bit_depth, Some(10));
    assert_eq!(video.color_space, Some(ColorPrimaries::Bt2020));
    assert_eq!(video.transfer, Some(HdrTransfer::Pq));
    assert_eq!(video.matrix, Some(MatrixCoefficients::Bt2020Ncl));
    assert_eq!(video.dovi_profile, Some(DolbyVisionProfile::Profile8));
    assert!(video.side_data.mastering.is_none());
    assert!(video.side_data.max_cll.is_none());
    assert!(r.audio.is_empty());
}

#[test]
fn multi_audio_with_pgs_subtitles() {
    let v = serde_json::json!({
        "streams": [
            {
                "index": 0,
                "codec_name": "h264",
                "codec_type": "video",
                "profile": "High",
                "width": 1920,
                "height": 1080,
                "pix_fmt": "yuv420p"
            },
            {
                "index": 1,
                "codec_name": "aac",
                "codec_type": "audio",
                "channels": 2,
                "channel_layout": "stereo",
                "disposition": {"default": 1, "forced": 0},
                "tags": {"language": "eng"}
            },
            {
                "index": 2,
                "codec_name": "aac",
                "codec_type": "audio",
                "channels": 2,
                "channel_layout": "stereo",
                "disposition": {"default": 0, "forced": 0},
                "tags": {"language": "jpn"}
            },
            {
                "index": 3,
                "codec_name": "hdmv_pgs_subtitle",
                "codec_type": "subtitle",
                "disposition": {"default": 0, "forced": 0},
                "tags": {"language": "eng"}
            },
            {
                "index": 4,
                "codec_name": "hdmv_pgs_subtitle",
                "codec_type": "subtitle",
                "disposition": {"default": 0, "forced": 1},
                "tags": {"language": "jpn", "title": "Forced"}
            }
        ],
        "format": {
            "format_name": "matroska,webm"
        }
    });
    let r = ProbeReport::from_ffprobe_json(v).expect("decode");
    assert_eq!(r.container.as_deref(), Some("matroska"));
    assert_eq!(r.audio.len(), 2);
    assert_eq!(r.audio[0].language.as_deref(), Some("eng"));
    assert!(r.audio[0].default);
    assert_eq!(r.audio[1].language.as_deref(), Some("jpn"));
    assert!(!r.audio[1].default);
    assert_eq!(r.subtitle.len(), 2);
    assert_eq!(r.subtitle[0].codec, SubtitleCodec::PgsImage);
    assert_eq!(r.subtitle[0].index, 3);
    assert!(!r.subtitle[0].default);
    assert!(!r.subtitle[0].forced);
    assert_eq!(r.subtitle[1].codec, SubtitleCodec::PgsImage);
    assert_eq!(r.subtitle[1].index, 4);
    assert!(r.subtitle[1].forced);
    assert_eq!(r.subtitle[1].title.as_deref(), Some("Forced"));
}
