// SPDX-License-Identifier: AGPL-3.0-only

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The hardware family an [`Encoder`] runs on. The server resolves
/// a single one of these at startup from runtime detection, then
/// `stream` maps it onto a concrete encoder per transcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwBackend {
    Software,
    Vaapi,
    VideoToolbox,
    Mf,
    Nvenc,
    Qsv,
    Amf,
}

/// Which ffmpeg video encoder (`-c:v`) a re-encode runs. Audio encoders
/// are chosen per codec in `ffmpeg_argv::audio_encoder_flag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "encoder", rename_all = "snake_case")]
pub enum Encoder {
    /// Software x264/x265/svt-av1.
    Software264,
    Software265,
    SoftwareAv1,
    /// Linux VAAPI.
    Vaapi264,
    Vaapi265,
    VaapiAv1,
    /// macOS VideoToolbox.
    VideoToolbox264,
    VideoToolbox265,
    /// Windows MediaFoundation.
    Mf264,
    Mf265,
    /// NVIDIA NVENC.
    Nvenc264,
    Nvenc265,
    NvencAv1,
    /// Intel QSV.
    Qsv264,
    Qsv265,
    QsvAv1,
    /// AMD AMF.
    Amf264,
    Amf265,
    AmfAv1,
}

impl Encoder {
    pub fn backend(self) -> HwBackend {
        match self {
            Encoder::Software264 | Encoder::Software265 | Encoder::SoftwareAv1 => {
                HwBackend::Software
            }
            Encoder::Vaapi264 | Encoder::Vaapi265 | Encoder::VaapiAv1 => HwBackend::Vaapi,
            Encoder::VideoToolbox264 | Encoder::VideoToolbox265 => HwBackend::VideoToolbox,
            Encoder::Mf264 | Encoder::Mf265 => HwBackend::Mf,
            Encoder::Nvenc264 | Encoder::Nvenc265 | Encoder::NvencAv1 => HwBackend::Nvenc,
            Encoder::Qsv264 | Encoder::Qsv265 | Encoder::QsvAv1 => HwBackend::Qsv,
            Encoder::Amf264 | Encoder::Amf265 | Encoder::AmfAv1 => HwBackend::Amf,
        }
    }

    pub fn video_flag(self) -> &'static str {
        match self {
            Encoder::Software264 => "libx264",
            Encoder::Software265 => "libx265",
            Encoder::SoftwareAv1 => "libsvtav1",
            Encoder::Vaapi264 => "h264_vaapi",
            Encoder::Vaapi265 => "hevc_vaapi",
            Encoder::VaapiAv1 => "av1_vaapi",
            Encoder::VideoToolbox264 => "h264_videotoolbox",
            Encoder::VideoToolbox265 => "hevc_videotoolbox",
            Encoder::Mf264 => "h264_mf",
            Encoder::Mf265 => "hevc_mf",
            Encoder::Nvenc264 => "h264_nvenc",
            Encoder::Nvenc265 => "hevc_nvenc",
            Encoder::NvencAv1 => "av1_nvenc",
            Encoder::Qsv264 => "h264_qsv",
            Encoder::Qsv265 => "hevc_qsv",
            Encoder::QsvAv1 => "av1_qsv",
            Encoder::Amf264 => "h264_amf",
            Encoder::Amf265 => "hevc_amf",
            Encoder::AmfAv1 => "av1_amf",
        }
    }

    pub fn is_hardware(self) -> bool {
        self.backend() != HwBackend::Software
    }

    /// Extra args the encoder needs that aren't in
    /// `-c:v <video_flag>`. VAAPI needs
    /// `-vaapi_device`; VideoToolbox + QSV don't. The detected
    /// render node wins, falling back to the canonical default.
    pub fn extra_args(self, vaapi_device: Option<&Path>) -> Vec<String> {
        match self {
            Encoder::Vaapi264 | Encoder::Vaapi265 | Encoder::VaapiAv1 => {
                let device = vaapi_device
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "/dev/dri/renderD128".to_string());
                vec!["-vaapi_device".to_string(), device]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_encoders_use_libavcodec_names() {
        assert_eq!(Encoder::Software264.video_flag(), "libx264");
        assert_eq!(Encoder::Software265.video_flag(), "libx265");
        assert_eq!(Encoder::SoftwareAv1.video_flag(), "libsvtav1");
    }

    #[test]
    fn vaapi_carries_device_path() {
        // No explicit device: canonical default.
        let args = Encoder::Vaapi264.extra_args(None);
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-vaapi_device");
        assert_eq!(args[1], "/dev/dri/renderD128");

        // Detected render node overrides the default.
        let args = Encoder::Vaapi265.extra_args(Some(std::path::Path::new("/dev/dri/renderD129")));
        assert_eq!(args[1], "/dev/dri/renderD129");
    }

    #[test]
    fn non_vaapi_encoders_have_no_extra_args() {
        assert!(Encoder::Software264.extra_args(None).is_empty());
        assert!(Encoder::VideoToolbox264.extra_args(None).is_empty());
        assert!(Encoder::Nvenc264.extra_args(None).is_empty());
        assert!(Encoder::Qsv264.extra_args(None).is_empty());
    }

    #[test]
    fn backend_groups_encoders_by_family() {
        assert_eq!(Encoder::Software264.backend(), HwBackend::Software);
        assert_eq!(Encoder::VaapiAv1.backend(), HwBackend::Vaapi);
        assert_eq!(Encoder::VideoToolbox265.backend(), HwBackend::VideoToolbox);
        assert_eq!(Encoder::Mf264.backend(), HwBackend::Mf);
        assert_eq!(Encoder::NvencAv1.backend(), HwBackend::Nvenc);
        assert_eq!(Encoder::Qsv265.backend(), HwBackend::Qsv);
        assert_eq!(Encoder::AmfAv1.backend(), HwBackend::Amf);
    }

    #[test]
    fn hardware_is_everything_but_software() {
        assert!(!Encoder::Software265.is_hardware());
        assert!(Encoder::Vaapi264.is_hardware());
        assert!(Encoder::NvencAv1.is_hardware());
    }
}
