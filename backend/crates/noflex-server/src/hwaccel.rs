// SPDX-License-Identifier: AGPL-3.0-only

//! Resolve a configured [`HwAccel`] + [`TranscodeFallback`] against a
//! runtime [`HwDetect`] snapshot into a single concrete
//! [`HwBackend`](noflex_transcoder::HwBackend). Runs once at startup;
//! the result threads into every transcode request.

use std::path::Path;

use noflex_core::VideoCodec;
use noflex_transcoder::{Encoder, HwDetect, encoder::HwBackend, hwdetect::encoder_works};

use crate::config::{HwAccel, TranscodeFallback};

/// Which of `codecs` this server can encode on `backend`: built into ffmpeg
/// and, for hardware, proven by encoding a frame. Keeps the given order.
pub async fn usable_codecs(
    ffmpeg: &Path,
    backend: HwBackend,
    det: &HwDetect,
    codecs: &[VideoCodec],
) -> Vec<VideoCodec> {
    let mut usable = Vec::new();
    for &codec in codecs {
        let encoder = pick_encoder(backend, codec);
        let built = det.encoders.iter().any(|name| name == encoder.video_flag());
        if built
            && (!encoder.is_hardware()
                || encoder_works(ffmpeg, encoder, det.vaapi_device.as_deref()).await)
        {
            usable.push(codec);
        }
    }
    usable
}

/// Live transcodes' codecs, preferred first: `usable` ones hardware can keep up
/// with, then H.264 as the one software encoder fast enough to choose, then the
/// slow software encoders for clients that play nothing else. H.264 is always
/// present, as the universal fallback.
pub fn live_codec_order(backend: HwBackend, usable: &[VideoCodec]) -> Vec<VideoCodec> {
    let (hardware, software): (Vec<_>, Vec<_>) = usable
        .iter()
        .copied()
        .partition(|codec| pick_encoder(backend, *codec).is_hardware());
    let mut order = hardware;
    if !order.contains(&VideoCodec::H264) {
        order.push(VideoCodec::H264);
    }
    order.extend(
        software
            .into_iter()
            .filter(|codec| *codec != VideoCodec::H264),
    );
    order
}

/// Map the configured preference onto a concrete backend the runtime
/// can actually use. `fallback` says what to fall *back to* when an
/// explicitly requested backend isn't available at runtime (not what
/// to fall back from).
pub fn resolve_backend(cfg: HwAccel, fallback: TranscodeFallback, det: &HwDetect) -> HwBackend {
    let Some(wanted) = configured_backend(cfg) else {
        // None forces software; Auto picks the best available.
        return match cfg {
            HwAccel::None => HwBackend::Software,
            _ => best_available(det).unwrap_or(HwBackend::Software),
        };
    };
    if det.available(wanted) {
        wanted
    } else {
        match fallback {
            TranscodeFallback::Auto => best_available(det).unwrap_or(HwBackend::Software),
            TranscodeFallback::Software => HwBackend::Software,
        }
    }
}

/// The backend an explicit `HwAccel` setting asks for. `None` for
/// `Auto`/`None`, which are resolved by [`resolve_backend`] rather
/// than mapped directly.
pub fn configured_backend(cfg: HwAccel) -> Option<HwBackend> {
    match cfg {
        HwAccel::None | HwAccel::Auto => None,
        HwAccel::Vaapi => Some(HwBackend::Vaapi),
        HwAccel::VideoToolbox => Some(HwBackend::VideoToolbox),
        HwAccel::Mf => Some(HwBackend::Mf),
        HwAccel::Nvenc => Some(HwBackend::Nvenc),
        HwAccel::Qsv => Some(HwBackend::Qsv),
        HwAccel::Amf => Some(HwBackend::Amf),
    }
}

/// Preference order when more than one backend works: the vendor APIs
/// (NVENC, QSV, AMF) first, then VAAPI, then the platform APIs.
fn best_available(det: &HwDetect) -> Option<HwBackend> {
    [
        HwBackend::Nvenc,
        HwBackend::Qsv,
        HwBackend::Amf,
        HwBackend::Vaapi,
        HwBackend::VideoToolbox,
        HwBackend::Mf,
    ]
    .into_iter()
    .find(|b| det.available(*b))
}

