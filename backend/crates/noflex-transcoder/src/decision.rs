// SPDX-License-Identifier: AGPL-3.0-only
//
// Direct-play / HLS decision. Capabilities are set-based, modelled on
// Jellyfin's DeviceProfile but trimmed to what a browser can actually
// report: per-delivery container/codec allow-lists plus per-codec
// decoder limits. Clients probe these themselves (the web client in
// `web/src/lib/codecSupport.ts`); the server never guesses from a user
// agent.

use async_graphql::{Enum, InputObject};
use noflex_core::{
    AudioCodec, DolbyVisionProfile, HdrTransfer, PixelFormat, SubtitleCodec, VideoCodec,
    VideoProfile,
};
use serde::{Deserialize, Serialize};

use crate::{
    probe::{AudioStreamProbe, ProbeReport, VideoStreamProbe, map_audio_codec},
    reasons::TranscodeReasons,
};

/// Dynamic-range type of a video stream; mirrors Jellyfin's
/// `VideoRangeType`. The `DoviWith*` variants are Dolby Vision streams
/// whose base layer is independently presentable as the named range
/// (profile 8.x, 7, 9), so a client without Dolby Vision can still
/// play the base layer untouched.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VideoRange {
    Sdr,
    #[graphql(name = "HDR10")]
    Hdr10,
    #[graphql(name = "HDR10_PLUS")]
    Hdr10Plus,
    Hlg,
    Dovi,
    DoviWithSdr,
    #[graphql(name = "DOVI_WITH_HDR10")]
    DoviWithHdr10,
    DoviWithHlg,
}

impl VideoRange {
    /// Classify a probed stream. HDR10+ dynamic metadata isn't probed,
    /// so PQ streams report as `Hdr10`.
    pub fn of(video: &VideoStreamProbe) -> Self {
        let transfer = video.transfer.unwrap_or(HdrTransfer::Sdr);
        match video.dovi_profile {
            Some(DolbyVisionProfile::Profile5) => VideoRange::Dovi,
            Some(_) => match transfer {
                HdrTransfer::Pq => VideoRange::DoviWithHdr10,
                HdrTransfer::Hlg => VideoRange::DoviWithHlg,
                _ => VideoRange::DoviWithSdr,
            },
            None => match transfer {
                HdrTransfer::Pq => VideoRange::Hdr10,
                HdrTransfer::Hlg => VideoRange::Hlg,
                _ => VideoRange::Sdr,
            },
        }
    }

    /// Whether presenting this range on an SDR pipeline needs a tonemap.
    pub fn is_hdr(self) -> bool {
        !matches!(self, VideoRange::Sdr | VideoRange::DoviWithSdr)
    }

    /// The range the client actually presents when handed this stream
    /// untouched, given the ranges it declared (SDR is always implied).
    /// Dolby Vision streams fall back to their base layer; profile 5
    /// has no presentable base layer. `None` means the stream can't be
    /// shown as-is.
    pub fn delivered(self, supported: &[VideoRange]) -> Option<VideoRange> {
        let has = |r: VideoRange| r == VideoRange::Sdr || supported.contains(&r);
        let fallback = match self {
            VideoRange::Hdr10Plus => Some(VideoRange::Hdr10),
            VideoRange::DoviWithSdr => Some(VideoRange::Sdr),
            VideoRange::DoviWithHdr10 => Some(VideoRange::Hdr10),
            VideoRange::DoviWithHlg => Some(VideoRange::Hlg),
            _ => None,
        };
        if has(self) {
            Some(self)
        } else {
            fallback.filter(|r| has(*r))
        }
    }
}

/// Segment format of an HLS rendition. fMP4 is what MSE consumes
/// natively and the only one that carries HEVC/AV1/VP9; MPEG-TS stays
/// for clients (and audio codecs) that need it.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HlsSegmentContainer {
    #[graphql(name = "FMP4")]
    Fmp4,
    Ts,
}

impl HlsSegmentContainer {
    /// Whether ffmpeg's segment muxer can carry `codec` stream-copied
    /// or encoded. Clients may claim more; the server never announces
    /// what it can't produce.
    pub fn carries_video(self, codec: VideoCodec) -> bool {
        match self {
            HlsSegmentContainer::Fmp4 => matches!(
                codec,
                VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1 | VideoCodec::Vp9
            ),
            HlsSegmentContainer::Ts => matches!(codec, VideoCodec::H264 | VideoCodec::H265),
        }
    }

    pub fn carries_audio(self, codec: AudioCodec) -> bool {
        match self {
            HlsSegmentContainer::Fmp4 => matches!(
                codec,
                AudioCodec::Aac
                    | AudioCodec::Ac3
                    | AudioCodec::Eac3
                    | AudioCodec::Mp3
                    | AudioCodec::Opus
                    | AudioCodec::Flac
            ),
            HlsSegmentContainer::Ts => matches!(
                codec,
                AudioCodec::Aac
                    | AudioCodec::Ac3
                    | AudioCodec::Eac3
                    | AudioCodec::Mp3
                    | AudioCodec::Opus
            ),
        }
    }

    /// The name the target-stream summary reports.
    pub fn as_str(self) -> &'static str {
        match self {
            HlsSegmentContainer::Fmp4 => "fmp4",
            HlsSegmentContainer::Ts => "mpegts",
        }
    }
}

/// Progressive (`<video src>`) playback of the file as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, InputObject)]
pub struct DirectPlayProfile {
    /// Container family: `mp4` matches every ISO-BMFF file (ffprobe
    /// calls them all `mov`), `mkv`/`webm`/`matroska` match Matroska,
    /// `ts` matches MPEG-TS; anything else compares as ffprobe names it.
    pub container: String,
    pub video_codecs: Vec<VideoCodec>,
    pub audio_codecs: Vec<AudioCodec>,
}

