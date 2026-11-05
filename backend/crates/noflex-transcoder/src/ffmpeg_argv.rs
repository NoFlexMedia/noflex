// SPDX-License-Identifier: AGPL-3.0-only

use std::path::{Path, PathBuf};

use noflex_core::{AudioCodec, TonemapCurve, VideoCodec, VideoProfile};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    decision::{AudioPlan, HlsSegmentContainer, TranscodePlan, VideoPlan},
    encoder::{Encoder, HwBackend},
    hls::{HlsVariant, audio_bitrate_bps},
    reasons::TranscodeReasons,
};

/// Input protocols for a network source (tuner, IPTV). `crypto` is
/// AES-128 HLS; the `hls` demuxer's nested opens are checked against the
/// same list, so a playlist can't pivot into `file:`, `concat:`, etc.
pub const NETWORK_PROTOCOL_WHITELIST: &str = "http,https,tcp,tls,crypto";

/// URL schemes a network source may use; anything else is refused at
/// import and again at playback.
pub const NETWORK_INPUT_SCHEMES: &[&str] = &["http", "https"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("network input must use http or https")]
    UnsupportedScheme,
}

/// What ffmpeg reads from. Kept typed so every argv carries the
/// matching `-protocol_whitelist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscodeInput {
    /// A library file; only the `file` protocol is allowed.
    File(PathBuf),
    /// A network stream; construct through [`TranscodeInput::url`].
    Url(String),
}

impl TranscodeInput {
    /// A network input, refused unless its scheme is one of
    /// [`NETWORK_INPUT_SCHEMES`] (case-insensitive).
    pub fn url(raw: impl Into<String>) -> Result<Self, InputError> {
        let raw = raw.into();
        if is_network_url(&raw) {
            Ok(Self::Url(raw))
        } else {
            Err(InputError::UnsupportedScheme)
        }
    }

    pub fn protocol_whitelist(&self) -> &'static str {
        match self {
            TranscodeInput::File(_) => "file",
            TranscodeInput::Url(_) => NETWORK_PROTOCOL_WHITELIST,
        }
    }

    /// The `-i` argument. Paths get an explicit `file:` so a name
    /// containing `:` is never parsed as a protocol prefix.
    pub fn ffmpeg_arg(&self) -> String {
        match self {
            TranscodeInput::File(path) => format!("file:{}", path.display()),
            TranscodeInput::Url(url) => url.clone(),
        }
    }

    pub fn as_path(&self) -> Option<&Path> {
        match self {
            TranscodeInput::File(path) => Some(path),
            TranscodeInput::Url(_) => None,
        }
    }
}

/// Whether `raw` starts with an allowed network scheme.
pub fn is_network_url(raw: &str) -> bool {
    raw.split_once("://").is_some_and(|(scheme, rest)| {
        !rest.is_empty()
            && NETWORK_INPUT_SCHEMES
                .iter()
                .any(|s| s.eq_ignore_ascii_case(scheme))
    })
}

/// One rung of the transcode ladder: what a re-encode in `video_codec` can
/// produce, up to `max_pixels` of picture area at `bitrate_kbps`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LadderEntry {
    pub max_pixels: u64,
    pub video_codec: VideoCodec,
    /// The most capable profile to encode at; a client that decodes less gets
    /// its own best.
    pub video_profile: VideoProfile,
    pub audio_codec: AudioCodec,
    pub container: String,
    pub bitrate_kbps: u32,
}

impl LadderEntry {
    /// `client`, unless this rung holds its codec to something lower.
    pub fn cap_profile(&self, client: VideoProfile) -> VideoProfile {
        // Encodes are 8-bit, so only H.264 has a choice of profile to rank.
        let rank = |profile| match profile {
            VideoProfile::H264Baseline | VideoProfile::H264ConstrainedBaseline => Some(0),
            VideoProfile::H264Main => Some(1),
            VideoProfile::H264High => Some(2),
            VideoProfile::H264High10 => Some(3),
            _ => None,
        };
        match (rank(self.video_profile), rank(client)) {
            (Some(rung), Some(wanted)) if rung < wanted => self.video_profile,
            _ => client,
        }
    }