/// Lowercase wire name of a backend, as config and diagnostics spell it.
pub fn backend_name(backend: HwBackend) -> &'static str {
    match backend {
        HwBackend::Software => "software",
        HwBackend::Vaapi => "vaapi",
        HwBackend::VideoToolbox => "videotoolbox",
        HwBackend::Mf => "mf",
        HwBackend::Nvenc => "nvenc",
        HwBackend::Qsv => "qsv",
        HwBackend::Amf => "amf",
    }
}

/// Every video encoder the transcoder can drive, for capability reports.
pub const ALL_VIDEO_ENCODERS: [Encoder; 19] = [
    Encoder::Software264,
    Encoder::Software265,
    Encoder::SoftwareAv1,
    Encoder::Vaapi264,
    Encoder::Vaapi265,
    Encoder::VaapiAv1,
    Encoder::VideoToolbox264,
    Encoder::VideoToolbox265,
    Encoder::Mf264,
    Encoder::Mf265,
    Encoder::Nvenc264,
    Encoder::Nvenc265,
    Encoder::NvencAv1,
    Encoder::Qsv264,
    Encoder::Qsv265,
    Encoder::QsvAv1,
    Encoder::Amf264,
    Encoder::Amf265,
    Encoder::AmfAv1,
];

/// Map the resolved backend + ladder codec onto an `Encoder`.
/// VideoToolbox and MediaFoundation have no AV1 encoder, so AV1 falls
/// back to software there; codecs outside h264/hevc/av1 have no
/// hardware mapping and always encode as software h264. Must agree
/// with the stream runtime's selection, which it is meant to replace.
pub fn pick_encoder(backend: HwBackend, codec: VideoCodec) -> Encoder {
    let software = match codec {
        VideoCodec::H265 => Encoder::Software265,
        VideoCodec::Av1 => Encoder::SoftwareAv1,
        _ => Encoder::Software264,
    };
    match (backend, codec) {
        (HwBackend::Vaapi, VideoCodec::H264) => Encoder::Vaapi264,
        (HwBackend::Vaapi, VideoCodec::H265) => Encoder::Vaapi265,
        (HwBackend::Vaapi, VideoCodec::Av1) => Encoder::VaapiAv1,
        (HwBackend::VideoToolbox, VideoCodec::H264) => Encoder::VideoToolbox264,
        (HwBackend::VideoToolbox, VideoCodec::H265) => Encoder::VideoToolbox265,
        (HwBackend::Mf, VideoCodec::H264) => Encoder::Mf264,
        (HwBackend::Mf, VideoCodec::H265) => Encoder::Mf265,
        (HwBackend::Nvenc, VideoCodec::H264) => Encoder::Nvenc264,
        (HwBackend::Nvenc, VideoCodec::H265) => Encoder::Nvenc265,
        (HwBackend::Nvenc, VideoCodec::Av1) => Encoder::NvencAv1,
        (HwBackend::Qsv, VideoCodec::H264) => Encoder::Qsv264,
        (HwBackend::Qsv, VideoCodec::H265) => Encoder::Qsv265,
        (HwBackend::Qsv, VideoCodec::Av1) => Encoder::QsvAv1,
        (HwBackend::Amf, VideoCodec::H264) => Encoder::Amf264,
        (HwBackend::Amf, VideoCodec::H265) => Encoder::Amf265,
        (HwBackend::Amf, VideoCodec::Av1) => Encoder::AmfAv1,
        _ => software,
    }
}

/// Software audio encoders worth reporting: the ladder's AAC plus the
/// common alternatives.
pub const AUDIO_ENCODERS: [&str; 6] = ["aac", "libopus", "libmp3lame", "ac3", "eac3", "flac"];