/// What the client's HLS player accepts in one segment format, for
/// stream-copied and encoded streams alike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, InputObject)]
pub struct HlsProfile {
    pub segment_container: HlsSegmentContainer,
    pub video_codecs: Vec<VideoCodec>,
    pub audio_codecs: Vec<AudioCodec>,
}

/// Decoder limits for one video codec, applied wherever the codec is
/// listed. A codec with no entry is unconstrained except for range,
/// which defaults to SDR only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, InputObject)]
pub struct VideoCodecSupport {
    pub codec: VideoCodec,
    /// Decodable profiles. Empty accepts any; a source whose profile
    /// ffprobe couldn't name is accepted.
    pub profiles: Vec<VideoProfile>,
    /// In ffprobe's `level` units: H.264 level×10, HEVC level×30
    /// (`general_level_idc`), AV1 `seq_level_idx`, VP9 level×10.
    pub max_level: Option<f32>,
    pub max_bit_depth: Option<u8>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    /// Ranges decoder and display can present; SDR is implied.
    pub video_ranges: Vec<VideoRange>,
}

/// Decoder limits for one audio codec. Without an entry, any channel
/// count is accepted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, InputObject)]
pub struct AudioCodecSupport {
    pub codec: AudioCodec,
    pub max_channels: Option<u8>,
}

/// What a playback client can play, as the client itself measured it.
/// The canonical `startPlaybackSession` input type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, InputObject)]
pub struct ClientCapabilities {
    /// Progressive profiles, tried in order.
    pub direct_play: Vec<DirectPlayProfile>,
    /// HLS segment formats, in preference order; ties go to the earlier.
    pub hls: Vec<HlsProfile>,
    pub video_codecs: Vec<VideoCodecSupport>,
    pub audio_codecs: Vec<AudioCodecSupport>,
    /// Text subtitle formats the client renders itself. Sidecar tracks
    /// convertible to one of these are delivered out of band rather
    /// than forcing a transcode.
    pub subtitle_codecs: Vec<SubtitleCodec>,
    /// Display ceiling: taller sources are scaled down.
    pub max_height: Option<u32>,
    /// The restrictions policy clamps this server-side.
    pub max_bitrate_bps: Option<u64>,
}

impl ClientCapabilities {
    /// A conservative browser: H.264/AAC in MP4 and fMP4/TS HLS,
    /// VP8/VP9/AV1 + Opus/Vorbis in WebM, SDR only, WebVTT subtitles.
    pub fn web() -> Self {
        let mp4_audio = vec![AudioCodec::Aac, AudioCodec::Mp3];
        Self {
            direct_play: vec![
                DirectPlayProfile {
                    container: "mp4".to_string(),
                    video_codecs: vec![VideoCodec::H264],
                    audio_codecs: mp4_audio.clone(),
                },
                DirectPlayProfile {
                    container: "webm".to_string(),
                    video_codecs: vec![VideoCodec::Vp8, VideoCodec::Vp9, VideoCodec::Av1],
                    audio_codecs: vec![AudioCodec::Opus, AudioCodec::Vorbis],
                },
            ],
            hls: vec![
                HlsProfile {
                    segment_container: HlsSegmentContainer::Fmp4,
                    video_codecs: vec![VideoCodec::H264],
                    audio_codecs: vec![AudioCodec::Aac],
                },
                HlsProfile {
                    segment_container: HlsSegmentContainer::Ts,
                    video_codecs: vec![VideoCodec::H264],
                    audio_codecs: mp4_audio,
                },
            ],
            video_codecs: vec![VideoCodecSupport {
                codec: VideoCodec::H264,
                profiles: vec![
                    VideoProfile::H264High,
                    VideoProfile::H264Main,
                    VideoProfile::H264Baseline,
                    VideoProfile::H264ConstrainedBaseline,
                ],
                max_level: Some(51.0),
                max_bit_depth: Some(8),
                max_width: Some(4096),
                max_height: Some(2304),
                video_ranges: vec![VideoRange::Sdr],
            }],
            audio_codecs: vec![AudioCodecSupport {
                codec: AudioCodec::Aac,
                max_channels: Some(6),
            }],
            subtitle_codecs: vec![SubtitleCodec::WebVtt],
            max_height: None,
            max_bitrate_bps: None,
        }
    }

    pub fn video_support(&self, codec: VideoCodec) -> Option<&VideoCodecSupport> {
        self.video_codecs.iter().find(|s| s.codec == codec)
    }

    pub fn audio_support(&self, codec: AudioCodec) -> Option<&AudioCodecSupport> {
        self.audio_codecs.iter().find(|s| s.codec == codec)
    }
}

/// Server-side inputs to [`decide`] that aren't client capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecideOptions {
    /// A subtitle must be burned into the picture: rules out direct
    /// play and video stream copy.
    pub burn_subtitle: bool,
    /// Codecs the server is willing to encode video to, preferred first.
    pub encode_video: Vec<VideoCodec>,
    /// Codecs the server is willing to encode audio to, preferred first.
    pub encode_audio: Vec<AudioCodec>,
}

impl Default for DecideOptions {
    fn default() -> Self {
        Self {
            burn_subtitle: false,
            encode_video: vec![VideoCodec::H264],
            encode_audio: vec![AudioCodec::Aac],
        }
    }
}