    /// The rung for a `pixels`-sized encode in `codec`: the smallest that holds
    /// it, else the largest. A ladder with no rung for `codec` lends its others,
    /// and rungs over `max_bitrate_bps` are out. `None` when none are left.
    pub fn pick(
        ladder: &[Self],
        codec: VideoCodec,
        pixels: u64,
        max_bitrate_bps: Option<u64>,
    ) -> Option<&Self> {
        let has_codec = ladder.iter().any(|rung| rung.video_codec == codec);
        let usable = ladder
            .iter()
            .filter(|rung| !has_codec || rung.video_codec == codec)
            .filter(|rung| {
                max_bitrate_bps
                    .is_none_or(|max| u64::from(rung.bitrate_kbps).saturating_mul(1000) <= max)
            });
        usable
            .clone()
            .filter(|rung| rung.max_pixels >= pixels)
            .min_by_key(|rung| rung.max_pixels)
            .or_else(|| usable.max_by_key(|rung| rung.max_pixels))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SegmentConfig {
    pub duration_secs: u32,
    pub playlist_type: PlaylistType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistType {
    /// Grows as the encode goes; ffmpeg writes a VOD playlist only at the end.
    Event,
    /// Sliding window: no playlist type, old segments deleted.
    Live,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            duration_secs: 4,
            playlist_type: PlaylistType::Event,
        }
    }
}

/// Where a burned-in subtitle track comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubtitleBurnSource {
    /// Index among the input's subtitle streams (0-based) — ffmpeg's
    /// `subtitles=:si=` counts subtitle streams only.
    Embedded(usize),
    /// Sidecar file on disk (scanner-discovered).
    External(PathBuf),
}

/// One HLS transcode. Output is always an HLS media playlist plus
/// segments next to it.
#[derive(Debug, Clone)]
pub struct TranscodeRequest {
    pub input: TranscodeInput,
    /// Media playlist path; segments land in its directory.
    pub output: PathBuf,
    pub plan: TranscodePlan,
    pub reasons: TranscodeReasons,
    /// Video encoder for `VideoPlan::Encode`; ignored otherwise.
    pub encoder: Encoder,
    pub ladder: LadderEntry,
    pub segment: SegmentConfig,
    /// Resolved VAAPI render node for `-vaapi_device` / `hwupload`.
    pub vaapi_device: Option<PathBuf>,
    /// How to tone-map, when the plan does.
    pub tonemap: TonemapOptions,
    pub start_secs: Option<f64>,
    /// Bounds the output; also lets progress report a percentage.
    pub duration_secs: Option<f64>,
    /// Index among the source's audio streams; `None` is the first.
    pub audio_stream: Option<usize>,
    pub subtitle_burn: Option<SubtitleBurnSource>,
    /// What the master playlist announces for this job, carried here so
    /// the playlist route renders from the job registry alone.
    pub variant: HlsVariant,
}

impl TranscodeRequest {
    /// A VOD request with every optional knob off.
    pub fn new(
        input: TranscodeInput,
        output: impl Into<PathBuf>,
        plan: TranscodePlan,
        encoder: Encoder,
        ladder: LadderEntry,
    ) -> Self {
        Self {
            input,
            output: output.into(),
            plan,
            reasons: TranscodeReasons::empty(),
            encoder,
            ladder,
            segment: SegmentConfig::default(),
            vaapi_device: None,
            tonemap: TonemapOptions::default(),
            start_secs: None,
            duration_secs: None,
            audio_stream: None,
            subtitle_burn: None,
            variant: HlsVariant {
                bandwidth_bps: 1,
                resolution: None,
                codecs: String::new(),
                supplemental_codecs: None,
                video_range: None,
                path: "index.m3u8".to_string(),
            },
        }
    }
}

/// The argv the ffmpeg subprocess receives, `argv[0] == "ffmpeg"`.
/// Pure over the request. Progress is written as `key=value` blocks
/// to stdout (`-progress pipe:1`).
pub fn build_ffmpeg_argv(req: &TranscodeRequest) -> Vec<String> {
    let mut argv: Vec<String> = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    if matches!(req.plan.video, VideoPlan::Encode { .. }) {
        argv.extend(req.encoder.extra_args(req.vaapi_device.as_deref()));
    }
    if req.tonemaps_on_gpu() {
        argv.extend(GPU_TONEMAP_DEVICE_ARGS.map(String::from));
    }
    if let Some(start) = req.start_secs {
        argv.extend(["-ss".to_string(), format!("{start}")]);
    }
    if let Some(d) = req.duration_secs {
        argv.extend(["-t".to_string(), format!("{d}")]);
    }
    argv.extend([
        "-protocol_whitelist".to_string(),
        req.input.protocol_whitelist().to_string(),
        "-i".to_string(),
        req.input.ffmpeg_arg(),
    ]);

    push_streams(&mut argv, req);
    push_hls(&mut argv, req);
    argv.push(req.output.display().to_string());
    argv
}

fn push_streams(argv: &mut Vec<String>, req: &TranscodeRequest) {
    push_video(argv, req);
    match req.plan.audio {
        AudioPlan::None => argv.push("-an".to_string()),
        AudioPlan::Copy { .. } => {
            argv.extend([
                "-map".to_string(),
                audio_map(req),
                "-c:a".to_string(),
                "copy".to_string(),
            ]);
        }
        AudioPlan::Encode { codec, channels } => {
            argv.extend([
                "-map".to_string(),
                audio_map(req),
                "-c:a".to_string(),
                audio_encoder_flag(codec).to_string(),
                "-ac".to_string(),
                channels.to_string(),
            ]);
            if !matches!(codec, AudioCodec::Flac | AudioCodec::Pcm) {
                argv.extend([
                    "-b:a".to_string(),
                    format!("{}", audio_bitrate_bps(codec, channels)),
                ]);
            }
        }
    }
    // HLS renditions never carry subtitles: text tracks are delivered
    // out of band, image tracks only via burn-in.
    argv.push("-sn".to_string());
}