/// `(ffmpeg decoder, backend label, is_video)` for common source codecs.
/// VAAPI/VideoToolbox decode goes through `-hwaccel`, not named
/// decoders, so only CUDA (cuvid) and QSV variants appear here.
pub const DECODERS: [(&str, &str, bool); 22] = [
    ("h264", "software", true),
    ("hevc", "software", true),
    ("av1", "software", true),
    ("libdav1d", "software", true),
    ("vp9", "software", true),
    ("mpeg2video", "software", true),
    ("vc1", "software", true),
    ("h264_cuvid", "cuda", true),
    ("hevc_cuvid", "cuda", true),
    ("av1_cuvid", "cuda", true),
    ("vp9_cuvid", "cuda", true),
    ("h264_qsv", "qsv", true),
    ("hevc_qsv", "qsv", true),
    ("av1_qsv", "qsv", true),
    ("vp9_qsv", "qsv", true),
    ("aac", "software", false),
    ("ac3", "software", false),
    ("eac3", "software", false),
    ("dca", "software", false),
    ("truehd", "software", false),
    ("flac", "software", false),
    ("opus", "software", false),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// An `HwDetect` on which exactly `backends` encoded a frame.
    fn det_with(backends: &[HwBackend]) -> HwDetect {
        HwDetect {
            working: backends.to_vec(),
            ..HwDetect::default()
        }
    }

    #[test]
    fn none_always_software() {
        let det = det_with(&[HwBackend::Nvenc]);
        assert_eq!(
            resolve_backend(HwAccel::None, TranscodeFallback::Auto, &det),
            HwBackend::Software
        );
    }

    #[test]
    fn auto_picks_best_available() {
        let det = det_with(&[HwBackend::Vaapi, HwBackend::Qsv]);
        // QSV is preferred over VAAPI.
        assert_eq!(
            resolve_backend(HwAccel::Auto, TranscodeFallback::Auto, &det),
            HwBackend::Qsv
        );
    }

    #[test]
    fn auto_falls_to_software_when_nothing_detected() {
        let det = HwDetect::default();
        assert_eq!(
            resolve_backend(HwAccel::Auto, TranscodeFallback::Auto, &det),
            HwBackend::Software
        );
    }

    #[test]
    fn explicit_available_backend_is_used() {
        let det = det_with(&[HwBackend::Vaapi]);
        assert_eq!(
            resolve_backend(HwAccel::Vaapi, TranscodeFallback::Software, &det),
            HwBackend::Vaapi
        );
    }

    #[test]
    fn explicit_unavailable_fallback_software() {
        let det = HwDetect::default();
        assert_eq!(
            resolve_backend(HwAccel::Vaapi, TranscodeFallback::Software, &det),
            HwBackend::Software
        );
    }

    #[test]
    fn explicit_unavailable_fallback_auto_picks_best() {
        // Configured NVENC but only VAAPI present.
        let det = det_with(&[HwBackend::Vaapi]);
        assert_eq!(
            resolve_backend(HwAccel::Nvenc, TranscodeFallback::Auto, &det),
            HwBackend::Vaapi
        );
    }

    #[test]
    fn best_available_prefers_vendor_apis_then_vaapi() {
        let det = det_with(&[
            HwBackend::Vaapi,
            HwBackend::Amf,
            HwBackend::Qsv,
            HwBackend::Nvenc,
            HwBackend::VideoToolbox,
            HwBackend::Mf,
        ]);
        assert_eq!(best_available(&det), Some(HwBackend::Nvenc));

        // Drop NVENC: QSV wins.
        let det = det_with(&[HwBackend::Vaapi, HwBackend::Amf, HwBackend::Qsv]);
        assert_eq!(best_available(&det), Some(HwBackend::Qsv));

        // An AMD card on Windows: AMF over Media Foundation.
        let det = det_with(&[HwBackend::Mf, HwBackend::Amf]);
        assert_eq!(best_available(&det), Some(HwBackend::Amf));

        // Drop QSV: VAAPI wins.
        let det = det_with(&[HwBackend::Vaapi]);
        assert_eq!(best_available(&det), Some(HwBackend::Vaapi));
    }

    #[test]
    fn pick_encoder_maps_hardware_and_falls_back_to_software() {
        assert_eq!(
            pick_encoder(HwBackend::Vaapi, VideoCodec::H265).video_flag(),
            "hevc_vaapi"
        );
        assert_eq!(
            pick_encoder(HwBackend::VideoToolbox, VideoCodec::Av1),
            Encoder::SoftwareAv1
        );
        assert_eq!(
            pick_encoder(HwBackend::Nvenc, VideoCodec::Vp9),
            Encoder::Software264
        );
        assert_eq!(
            pick_encoder(HwBackend::Software, VideoCodec::H265),
            Encoder::Software265
        );
    }

    #[test]
    fn encoder_catalog_has_no_duplicates() {
        let mut flags: Vec<_> = ALL_VIDEO_ENCODERS.iter().map(|e| e.video_flag()).collect();
        flags.sort_unstable();
        flags.dedup();
        assert_eq!(flags.len(), ALL_VIDEO_ENCODERS.len());
    }

    #[test]
    fn best_available_none_when_empty() {
        assert_eq!(best_available(&HwDetect::default()), None);
    }
}