/// Codecs ffmpeg has an encoder wired for (see `Encoder` and the audio
/// encoder table in `ffmpeg_argv`).
fn can_encode_video(codec: VideoCodec) -> bool {
    matches!(codec, VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1)
}

fn can_encode_audio(codec: AudioCodec) -> bool {
    matches!(
        codec,
        AudioCodec::Aac
            | AudioCodec::Opus
            | AudioCodec::Ac3
            | AudioCodec::Eac3
            | AudioCodec::Mp3
            | AudioCodec::Flac
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoPlan {
    /// The source has no video stream.
    None,
    /// Stream-copy; `range` is what the client will present (a Dolby
    /// Vision source may present as its base layer).
    Copy {
        codec: VideoCodec,
        range: VideoRange,
    },
    /// Re-encode to 8-bit SDR; `tonemap` maps an HDR source down first.
    /// `profile` is the most capable one the client decodes for `codec`.
    Encode {
        codec: VideoCodec,
        max_height: u32,
        tonemap: bool,
        profile: VideoProfile,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioPlan {
    None,
    Copy { codec: AudioCodec },
    Encode { codec: AudioCodec, channels: u8 },
}

/// Per-stream outcome of the HLS route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscodePlan {
    pub segment_container: HlsSegmentContainer,
    pub video: VideoPlan,
    pub audio: AudioPlan,
}

impl TranscodePlan {
    pub fn copies_video(&self) -> bool {
        !matches!(self.video, VideoPlan::Encode { .. })
    }

    pub fn copies_audio(&self) -> bool {
        !matches!(self.audio, AudioPlan::Encode { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaybackRoute {
    Direct,
    Transcoding {
        plan: TranscodePlan,
        reasons: TranscodeReasons,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackDecision {
    pub route: PlaybackRoute,
    /// Index into `ProbeReport::audio` of the track decided for (the
    /// default-flagged track, else the first); `None` without audio.
    pub audio_stream: Option<usize>,
}

/// Pure decision: direct play when some [`DirectPlayProfile`] accepts
/// the file as-is, otherwise the lightest HLS route — copy each stream
/// the chosen [`HlsProfile`] accepts, encode the rest. Reasons explain
/// the direct-play failure and every re-encoded stream; a non-direct
/// decision always carries at least one.
pub fn decide(
    probe: &ProbeReport,
    caps: &ClientCapabilities,
    opts: &DecideOptions,
) -> PlaybackDecision {
    let audio_stream = pick_audio(&probe.audio);
    let audio = audio_stream.map(|i| &probe.audio[i]);
    let video = probe.video.as_ref();
    let over_bitrate = matches!(
        (caps.max_bitrate_bps, probe.format_bit_rate),
        (Some(max), Some(bps)) if bps > max
    );
    let bitrate_bit = if over_bitrate {
        TranscodeReasons::ContainerBitrateExceedsLimit
    } else {
        TranscodeReasons::empty()
    };

    let direct_reasons = if opts.burn_subtitle {
        TranscodeReasons::SubtitleBurnIn
    } else {
        let mut best: Option<TranscodeReasons> = None;
        for profile in &caps.direct_play {
            let mut r = video_issues(video, &profile.video_codecs, caps)
                | audio_issues(audio, &profile.audio_codecs, caps)
                | bitrate_bit;
            if !probe
                .container
                .as_deref()
                .is_some_and(|c| same_container(c, &profile.container))
            {
                r |= TranscodeReasons::ContainerNotSupported;
            }
            if r.is_empty() {
                return PlaybackDecision {
                    route: PlaybackRoute::Direct,
                    audio_stream,
                };
            }
            if best.is_none_or(|b| r.bits().count_ones() < b.bits().count_ones()) {
                best = Some(r);
            }
        }
        best.unwrap_or(TranscodeReasons::ContainerNotSupported)
    };

    let burn_bit = if opts.burn_subtitle {
        TranscodeReasons::SubtitleBurnIn
    } else {
        TranscodeReasons::empty()
    };
    let mut chosen: Option<(u8, TranscodePlan, TranscodeReasons, TranscodeReasons)> = None;
    for profile in &caps.hls {
        let container = profile.segment_container;
        let video_ok: Vec<VideoCodec> = profile
            .video_codecs
            .iter()
            .copied()
            .filter(|c| container.carries_video(*c))
            .collect();
        let audio_ok: Vec<AudioCodec> = profile
            .audio_codecs
            .iter()
            .copied()
            .filter(|c| container.carries_audio(*c))
            .collect();
        let v_issues = match video {
            Some(_) => video_issues(video, &video_ok, caps) | burn_bit | bitrate_bit,
            None => TranscodeReasons::empty(),
        };
        let a_issues = audio_issues(audio, &audio_ok, caps)
            | if video.is_none() {
                bitrate_bit
            } else {
                TranscodeReasons::empty()
            };
        let Some(video_plan) = plan_video(video, v_issues, &video_ok, caps, opts) else {
            continue;
        };
        let Some(audio_plan) = plan_audio(audio, a_issues, &audio_ok, caps, opts) else {
            continue;
        };
        let plan = TranscodePlan {
            segment_container: container,
            video: video_plan,
            audio: audio_plan,
        };
        let score = u8::from(plan.copies_video()) * 2 + u8::from(plan.copies_audio());
        if chosen.as_ref().is_none_or(|(best, ..)| score > *best) {
            chosen = Some((score, plan, v_issues, a_issues));
        }
    }

    let (plan, v_issues, a_issues) = match chosen {
        Some((_, plan, v, a)) => (plan, v, a),
        // No declared profile is servable; H.264/AAC in TS is what
        // every HLS player handles.
        None => {
            let plan = TranscodePlan {
                segment_container: HlsSegmentContainer::Ts,
                video: match video {
                    Some(v) => VideoPlan::Encode {
                        codec: VideoCodec::H264,
                        max_height: encode_height(v, VideoCodec::H264, caps),
                        tonemap: VideoRange::of(v).is_hdr(),
                        profile: encode_profile(VideoCodec::H264, caps),
                    },
                    None => VideoPlan::None,
                },
                audio: match audio {
                    Some(a) => AudioPlan::Encode {
                        codec: AudioCodec::Aac,
                        channels: a.channels.clamp(1, 2),
                    },
                    None => AudioPlan::None,
                },
            };
            (
                plan,
                TranscodeReasons::VideoCodecNotSupported | burn_bit | bitrate_bit,
                TranscodeReasons::AudioCodecNotSupported,
            )
        }
    };

    let mut reasons = direct_reasons;
    if plan.copies_video() {
        reasons.remove(TranscodeReasons::VIDEO);
    } else {
        reasons |= v_issues;
    }
    if plan.copies_audio() {
        reasons.remove(TranscodeReasons::AUDIO);
    } else {
        reasons |= a_issues;
    }
    if reasons.is_empty() {
        // Each stream is fine on its own; it's the combination direct
        // play can't take.
        reasons = TranscodeReasons::ContainerNotSupported;
    }
    PlaybackDecision {
        route: PlaybackRoute::Transcoding { plan, reasons },
        audio_stream,
    }
}

fn pick_audio(audio: &[AudioStreamProbe]) -> Option<usize> {
    audio
        .iter()
        .position(|a| a.default)
        .or((!audio.is_empty()).then_some(0))
}

fn plan_video(
    video: Option<&VideoStreamProbe>,
    issues: TranscodeReasons,
    allowed: &[VideoCodec],
    caps: &ClientCapabilities,
    opts: &DecideOptions,
) -> Option<VideoPlan> {
    let Some(v) = video else {
        return Some(VideoPlan::None);
    };
    if issues.is_empty() {
        let ranges = caps
            .video_support(v.codec)
            .map_or(&[][..], |s| &s.video_ranges);
        let range = VideoRange::of(v).delivered(ranges)?;
        return Some(VideoPlan::Copy {
            codec: v.codec,
            range,
        });
    }
    let codec = opts
        .encode_video
        .iter()
        .chain(allowed)
        .copied()
        .find(|c| allowed.contains(c) && can_encode_video(*c))?;
    Some(VideoPlan::Encode {
        codec,
        max_height: encode_height(v, codec, caps),
        tonemap: VideoRange::of(v).is_hdr(),
        profile: encode_profile(codec, caps),
    })
}

fn plan_audio(
    audio: Option<&AudioStreamProbe>,
    issues: TranscodeReasons,
    allowed: &[AudioCodec],
    caps: &ClientCapabilities,
    opts: &DecideOptions,
) -> Option<AudioPlan> {
    let Some(a) = audio else {
        return Some(AudioPlan::None);
    };
    if issues.is_empty()
        && let Some(codec) = map_audio_codec(&a.codec)
    {
        return Some(AudioPlan::Copy { codec });
    }
    let codec = opts
        .encode_audio
        .iter()
        .chain(allowed)
        .copied()
        .find(|c| allowed.contains(c) && can_encode_audio(*c))?;
    let ceiling = caps
        .audio_support(codec)
        .and_then(|s| s.max_channels)
        .unwrap_or(6);
    Some(AudioPlan::Encode {
        codec,
        channels: a.channels.min(ceiling).max(1),
    })
}

/// The profile to encode `codec` in: the most capable 8-bit profile the
/// client lists for it (any, when it lists none). Encodes are always
/// 8-bit 4:2:0, so High 10 / Main 10 never apply.
pub fn encode_profile(codec: VideoCodec, caps: &ClientCapabilities) -> VideoProfile {
    let listed = caps
        .video_support(codec)
        .map_or(&[][..], |s| &s.profiles[..]);
    let accepts = |p: VideoProfile| listed.is_empty() || listed.contains(&p);
    let ladder: &[VideoProfile] = match codec {
        VideoCodec::H264 => &[
            VideoProfile::H264High,
            VideoProfile::H264Main,
            VideoProfile::H264ConstrainedBaseline,
            VideoProfile::H264Baseline,
        ],
        VideoCodec::H265 => &[VideoProfile::HevcMain],
        VideoCodec::Av1 => &[VideoProfile::Av1Main],
        _ => &[VideoProfile::Vp9Profile0],
    };
    ladder
        .iter()
        .copied()
        .find(|p| accepts(*p))
        .unwrap_or(ladder[0])
}

/// Output height for a re-encode: the source, capped by the display
/// ceiling and the target codec's decoder limit.
fn encode_height(v: &VideoStreamProbe, codec: VideoCodec, caps: &ClientCapabilities) -> u32 {
    let mut h = v.height;
    if let Some(max) = caps.max_height {
        h = h.min(max);
    }
    if let Some(max) = caps.video_support(codec).and_then(|s| s.max_height) {
        h = h.min(max);
    }
    h.max(1)
}

/// Everything about `video` that keeps it from being presented as-is
/// when the delivery accepts `allowed` codecs. Empty when there is no
/// video.
fn video_issues(
    video: Option<&VideoStreamProbe>,
    allowed: &[VideoCodec],
    caps: &ClientCapabilities,
) -> TranscodeReasons {
    let Some(v) = video else {
        return TranscodeReasons::empty();
    };
    if !allowed.contains(&v.codec) {
        return TranscodeReasons::VideoCodecNotSupported;
    }
    let support = caps.video_support(v.codec);
    let mut r = TranscodeReasons::empty();
    if let Some(s) = support {
        if let Some(p) = v.profile
            && !s.profiles.is_empty()
            && !s.profiles.contains(&p)
        {
            r |= TranscodeReasons::VideoProfileNotSupported;
        }
        // ffprobe reports unknown levels as negative (-99).
        if let (Some(level), Some(max)) = (v.level.filter(|l| *l > 0.0), s.max_level)
            && level > max
        {
            r |= TranscodeReasons::VideoLevelNotSupported;
        }
        if let (Some(depth), Some(max)) = (v.bit_depth, s.max_bit_depth)
            && depth > max
        {
            r |= TranscodeReasons::VideoBitDepthNotSupported;
        }
        if s.max_width.is_some_and(|w| v.width > w) || s.max_height.is_some_and(|h| v.height > h) {
            r |= TranscodeReasons::VideoResolutionNotSupported;
        }
    }
    if caps.max_height.is_some_and(|h| v.height > h) {
        r |= TranscodeReasons::VideoResolutionNotSupported;
    }
    if !chroma_ok(v, support) {
        r |= TranscodeReasons::PixelFormatNotSupported;
    }
    let ranges = support.map_or(&[][..], |s| &s.video_ranges);
    if VideoRange::of(v).delivered(ranges).is_none() {
        r |= TranscodeReasons::VideoRangeTypeNotSupported;
    }
    r
}

/// Browser decoders are 4:2:0-only unless the client explicitly lists
/// a chroma-extension profile the source carries. Profile strings
/// ffprobe can't name (e.g. H.264 High 4:2:2) leave only the pixel
/// format to go on.
fn chroma_ok(v: &VideoStreamProbe, support: Option<&VideoCodecSupport>) -> bool {
    let subsampled = !matches!(
        v.pixel_format,
        Some(
            PixelFormat::Yuv422p
                | PixelFormat::Yuv422p10le
                | PixelFormat::Yuv422p12le
                | PixelFormat::Yuv444p
                | PixelFormat::Yuv444p10le
                | PixelFormat::Yuv444p12le
        )
    );
    if subsampled {
        return true;
    }
    let extension = matches!(
        v.profile,
        Some(
            VideoProfile::Vp9Profile1
                | VideoProfile::Vp9Profile3
                | VideoProfile::HevcHigh
                | VideoProfile::HevcHigh10
                | VideoProfile::Av1High
                | VideoProfile::Av1Professional
        )
    );
    extension && support.is_some_and(|s| v.profile.is_some_and(|p| s.profiles.contains(&p)))
}

fn audio_issues(
    audio: Option<&AudioStreamProbe>,
    allowed: &[AudioCodec],
    caps: &ClientCapabilities,
) -> TranscodeReasons {
    let Some(a) = audio else {
        return TranscodeReasons::empty();
    };
    let Some(codec) = map_audio_codec(&a.codec).filter(|c| allowed.contains(c)) else {
        return TranscodeReasons::AudioCodecNotSupported;
    };
    if caps
        .audio_support(codec)
        .and_then(|s| s.max_channels)
        .is_some_and(|max| a.channels > max)
    {
        return TranscodeReasons::AudioChannelsNotSupported;
    }
    TranscodeReasons::empty()
}

/// ffprobe names demuxers, not containers: every ISO-BMFF file probes
/// as `mov`, Matroska and WebM both as `matroska`.
fn container_family(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "mov" | "mp4" | "m4a" | "m4v" | "3gp" | "3g2" | "mj2" => "mp4".to_string(),
        "mkv" | "webm" | "matroska" => "matroska".to_string(),
        "ts" | "m2ts" | "mpegts" => "mpegts".to_string(),
        "oga" | "ogg" | "ogv" => "ogg".to_string(),
        _ => name,
    }
}

fn same_container(source: &str, client: &str) -> bool {
    container_family(source) == container_family(client)
}

#[cfg(test)]
mod tests {
    use noflex_core::{ChannelLayout, ColorPrimaries, MatrixCoefficients};

    use super::*;

    fn h264_1080p() -> VideoStreamProbe {
        VideoStreamProbe {
            codec: VideoCodec::H264,
            profile: Some(VideoProfile::H264High),
            level: Some(40.0),
            width: 1920,
            height: 1080,
            bit_depth: Some(8),
            pixel_format: Some(PixelFormat::Yuv420p),
            frame_rate: Some(24.0),
            color_space: Some(ColorPrimaries::Bt709),
            transfer: Some(HdrTransfer::Sdr),
            matrix: Some(MatrixCoefficients::Bt709),
            dovi_profile: None,
            side_data: Default::default(),
        }
    }

    fn hevc_hdr10_4k() -> VideoStreamProbe {
        VideoStreamProbe {
            codec: VideoCodec::H265,
            profile: Some(VideoProfile::HevcMain10),
            level: Some(153.0),
            width: 3840,
            height: 2160,
            bit_depth: Some(10),
            pixel_format: Some(PixelFormat::Yuv420p10le),
            color_space: Some(ColorPrimaries::Bt2020),
            transfer: Some(HdrTransfer::Pq),
            matrix: Some(MatrixCoefficients::Bt2020Ncl),
            ..h264_1080p()
        }
    }

    fn audio(codec: &str, channels: u8) -> AudioStreamProbe {
        AudioStreamProbe {
            index: 1,
            codec: codec.to_string(),
            profile: Some("LC".to_string()),
            bit_depth: Some(16),
            sample_rate: Some(48_000),
            channels,
            channel_layout: Some(ChannelLayout::Stereo),
            language: Some("eng".to_string()),
            default: true,
            forced: false,
            title: None,
            audio_object_type: None,
        }
    }

    fn probe(container: &str, video: VideoStreamProbe, a: AudioStreamProbe) -> ProbeReport {
        ProbeReport {
            container: Some(container.to_string()),
            video: Some(video),
            audio: vec![a],
            subtitle: vec![],
            format_bit_rate: Some(5_000_000),
            duration_ms: None,
        }
    }

    fn basic() -> ProbeReport {
        probe("mov", h264_1080p(), audio("aac", 2))
    }

    /// A Chrome-like profile with HEVC/AV1 hardware and an HDR10 display.
    fn hdr_browser() -> ClientCapabilities {
        let mut caps = ClientCapabilities::web();
        caps.direct_play[0]
            .video_codecs
            .extend([VideoCodec::H265, VideoCodec::Av1]);
        caps.direct_play.push(DirectPlayProfile {
            container: "mkv".to_string(),
            video_codecs: vec![VideoCodec::H264, VideoCodec::H265],
            audio_codecs: vec![AudioCodec::Aac, AudioCodec::Opus],
        });
        caps.hls[0]
            .video_codecs
            .extend([VideoCodec::H265, VideoCodec::Av1]);
        caps.video_codecs.push(VideoCodecSupport {
            codec: VideoCodec::H265,
            profiles: vec![VideoProfile::HevcMain10, VideoProfile::HevcMain],
            max_level: Some(183.0),
            max_bit_depth: Some(10),
            max_width: Some(7680),
            max_height: Some(4320),
            video_ranges: vec![VideoRange::Hdr10, VideoRange::Hlg],
        });
        caps
    }

    fn transcoding(d: &PlaybackDecision) -> (TranscodePlan, TranscodeReasons) {
        match &d.route {
            PlaybackRoute::Transcoding { plan, reasons } => (*plan, *reasons),
            PlaybackRoute::Direct => panic!("expected transcode, got direct"),
        }
    }

    #[test]
    fn encode_profile_is_the_best_the_client_lists() {
        let mut caps = ClientCapabilities::web();
        let h264 = |caps: &mut ClientCapabilities, profiles: Vec<VideoProfile>| {
            if let Some(s) = caps
                .video_codecs
                .iter_mut()
                .find(|s| s.codec == VideoCodec::H264)
            {
                s.profiles = profiles;
            }
        };
        h264(&mut caps, vec![]);
        assert_eq!(
            encode_profile(VideoCodec::H264, &caps),
            VideoProfile::H264High
        );
        h264(
            &mut caps,
            vec![VideoProfile::H264Main, VideoProfile::H264Baseline],
        );
        assert_eq!(
            encode_profile(VideoCodec::H264, &caps),
            VideoProfile::H264Main
        );
        h264(
            &mut caps,
            vec![
                VideoProfile::H264High10,
                VideoProfile::H264ConstrainedBaseline,
            ],
        );
        assert_eq!(
            encode_profile(VideoCodec::H264, &caps),
            VideoProfile::H264ConstrainedBaseline,
            "High 10 doesn't help an 8-bit encode"
        );
        assert_eq!(
            encode_profile(VideoCodec::H265, &caps),
            VideoProfile::HevcMain
        );
    }

    fn decide_default(p: &ProbeReport, caps: &ClientCapabilities) -> PlaybackDecision {
        decide(p, caps, &DecideOptions::default())
    }

    #[test]
    fn direct_when_profile_matches() {
        let d = decide_default(&basic(), &ClientCapabilities::web());
        assert_eq!(d.route, PlaybackRoute::Direct);
        assert_eq!(d.audio_stream, Some(0));
    }

    #[test]
    fn mkv_h264_aac_remuxes_without_reencoding() {
        let mut p = basic();
        p.container = Some("matroska".to_string());
        let (plan, reasons) = transcoding(&decide_default(&p, &ClientCapabilities::web()));
        assert_eq!(reasons, TranscodeReasons::ContainerNotSupported);
        assert!(matches!(
            plan.video,
            VideoPlan::Copy {
                codec: VideoCodec::H264,
                range: VideoRange::Sdr
            }
        ));
        assert!(matches!(
            plan.audio,
            AudioPlan::Copy {
                codec: AudioCodec::Aac
            }
        ));
        assert_eq!(plan.segment_container, HlsSegmentContainer::Fmp4);
    }

    #[test]
    fn mkv_direct_plays_when_client_lists_matroska() {
        let mut p = probe("matroska", hevc_hdr10_4k(), audio("opus", 2));
        p.format_bit_rate = Some(30_000_000);
        assert_eq!(
            decide_default(&p, &hdr_browser()).route,
            PlaybackRoute::Direct
        );
    }

    #[test]
    fn hevc_hdr10_on_sdr_client_tonemaps() {
        let p = probe("mov", hevc_hdr10_4k(), audio("aac", 2));
        let (plan, reasons) = transcoding(&decide_default(&p, &ClientCapabilities::web()));
        assert!(reasons.contains(TranscodeReasons::VideoCodecNotSupported));
        assert!(matches!(
            plan.video,
            VideoPlan::Encode {
                codec: VideoCodec::H264,
                tonemap: true,
                max_height: 2160,
                profile: VideoProfile::H264High,
            }
        ));
        assert!(plan.copies_audio());
    }

    #[test]
    fn hevc_hdr10_on_hevc_sdr_client_flags_range() {
        let mut caps = hdr_browser();
        caps.video_codecs[1].video_ranges.clear();
        let p = probe("mov", hevc_hdr10_4k(), audio("aac", 2));
        let (plan, reasons) = transcoding(&decide_default(&p, &caps));
        assert_eq!(reasons, TranscodeReasons::VideoRangeTypeNotSupported);
        assert!(matches!(
            plan.video,
            VideoPlan::Encode { tonemap: true, .. }
        ));
    }

    #[test]
    fn hevc_in_mkv_with_truehd_copies_video_encodes_audio() {
        let caps = hdr_browser();
        let mut p = probe("matroska", hevc_hdr10_4k(), audio("truehd", 8));
        p.format_bit_rate = Some(40_000_000);
        let (plan, reasons) = transcoding(&decide_default(&p, &caps));
        assert!(matches!(
            plan.video,
            VideoPlan::Copy {
                codec: VideoCodec::H265,
                range: VideoRange::Hdr10
            }
        ));
        assert_eq!(
            plan.audio,
            AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: 6
            }
        );
        assert_eq!(plan.segment_container, HlsSegmentContainer::Fmp4);
        assert_eq!(reasons, TranscodeReasons::AudioCodecNotSupported);
    }

    #[test]
    fn dovi_profile8_falls_back_to_hdr10_base_layer() {
        let mut v = hevc_hdr10_4k();
        v.dovi_profile = Some(DolbyVisionProfile::Profile8);
        let p = probe("mov", v, audio("aac", 2));
        let d = decide_default(&p, &hdr_browser());
        assert_eq!(d.route, PlaybackRoute::Direct);
        let mut caps = hdr_browser();
        caps.direct_play.clear();
        let (plan, _) = transcoding(&decide_default(&p, &caps));
        assert!(matches!(
            plan.video,
            VideoPlan::Copy {
                range: VideoRange::Hdr10,
                ..
            }
        ));
    }

    #[test]
    fn dovi_profile5_needs_dovi() {
        let mut v = hevc_hdr10_4k();
        v.dovi_profile = Some(DolbyVisionProfile::Profile5);
        let p = probe("mov", v, audio("aac", 2));
        let (plan, reasons) = transcoding(&decide_default(&p, &hdr_browser()));
        assert!(reasons.contains(TranscodeReasons::VideoRangeTypeNotSupported));
        assert!(matches!(
            plan.video,
            VideoPlan::Encode { tonemap: true, .. }
        ));

        let mut caps = hdr_browser();
        caps.video_codecs[1].video_ranges.push(VideoRange::Dovi);
        assert_eq!(decide_default(&p, &caps).route, PlaybackRoute::Direct);
    }

    #[test]
    fn dovi_profile8_hlg_needs_hlg_or_dovi() {
        assert_eq!(
            VideoRange::DoviWithHlg.delivered(&[VideoRange::Hlg]),
            Some(VideoRange::Hlg)
        );
        assert_eq!(
            VideoRange::DoviWithHlg.delivered(&[VideoRange::Hdr10]),
            None
        );
        assert_eq!(
            VideoRange::DoviWithSdr.delivered(&[]),
            Some(VideoRange::Sdr)
        );
        assert_eq!(
            VideoRange::DoviWithHdr10.delivered(&[VideoRange::DoviWithHdr10]),
            Some(VideoRange::DoviWithHdr10)
        );
        assert_eq!(VideoRange::Dovi.delivered(&[VideoRange::Hdr10]), None);
    }

    #[test]
    fn av1_webm_direct_plays_without_support_entry() {
        let v = VideoStreamProbe {
            codec: VideoCodec::Av1,
            profile: Some(VideoProfile::Av1Main),
            level: Some(8.0),
            ..h264_1080p()
        };
        let p = probe("matroska", v, audio("opus", 2));
        assert_eq!(
            decide_default(&p, &ClientCapabilities::web()).route,
            PlaybackRoute::Direct
        );
    }

    #[test]
    fn level_and_profile_limits_force_video_encode() {
        let mut p = basic();
        p.video.as_mut().unwrap().level = Some(52.0);
        let (plan, reasons) = transcoding(&decide_default(&p, &ClientCapabilities::web()));
        assert_eq!(reasons, TranscodeReasons::VideoLevelNotSupported);
        assert!(!plan.copies_video());
        assert!(plan.copies_audio());

        let mut p = basic();
        let v = p.video.as_mut().unwrap();
        v.profile = Some(VideoProfile::H264High10);
        v.bit_depth = Some(10);
        let (_, reasons) = transcoding(&decide_default(&p, &ClientCapabilities::web()));
        assert!(reasons.contains(TranscodeReasons::VideoProfileNotSupported));
        assert!(reasons.contains(TranscodeReasons::VideoBitDepthNotSupported));
    }

    #[test]
    fn unknown_level_is_not_a_reason() {
        let mut p = basic();
        p.video.as_mut().unwrap().level = Some(-99.0);
        assert_eq!(
            decide_default(&p, &ClientCapabilities::web()).route,
            PlaybackRoute::Direct
        );
    }

    #[test]
    fn chroma_444_needs_a_listed_extension_profile() {
        let mut p = basic();
        p.video.as_mut().unwrap().pixel_format = Some(PixelFormat::Yuv444p);
        p.video.as_mut().unwrap().profile = None;
        let (_, reasons) = transcoding(&decide_default(&p, &ClientCapabilities::web()));
        assert_eq!(reasons, TranscodeReasons::PixelFormatNotSupported);
    }

    #[test]
    fn display_ceiling_scales_down() {
        let mut caps = ClientCapabilities::web();
        caps.max_height = Some(720);
        let (plan, reasons) = transcoding(&decide_default(&basic(), &caps));
        assert_eq!(reasons, TranscodeReasons::VideoResolutionNotSupported);
        assert!(matches!(
            plan.video,
            VideoPlan::Encode {
                max_height: 720,
                tonemap: false,
                ..
            }
        ));
    }

    #[test]
    fn bitrate_ceiling_forces_video_encode() {
        let mut caps = ClientCapabilities::web();
        caps.max_bitrate_bps = Some(1_000_000);
        let (plan, reasons) = transcoding(&decide_default(&basic(), &caps));
        assert_eq!(reasons, TranscodeReasons::ContainerBitrateExceedsLimit);
        assert!(!plan.copies_video());
        assert!(plan.copies_audio());
    }

    #[test]
    fn audio_channels_over_limit_encode_audio_only() {
        let p = probe("mov", h264_1080p(), audio("aac", 8));
        let (plan, reasons) = transcoding(&decide_default(&p, &ClientCapabilities::web()));
        assert_eq!(reasons, TranscodeReasons::AudioChannelsNotSupported);
        assert!(plan.copies_video());
        assert_eq!(
            plan.audio,
            AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: 6
            }
        );
    }

    #[test]
    fn ac3_prefers_ts_profile_when_only_it_carries_the_codec() {
        let mut caps = ClientCapabilities::web();
        caps.hls[1].audio_codecs.push(AudioCodec::Ac3);
        let mut p = probe("matroska", h264_1080p(), audio("ac3", 6));
        p.format_bit_rate = None;
        let (plan, reasons) = transcoding(&decide_default(&p, &caps));
        assert_eq!(plan.segment_container, HlsSegmentContainer::Ts);
        assert!(plan.copies_video() && plan.copies_audio());
        assert_eq!(reasons, TranscodeReasons::ContainerNotSupported);
    }

    #[test]
    fn ts_never_carries_av1_copies() {
        let mut caps = ClientCapabilities::web();
        caps.hls = vec![HlsProfile {
            segment_container: HlsSegmentContainer::Ts,
            video_codecs: vec![VideoCodec::Av1, VideoCodec::H264],
            audio_codecs: vec![AudioCodec::Aac],
        }];
        let v = VideoStreamProbe {
            codec: VideoCodec::Av1,
            ..h264_1080p()
        };
        let p = probe("mov", v, audio("aac", 2));
        let (plan, _) = transcoding(&decide_default(&p, &caps));
        assert!(matches!(
            plan.video,
            VideoPlan::Encode {
                codec: VideoCodec::H264,
                ..
            }
        ));
    }

    #[test]
    fn subtitles_never_force_a_transcode() {
        let mut p = basic();
        p.subtitle = vec![crate::probe::SubtitleStreamProbe {
            index: 2,
            codec: SubtitleCodec::PgsImage,
            language: None,
            default: true,
            forced: false,
            external: false,
            title: None,
        }];
        assert_eq!(
            decide_default(&p, &ClientCapabilities::web()).route,
            PlaybackRoute::Direct
        );
    }

    #[test]
    fn burn_in_forces_video_encode() {
        let opts = DecideOptions {
            burn_subtitle: true,
            ..Default::default()
        };
        let (plan, reasons) = transcoding(&decide(&basic(), &ClientCapabilities::web(), &opts));
        assert_eq!(reasons, TranscodeReasons::SubtitleBurnIn);
        assert!(!plan.copies_video());
        assert!(plan.copies_audio());
    }

    #[test]
    fn audio_only_file_direct_plays_or_encodes_audio() {
        let p = ProbeReport {
            container: Some("flac".to_string()),
            video: None,
            audio: vec![audio("flac", 2)],
            subtitle: vec![],
            format_bit_rate: None,
            duration_ms: None,
        };
        let mut caps = ClientCapabilities::web();
        let (plan, reasons) = transcoding(&decide_default(&p, &caps));
        assert_eq!(plan.video, VideoPlan::None);
        assert!(matches!(
            plan.audio,
            AudioPlan::Encode {
                codec: AudioCodec::Aac,
                ..
            }
        ));
        assert!(reasons.contains(TranscodeReasons::AudioCodecNotSupported));
        caps.direct_play.push(DirectPlayProfile {
            container: "flac".to_string(),
            video_codecs: vec![],
            audio_codecs: vec![AudioCodec::Flac],
        });
        assert_eq!(decide_default(&p, &caps).route, PlaybackRoute::Direct);
    }

    #[test]
    fn default_audio_track_is_decided_for() {
        let mut p = basic();
        let mut second = audio("truehd", 8);
        p.audio[0].default = false;
        second.default = true;
        p.audio.push(second);
        let d = decide_default(&p, &ClientCapabilities::web());
        assert_eq!(d.audio_stream, Some(1));
        let (plan, _) = transcoding(&d);
        assert!(!plan.copies_audio());
    }

    #[test]
    fn no_hls_profiles_falls_back_to_h264_aac_ts() {
        let mut caps = ClientCapabilities::web();
        caps.hls.clear();
        let mut p = basic();
        p.container = Some("avi".to_string());
        let (plan, reasons) = transcoding(&decide_default(&p, &caps));
        assert_eq!(plan.segment_container, HlsSegmentContainer::Ts);
        assert!(!plan.copies_video() && !plan.copies_audio());
        assert!(!reasons.is_empty());
    }

    #[test]
    fn iso_bmff_and_matroska_families_compare_as_one() {
        assert!(same_container("mov", "mp4"));
        assert!(same_container("matroska", "webm"));
        assert!(same_container("matroska", "mkv"));
        assert!(same_container("mpegts", "ts"));
        assert!(!same_container("matroska", "mp4"));
    }
}