/// The arguments mapping and encoding `req.plan.video`.
pub(crate) fn push_video(argv: &mut Vec<String>, req: &TranscodeRequest) {
    let fmp4 = req.plan.segment_container == HlsSegmentContainer::Fmp4;
    match req.plan.video {
        VideoPlan::None => argv.push("-vn".to_string()),
        VideoPlan::Copy { codec, .. } => {
            argv.extend(["-map", "0:v:0", "-c:v", "copy"].map(String::from));
            push_hevc_tag(argv, codec, fmp4);
        }
        VideoPlan::Encode {
            codec,
            width,
            height,
            tonemap,
            profile,
        } => {
            argv.extend(["-map", "0:v:0", "-c:v", req.encoder.video_flag()].map(String::from));
            push_hevc_tag(argv, codec, fmp4);
            if let Some(name) = profile_flag(profile, req.encoder.backend()) {
                argv.extend(["-profile:v".to_string(), name.to_string()]);
            }
            let kbps = scaled_bitrate_kbps(&req.ladder, u64::from(width) * u64::from(height));
            argv.extend([
                "-b:v".to_string(),
                format!("{kbps}k"),
                "-maxrate".to_string(),
                format!("{}k", kbps.saturating_mul(3) / 2),
                "-bufsize".to_string(),
                format!("{}k", kbps.saturating_mul(2)),
                // Keyframes on segment boundaries keep segments even
                // and seekable.
                "-force_key_frames".to_string(),
                format!("expr:gte(t,n_forced*{})", req.segment.duration_secs.max(1)),
            ]);
            if tonemap && !req.tonemaps_on_gpu() {
                argv.extend([
                    "-filter_threads".to_string(),
                    CPU_TONEMAP_THREADS.to_string(),
                ]);
            }
            argv.extend(["-vf".to_string(), video_filter(req, width, height, tonemap)]);
        }
    }
}

/// MSE and Safari only accept HEVC in MP4 under the `hvc1` sample entry.
/// Video bitrate for a `pixels`-sized encode on `ladder`'s rung: the rung's
/// rate scaled by area (a 480p encode on a 1080p rung doesn't need 1080p's
/// bits), floored at 15% so small outputs stay clean, and never above the
/// rung's rate.
pub fn scaled_bitrate_kbps(ladder: &LadderEntry, pixels: u64) -> u32 {
    if ladder.max_pixels == 0 || pixels >= ladder.max_pixels {
        return ladder.bitrate_kbps;
    }
    let scale = (pixels as f64 / ladder.max_pixels as f64).max(0.15);
    (f64::from(ladder.bitrate_kbps) * scale).round() as u32
}

/// `-profile:v` value for `profile` on `backend`'s encoder; `None`
/// where the encoder's default is the only option offered.
fn profile_flag(profile: VideoProfile, backend: HwBackend) -> Option<&'static str> {
    Some(match profile {
        VideoProfile::H264High | VideoProfile::H264High10 => "high",
        VideoProfile::H264Main => "main",
        VideoProfile::H264Baseline | VideoProfile::H264ConstrainedBaseline => match backend {
            HwBackend::Vaapi => "constrained_baseline",
            _ => "baseline",
        },
        VideoProfile::HevcMain | VideoProfile::HevcMain10 => "main",
        _ => return None,
    })
}

fn push_hevc_tag(argv: &mut Vec<String>, codec: VideoCodec, fmp4: bool) {
    if fmp4 && codec == VideoCodec::H265 {
        argv.extend(["-tag:v".to_string(), "hvc1".to_string()]);
    }
}

fn audio_map(req: &TranscodeRequest) -> String {
    format!("0:a:{}", req.audio_stream.unwrap_or(0))
}

/// ffmpeg encoder for an audio codec the decision may target.
pub fn audio_encoder_flag(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::Opus => "libopus",
        AudioCodec::Ac3 => "ac3",
        AudioCodec::Eac3 => "eac3",
        AudioCodec::Mp3 => "libmp3lame",
        AudioCodec::Flac => "flac",
        _ => "aac",
    }
}

/// How to tone-map an HDR source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TonemapOptions {
    /// Run on the GPU where the encoder allows it; see
    /// [`crate::hwdetect::gpu_tonemap_works`].
    pub gpu: bool,
    pub curve: TonemapCurve,
}

/// Filter threads for tone-mapping on the CPU. Its passes scale poorly: past
/// this, more threads mostly burn CPU waiting on each other, and ffmpeg's
/// default of one per core costs twice the CPU for about the same speed.
const CPU_TONEMAP_THREADS: &str = "8";

/// HDR to SDR at `width`×`height` on the CPU. Each pass touches every pixel in
/// floating point, so there are as few as possible and they run at the output
/// size: converting and scaling together means a 1080p encode of a 4K source
/// never does this work at 4K.
pub fn cpu_tonemap_filter(width: u32, height: u32, curve: TonemapCurve) -> String {
    format!(
        "zscale=w={width}:h={height}:t=linear:npl=100:p=bt709,format=gbrpf32le,\
         tonemap=tonemap={}:desat=0,zscale=t=bt709:m=bt709:r=tv",
        curve.cpu_name()
    )
}

/// Arguments creating the device [`gpu_tonemap_filter`] runs on.
pub const GPU_TONEMAP_DEVICE_ARGS: [&str; 4] =
    ["-init_hw_device", "vulkan=vk", "-filter_hw_device", "vk"];

/// [`cpu_tonemap_filter`] on the GPU, for a fraction of the CPU time. Peak
/// detection stays off because a seek starts a new encode, and adapting from
/// nothing would visibly shift the brightness each time.
pub fn gpu_tonemap_filter(width: u32, height: u32, curve: TonemapCurve) -> String {
    let curve = curve.gpu_name();
    format!(
        "format=yuv420p10le,hwupload,libplacebo=w={width}:h={height}:tonemapping={curve}:\
         peak_detect=false:colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv:\
         format=yuv420p,hwdownload,format=yuv420p"
    )
}

impl TranscodeRequest {
    /// Whether this request's tone-mapping runs on the GPU. VAAPI and QSV stay
    /// on the CPU path: they bind the filter device to their own, which their
    /// final upload needs.
    pub(crate) fn tonemaps_on_gpu(&self) -> bool {
        self.tonemap.gpu
            && matches!(self.plan.video, VideoPlan::Encode { tonemap: true, .. })
            && !matches!(self.encoder.backend(), HwBackend::Vaapi | HwBackend::Qsv)
    }
}

/// Scale / tonemap / burn-in chain. VAAPI and QSV encode from hardware
/// frames, so the chain ends in an upload; software-side work (tonemap,
/// subtitles) always runs before it. Software and other encoders get an
/// explicit 8-bit 4:2:0 format, or a 10-bit or tonemapped source would
/// make libx264 emit High 10 / 4:4:4 that browsers can't decode.
fn video_filter(req: &TranscodeRequest, width: u32, height: u32, tonemap: bool) -> String {
    let mut chain: Vec<String> = Vec::new();
    let on_gpu = req.tonemaps_on_gpu();
    let width = width & !1;
    if tonemap && !on_gpu {
        chain.push(cpu_tonemap_filter(width, height, req.tonemap.curve));
    }
    let subtitles = req.subtitle_burn.as_ref().map(|source| match source {
        SubtitleBurnSource::Embedded(idx) => {
            let path = req
                .input
                .as_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            format!("subtitles=filename={}:si={idx}", quoted_filter_value(&path))
        }
        SubtitleBurnSource::External(path) => {
            format!(
                "subtitles=filename={}",
                quoted_filter_value(&path.display().to_string())
            )
        }
    });
    let software_scale = tonemap || subtitles.is_some();
    match req.encoder.backend() {
        HwBackend::Vaapi if !software_scale => {
            chain.push(format!("format=nv12,hwupload,scale_vaapi=-2:{height}"));
        }
        HwBackend::Qsv if !software_scale => {
            chain.push(format!(
                "format=nv12,hwupload=extra_hw_frames=64,scale_qsv=-2:{height}"
            ));
        }
        backend => {
            if on_gpu {
                chain.push(gpu_tonemap_filter(width, height, req.tonemap.curve));
            } else if !tonemap {
                chain.push(format!("scale=-2:{height}"));
            }
            if let Some(subtitles) = subtitles {
                // An input seek restarts timestamps at zero, but the
                // subtitles filter matches cues by source time: shift
                // frames back to source time for it, then rebase.
                match req.start_secs.filter(|start| *start > 0.0) {
                    Some(start) => chain.extend([
                        format!("setpts=PTS+{start}/TB"),
                        subtitles,
                        "setpts=PTS-STARTPTS".to_string(),
                    ]),
                    None => chain.push(subtitles),
                }
            }
            chain.push(
                match backend {
                    HwBackend::Vaapi => "format=nv12,hwupload",
                    HwBackend::Qsv => "format=nv12,hwupload=extra_hw_frames=64",
                    _ => "format=yuv420p",
                }
                .to_string(),
            );
        }
    }
    chain.join(",")
}

fn push_hls(argv: &mut Vec<String>, req: &TranscodeRequest) {
    let dir = req.output.parent().unwrap_or_else(|| Path::new("."));
    argv.extend([
        "-f".to_string(),
        "hls".to_string(),
        "-hls_time".to_string(),
        req.segment.duration_secs.to_string(),
    ]);
    match req.segment.playlist_type {
        PlaylistType::Event => argv.extend(["-hls_playlist_type", "event"].map(String::from)),
        PlaylistType::Live => argv.extend(
            [
                "-hls_list_size",
                "10",
                "-hls_flags",
                "delete_segments+independent_segments",
            ]
            .map(String::from),
        ),
    }
    let pattern = match req.plan.segment_container {
        HlsSegmentContainer::Fmp4 => {
            argv.extend(
                [
                    "-hls_segment_type",
                    "fmp4",
                    "-hls_fmp4_init_filename",
                    "init.mp4",
                ]
                .map(String::from),
            );
            "segment_%05d.m4s"
        }
        HlsSegmentContainer::Ts => "segment_%05d.ts",
    };
    argv.extend([
        "-hls_segment_filename".to_string(),
        dir.join(pattern).display().to_string(),
    ]);
}

fn quoted_filter_value(value: &str) -> String {
    // argv reaches ffmpeg directly (no shell), so only ffmpeg's parser matters.
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::VideoRange;

    fn encode_plan(tonemap: bool) -> TranscodePlan {
        TranscodePlan {
            segment_container: HlsSegmentContainer::Ts,
            video: VideoPlan::Encode {
                codec: VideoCodec::H264,
                width: 1920,
                height: 1080,
                tonemap,
                profile: VideoProfile::H264High,
            },
            audio: AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: 2,
            },
        }
    }

    fn req(encoder: Encoder) -> TranscodeRequest {
        TranscodeRequest::new(
            TranscodeInput::File("/data/movies/blade_runner.mkv".into()),
            "/cache/transcode/br/index.m3u8",
            encode_plan(false),
            encoder,
            LadderEntry {
                max_pixels: 2_073_600,
                video_codec: VideoCodec::H264,
                video_profile: VideoProfile::H264High,
                audio_codec: AudioCodec::Aac,
                container: "ts".to_string(),
                bitrate_kbps: 8000,
            },
        )
    }

    fn after<'a>(argv: &'a [String], flag: &str) -> &'a str {
        let i = argv
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} missing: {argv:?}"));
        &argv[i + 1]
    }

    fn vf(argv: &[String]) -> &str {
        after(argv, "-vf")
    }

    #[test]
    fn file_input_is_whitelisted_and_prefixed() {
        let argv = build_ffmpeg_argv(&req(Encoder::Software264));
        let wl = argv
            .iter()
            .position(|a| a == "-protocol_whitelist")
            .unwrap();
        let i = argv.iter().position(|a| a == "-i").unwrap();
        assert!(wl < i, "whitelist must be an input option");
        assert_eq!(argv[wl + 1], "file");
        assert_eq!(argv[i + 1], "file:/data/movies/blade_runner.mkv");
    }

    #[test]
    fn colon_in_filename_stays_a_file() {
        let mut r = req(Encoder::Software264);
        r.input = TranscodeInput::File("concat:/etc/passwd|x.mkv".into());
        let argv = build_ffmpeg_argv(&r);
        assert_eq!(after(&argv, "-i"), "file:concat:/etc/passwd|x.mkv");
        assert_eq!(after(&argv, "-protocol_whitelist"), "file");
    }

    #[test]
    fn url_input_gets_network_whitelist() {
        let mut r = req(Encoder::Software264);
        r.input = TranscodeInput::url("https://tuner.local/ch/7.m3u8").unwrap();
        let argv = build_ffmpeg_argv(&r);
        assert_eq!(
            after(&argv, "-protocol_whitelist"),
            NETWORK_PROTOCOL_WHITELIST
        );
        assert_eq!(after(&argv, "-i"), "https://tuner.local/ch/7.m3u8");
    }

    #[test]
    fn url_input_rejects_other_schemes() {
        for bad in [
            "file:///etc/passwd",
            "concat:a|b",
            "rtmp://x/y",
            "/local/path",
            "http://",
            "HTTPS",
        ] {
            assert_eq!(
                TranscodeInput::url(bad),
                Err(InputError::UnsupportedScheme),
                "{bad}"
            );
        }
        assert!(TranscodeInput::url("HTTP://Example/x").is_ok());
    }

    #[test]
    fn software_encode_args() {
        let argv = build_ffmpeg_argv(&req(Encoder::Software264));
        assert_eq!(after(&argv, "-c:v"), "libx264");
        assert_eq!(after(&argv, "-c:a"), "aac");
        assert_eq!(after(&argv, "-b:v"), "8000k");
        assert_eq!(after(&argv, "-ac"), "2");
        assert_eq!(after(&argv, "-b:a"), "128000");
        assert_eq!(vf(&argv), "scale=-2:1080,format=yuv420p");
        assert_eq!(after(&argv, "-progress"), "pipe:1");
        assert!(argv.contains(&"-nostats".to_string()));
        assert!(argv.contains(&"-sn".to_string()));
        assert_eq!(argv[0], "ffmpeg");
    }

    #[test]
    fn nvenc_and_vaapi_flags() {
        let argv = build_ffmpeg_argv(&req(Encoder::Nvenc264));
        assert_eq!(after(&argv, "-c:v"), "h264_nvenc");
        let argv = build_ffmpeg_argv(&req(Encoder::Vaapi265));
        assert_eq!(after(&argv, "-vaapi_device"), "/dev/dri/renderD128");
        assert_eq!(vf(&argv), "format=nv12,hwupload,scale_vaapi=-2:1080");
        let argv = build_ffmpeg_argv(&req(Encoder::Qsv264));
        assert!(!argv.contains(&"-vaapi_device".to_string()));
        assert_eq!(
            vf(&argv),
            "format=nv12,hwupload=extra_hw_frames=64,scale_qsv=-2:1080"
        );
    }

    #[test]
    fn copy_plan_emits_no_encoder_or_filter() {
        let mut r = req(Encoder::Vaapi264);
        r.plan = TranscodePlan {
            segment_container: HlsSegmentContainer::Fmp4,
            video: VideoPlan::Copy {
                codec: VideoCodec::H265,
                range: VideoRange::Hdr10,
            },
            audio: AudioPlan::Copy {
                codec: AudioCodec::Eac3,
            },
        };
        let argv = build_ffmpeg_argv(&r);
        assert_eq!(after(&argv, "-c:v"), "copy");
        assert_eq!(after(&argv, "-c:a"), "copy");
        assert_eq!(after(&argv, "-tag:v"), "hvc1");
        assert!(!argv.contains(&"-vf".to_string()));
        assert!(!argv.contains(&"-vaapi_device".to_string()));
        assert_eq!(after(&argv, "-hls_segment_type"), "fmp4");
        assert_eq!(after(&argv, "-hls_fmp4_init_filename"), "init.mp4");
        assert_eq!(
            after(&argv, "-hls_segment_filename"),
            "/cache/transcode/br/segment_%05d.m4s"
        );
    }

    #[test]
    fn ts_segments_and_vod_playlist() {
        let argv = build_ffmpeg_argv(&req(Encoder::Software264));
        assert_eq!(after(&argv, "-f"), "hls");
        assert_eq!(after(&argv, "-hls_time"), "4");
        assert_eq!(after(&argv, "-hls_playlist_type"), "event");
        assert_eq!(
            after(&argv, "-hls_segment_filename"),
            "/cache/transcode/br/segment_%05d.ts"
        );
        assert!(!argv.contains(&"-hls_segment_type".to_string()));
        assert_eq!(argv.last().unwrap(), "/cache/transcode/br/index.m3u8");
    }

    #[test]
    fn live_playlist_slides_instead_of_typing() {
        let mut r = req(Encoder::Software264);
        r.segment.playlist_type = PlaylistType::Live;
        let argv = build_ffmpeg_argv(&r);
        assert!(!argv.contains(&"-hls_playlist_type".to_string()));
        assert_eq!(
            after(&argv, "-hls_flags"),
            "delete_segments+independent_segments"
        );
    }

    #[test]
    fn audio_only_and_video_only_plans() {
        let mut r = req(Encoder::Software264);
        r.plan.video = VideoPlan::None;
        r.audio_stream = Some(2);
        let argv = build_ffmpeg_argv(&r);
        assert!(argv.contains(&"-vn".to_string()));
        assert!(!argv.contains(&"0:v:0".to_string()));
        assert_eq!(after(&argv, "-map"), "0:a:2");
        r.plan = encode_plan(false);
        r.plan.audio = AudioPlan::None;
        let argv = build_ffmpeg_argv(&r);
        assert!(argv.contains(&"-an".to_string()));
    }

    #[test]
    fn opus_encode_uses_libopus() {
        let mut r = req(Encoder::Software264);
        r.plan.audio = AudioPlan::Encode {
            codec: AudioCodec::Opus,
            channels: 6,
        };
        let argv = build_ffmpeg_argv(&r);
        assert_eq!(after(&argv, "-c:a"), "libopus");
        assert_eq!(after(&argv, "-b:a"), "288000");
    }

    #[test]
    fn gpu_tonemap_replaces_the_cpu_chain_and_its_scale() {
        let mut r = req(Encoder::Nvenc264);
        r.plan = encode_plan(true);
        r.tonemap.gpu = true;
        let argv = build_ffmpeg_argv(&r);
        assert!(!argv.contains(&"-filter_threads".to_string()));
        let filter = vf(&argv);
        assert!(
            filter.starts_with(
                "format=yuv420p10le,hwupload,libplacebo=w=1920:h=1080:tonemapping=hable:"
            ),
            "{filter}"
        );
        assert!(
            !filter.contains("zscale") && !filter.contains("scale=-2"),
            "{filter}"
        );
        assert!(
            filter.ends_with("hwdownload,format=yuv420p,format=yuv420p"),
            "{filter}"
        );
        // The device has to exist before the input opens.
        let device = argv
            .iter()
            .position(|arg| arg == "-init_hw_device")
            .expect("device");
        assert!(device < argv.iter().position(|arg| arg == "-i").unwrap());
        // An SDR encode has nothing to tone-map, so it wants no GPU device.
        r.plan = encode_plan(false);
        assert!(!build_ffmpeg_argv(&r).contains(&"-init_hw_device".to_string()));
    }

    #[test]
    fn gpu_tonemap_leaves_vaapi_and_qsv_on_the_cpu_path() {
        for encoder in [Encoder::Vaapi264, Encoder::Qsv264] {
            let mut r = req(encoder);
            r.plan = encode_plan(true);
            r.tonemap.gpu = true;
            let argv = build_ffmpeg_argv(&r);
            assert!(vf(&argv).starts_with("zscale=w=1920:h=1080:"));
            assert!(!argv.contains(&"-init_hw_device".to_string()));
        }
    }

    #[test]
    fn tonemap_runs_first_and_vaapi_uploads_last() {
        let mut r = req(Encoder::Vaapi264);
        r.plan = encode_plan(true);
        let argv = build_ffmpeg_argv(&r);
        let chain = vf(&argv);
        assert!(
            chain.starts_with(&cpu_tonemap_filter(1920, 1080, TonemapCurve::Hable)),
            "{chain}"
        );
        assert!(chain.ends_with("r=tv,format=nv12,hwupload"), "{chain}");
        let mut r = req(Encoder::Qsv264);
        r.plan = encode_plan(true);
        let argv = build_ffmpeg_argv(&r);
        assert!(vf(&argv).ends_with("hwupload=extra_hw_frames=64"));
        let mut r = req(Encoder::Software264);
        r.plan = encode_plan(true);
        let argv = build_ffmpeg_argv(&r);
        assert!(vf(&argv).ends_with("r=tv,format=yuv420p"));
    }

    #[test]
    fn the_curve_reaches_the_filter_and_falls_back_where_cpu_lacks_it() {
        let mut r = req(Encoder::Software264);
        r.plan = encode_plan(true);
        r.tonemap.curve = TonemapCurve::Mobius;
        assert!(vf(&build_ffmpeg_argv(&r)).contains("tonemap=tonemap=mobius:"));
        r.tonemap.curve = TonemapCurve::Bt2390;
        assert!(vf(&build_ffmpeg_argv(&r)).contains("tonemap=tonemap=hable:"));
        r.tonemap.gpu = true;
        assert!(vf(&build_ffmpeg_argv(&r)).contains("tonemapping=bt.2390:"));
    }

    #[test]
    fn cpu_tonemap_holds_back_filter_threads() {
        let mut r = req(Encoder::Software264);
        r.plan = encode_plan(true);
        let argv = build_ffmpeg_argv(&r);
        assert_eq!(after(&argv, "-filter_threads"), CPU_TONEMAP_THREADS);
        // Nothing else needs the cap.
        r.plan = encode_plan(false);
        assert!(!build_ffmpeg_argv(&r).contains(&"-filter_threads".to_string()));
    }

    #[test]
    fn burn_in_chains_after_scale_and_before_upload() {
        let mut r = req(Encoder::Vaapi264);
        r.subtitle_burn = Some(SubtitleBurnSource::External("/subs/a\\b's file.srt".into()));
        let argv = build_ffmpeg_argv(&r);
        let chain = vf(&argv);
        assert_eq!(
            chain,
            "scale=-2:1080,subtitles=filename='/subs/a\\\\b\\'s file.srt',format=nv12,hwupload"
        );
        let mut r = req(Encoder::Software264);
        r.subtitle_burn = Some(SubtitleBurnSource::Embedded(2));
        let argv = build_ffmpeg_argv(&r);
        assert!(vf(&argv).contains("subtitles=filename='/data/movies/blade_runner.mkv':si=2"));
    }

    #[test]
    fn encode_carries_the_profile_and_a_resolution_scaled_bitrate() {
        let mut r = req(Encoder::Software264);
        r.plan.video = VideoPlan::Encode {
            codec: VideoCodec::H264,
            width: 854,
            height: 480,
            tonemap: false,
            profile: VideoProfile::H264Main,
        };
        let argv = build_ffmpeg_argv(&r);
        assert_eq!(after(&argv, "-profile:v"), "main");
        // 8000k on the 1080p rung → 854×480 is ≈ 0.198 of its area.
        assert_eq!(after(&argv, "-b:v"), "1581k");
        assert_eq!(after(&argv, "-maxrate"), "2371k");
    }

    #[test]
    fn bitrate_scaling_is_floored_and_capped() {
        let rung = LadderEntry {
            max_pixels: 2_073_600,
            video_codec: VideoCodec::H264,
            video_profile: VideoProfile::H264High,
            audio_codec: AudioCodec::Aac,
            container: "mp4".into(),
            bitrate_kbps: 8000,
        };
        assert_eq!(scaled_bitrate_kbps(&rung, 2_073_600), 8000);
        assert_eq!(scaled_bitrate_kbps(&rung, 8_294_400), 8000);
        assert_eq!(scaled_bitrate_kbps(&rung, 921_600), 3556);
        assert_eq!(scaled_bitrate_kbps(&rung, 102_240), 1200, "15% floor");
    }

    #[test]
    fn baseline_profile_name_follows_the_backend() {
        assert_eq!(
            profile_flag(VideoProfile::H264ConstrainedBaseline, HwBackend::Vaapi),
            Some("constrained_baseline")
        );
        assert_eq!(
            profile_flag(VideoProfile::H264ConstrainedBaseline, HwBackend::Nvenc),
            Some("baseline")
        );
        assert_eq!(
            profile_flag(VideoProfile::Av1Main, HwBackend::Software),
            None
        );
    }

    fn rung(max_pixels: u64, video_codec: VideoCodec, bitrate_kbps: u32) -> LadderEntry {
        LadderEntry {
            max_pixels,
            video_codec,
            video_profile: VideoProfile::H264High,
            audio_codec: AudioCodec::Aac,
            container: "mp4".into(),
            bitrate_kbps,
        }
    }

    #[test]
    fn picks_the_smallest_rung_holding_the_size_in_that_codec() {
        let ladder = [
            rung(2_073_600, VideoCodec::Av1, 3_500),
            rung(8_294_400, VideoCodec::Av1, 12_000),
            rung(921_600, VideoCodec::H264, 4_000),
            rung(2_073_600, VideoCodec::H264, 8_000),
        ];
        let picked = |codec, pixels, limit| {
            LadderEntry::pick(&ladder, codec, pixels, limit).map(|rung| rung.bitrate_kbps)
        };
        // A scope 4K picture is under 4K's area and still takes its rung.
        assert_eq!(picked(VideoCodec::Av1, 6_174_720, None), Some(12_000));
        // Bigger than anything on offer: the largest, for fitting down.
        assert_eq!(picked(VideoCodec::H264, 8_294_400, None), Some(8_000));
        // The limit rules out the rung that would have held it.
        assert_eq!(
            picked(VideoCodec::H264, 2_073_600, Some(5_000_000)),
            Some(4_000)
        );
        // No H.265 rungs: borrow the closest fit from the others.
        assert_eq!(picked(VideoCodec::H265, 921_600, None), Some(4_000));
        assert_eq!(picked(VideoCodec::H264, 921_600, Some(1_000)), None);
    }

    #[test]
    fn a_rung_only_lowers_the_clients_profile() {
        let mut main = rung(2_073_600, VideoCodec::H264, 8_000);
        main.video_profile = VideoProfile::H264Main;
        assert_eq!(
            main.cap_profile(VideoProfile::H264High),
            VideoProfile::H264Main
        );
        assert_eq!(
            main.cap_profile(VideoProfile::H264Baseline),
            VideoProfile::H264Baseline
        );
        assert_eq!(
            main.cap_profile(VideoProfile::Av1Main),
            VideoProfile::Av1Main
        );
    }

    #[test]
    fn seek_and_duration_precede_input() {
        let mut r = req(Encoder::Software264);
        r.start_secs = Some(30.5);
        r.duration_secs = Some(120.0);
        let argv = build_ffmpeg_argv(&r);
        let i = argv.iter().position(|a| a == "-i").unwrap();
        assert!(argv.iter().position(|a| a == "-ss").unwrap() < i);
        assert_eq!(after(&argv, "-ss"), "30.5");
        assert_eq!(after(&argv, "-t"), "120");
    }

    #[test]
    fn seeked_burn_in_matches_cues_by_source_time() {
        let mut r = req(Encoder::Software264);
        r.subtitle_burn = Some(SubtitleBurnSource::External("/s/a.srt".into()));
        r.start_secs = Some(600.0);
        let argv = build_ffmpeg_argv(&r);
        let chain = vf(&argv);
        let shift = chain.find("setpts=PTS+600/TB").expect("shift");
        let subs = chain.find("subtitles=").expect("subtitles");
        let rebase = chain.find("setpts=PTS-STARTPTS").expect("rebase");
        assert!(shift < subs && subs < rebase, "{chain}");
    }
}
