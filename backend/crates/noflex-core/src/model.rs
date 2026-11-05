// SPDX-License-Identifier: AGPL-3.0-only

use std::str::FromStr;

use async_graphql::{Enum, InputObject, SimpleObject};
use serde::{Deserialize, Serialize};

use crate::ids::{EditionId, MediaFileId, WorkId};

/// What a Work *is*. Drives scanner heuristics and metadata provider selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum WorkKind {
    Movie,
    Series,
    Season,
    Episode,
    Album,
    Track,
    Artist,
    AudioBook,
    Photo,
}

/// Confidence level on a Work's canonical metadata (`title`, `year`,
/// `overview`, …). The scanner always lands at [`Unconfirmed`]; metadata
/// providers escalate to [`Confirmed`]; users escalate to [`UserEdited`].
///
/// Frontends use this to:
///   - badge unconfirmed items so end users know they're filesystem
///     heuristics, not verified metadata;
///   - decide whether a provider refresh should overwrite the row.
///
/// Note that the variants form a precedence ladder, not a state machine:
/// `UserEdited` outranks `Confirmed` (a manual edit beats a provider
/// refresh) and `Confirmed` outranks `Unconfirmed` (a provider beats the
/// scanner). Downgrading is always a deliberate caller decision, never a
/// scanner side-effect.
///
/// [`Unconfirmed`]: MetadataStatus::Unconfirmed
/// [`Confirmed`]: MetadataStatus::Confirmed
/// [`UserEdited`]: MetadataStatus::UserEdited
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum MetadataStatus {
    #[default]
    Unconfirmed,
    Confirmed,
    UserEdited,
}

/// The per-work metadata fields a user can lock against provider
/// refreshes: once locked, a refresh must not overwrite that field.
/// [`ExternalId`] is deliberately absent — external IDs are always
/// merged, never overwritten.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Enum,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkField {
    Title,
    SortTitle,
    OriginalTitle,
    Year,
    ReleaseDate,
    Overview,
    Tagline,
    CommunityRating,
    Genres,
    Studios,
    People,
    ParentalRating,
}

/// Cross-references to external metadata sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum ExternalIdKind {
    Tmdb,
    Tvdb,
    Imdb,
    MusicBrainzReleaseGroup,
    MusicBrainzArtist,
    MusicBrainzRecording,
    AniDb,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, SimpleObject, InputObject)]
#[graphql(input_name = "ExternalIdInput")]
pub struct ExternalId {
    pub kind: ExternalIdKind,
    pub value: String,
}

/// Image kinds that can be attached to a target (work, edition, or person).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    Primary,
    Backdrop,
    Banner,
    Thumb,
    Logo,
    Disc,
    Box,
    Menu,
    Chapter,
    Art,
}

impl ImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ImageKind::Primary => "primary",
            ImageKind::Backdrop => "backdrop",
            ImageKind::Banner => "banner",
            ImageKind::Thumb => "thumb",
            ImageKind::Logo => "logo",
            ImageKind::Disc => "disc",
            ImageKind::Box => "box",
            ImageKind::Menu => "menu",
            ImageKind::Chapter => "chapter",
            ImageKind::Art => "art",
        }
    }

    // Inherent rather than `FromStr`: an unknown kind is a routine lookup
    // miss for callers (row mappers, route parsing), not an error payload.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "primary" => ImageKind::Primary,
            "backdrop" => ImageKind::Backdrop,
            "banner" => ImageKind::Banner,
            "thumb" => ImageKind::Thumb,
            "logo" => ImageKind::Logo,
            "disc" => ImageKind::Disc,
            "box" => ImageKind::Box,
            "menu" => ImageKind::Menu,
            "chapter" => ImageKind::Chapter,
            "art" => ImageKind::Art,
            _ => return None,
        })
    }
}

/// What gets tagged. Tags can attach to any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum TagTarget {
    Work(WorkId),
    Edition(EditionId),
    MediaFile(MediaFileId),
}

/// What playback state attaches to. Edition for per-cut state, Work for "watched the movie."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum PlayTarget {
    Work(WorkId),
    Edition(EditionId),
}

impl PlayTarget {
    pub fn as_str_kind(&self) -> &'static str {
        match self {
            PlayTarget::Work(_) => "work",
            PlayTarget::Edition(_) => "edition",
        }
    }

    pub fn id_uuid(&self) -> uuid::Uuid {
        match self {
            PlayTarget::Work(id) => id.0,
            PlayTarget::Edition(id) => id.0,
        }
    }
}

/// Closed enum of [`PlayTarget`]'s variant tag. Exposed as a separate
/// type so storage rows and the GQL `HistoryRow.targetKind` field can
/// use it directly without round-tripping through the enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum PlayTargetKind {
    Work,
    Edition,
}

impl From<PlayTarget> for PlayTargetKind {
    fn from(t: PlayTarget) -> Self {
        match t {
            PlayTarget::Work(_) => PlayTargetKind::Work,
            PlayTarget::Edition(_) => PlayTargetKind::Edition,
        }
    }
}

impl From<PlayTargetKind> for &'static str {
    fn from(k: PlayTargetKind) -> Self {
        match k {
            PlayTargetKind::Work => "work",
            PlayTargetKind::Edition => "edition",
        }
    }
}

/// Server-side user role. Admin sees every library and can manage
/// other users; User is the standard interactive role with per-library
/// access gating; Guest is read-only and defaults to no library
/// access unless explicitly granted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    #[default]
    User,
    Admin,
    Guest,
}

/// What an embedding represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum EmbeddingTarget {
    Work(WorkId),
    Edition(EditionId),
    MediaFile(MediaFileId),
}

// ---------------------------------------------------------------------------
// v2 codec / HDR / channel-layout matrix
//
// Typed replacements for the loose `Option<String>` fields the legacy
// `Edition`/`AudioTrack`/`SubtitleTrack` schemas used. Each variant
// is the canonical codec/profile/etc. string the probe pipeline emits,
// serialised to the same snake_case spelling via `serde(rename_all)`
// so the on-disk column stays a plain TEXT.
// ---------------------------------------------------------------------------

/// Video codec carried by a `MediaFile`/`Edition`. Closed set; the
/// transcoder's `decide()` function in `noflex-transcoder` reads these
/// directly to pick a direct-play vs. transcode route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    H264,
    H265,
    #[graphql(name = "AV1")]
    Av1,
    #[graphql(name = "VP9")]
    Vp9,
    #[graphql(name = "VP8")]
    Vp8,
    #[graphql(name = "MPEG2")]
    Mpeg2,
    Prores,
    Theora,
}

impl VideoCodec {
    pub fn as_str(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::H265 => "h265",
            VideoCodec::Av1 => "av1",
            VideoCodec::Vp9 => "vp9",
            VideoCodec::Vp8 => "vp8",
            VideoCodec::Mpeg2 => "mpeg2",
            VideoCodec::Prores => "prores",
            VideoCodec::Theora => "theora",
        }
    }
}

impl FromStr for VideoCodec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "h264" => VideoCodec::H264,
            "h265" => VideoCodec::H265,
            "av1" => VideoCodec::Av1,
            "vp9" => VideoCodec::Vp9,
            "vp8" => VideoCodec::Vp8,
            "mpeg2" => VideoCodec::Mpeg2,
            "prores" => VideoCodec::Prores,
            "theora" => VideoCodec::Theora,
            other => return Err(format!("unknown video codec {other}")),
        })
    }
}

/// Audio codec carried by a `MediaFile`/`Edition` and requested by
/// client capability declarations. Closed set, same contract as
/// [`VideoCodec`]; probe-side audio codec names stay `String`
/// because ffprobe reports an open-ended set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    Aac,
    #[graphql(name = "AC3")]
    Ac3,
    #[graphql(name = "EAC3")]
    Eac3,
    #[graphql(name = "MP3")]
    Mp3,
    Opus,
    Vorbis,
    Flac,
    Truehd,
    Dts,
    Pcm,
}

impl AudioCodec {
    pub fn as_str(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Ac3 => "ac3",
            AudioCodec::Eac3 => "eac3",
            AudioCodec::Mp3 => "mp3",
            AudioCodec::Opus => "opus",
            AudioCodec::Vorbis => "vorbis",
            AudioCodec::Flac => "flac",
            AudioCodec::Truehd => "truehd",
            AudioCodec::Dts => "dts",
            AudioCodec::Pcm => "pcm",
        }
    }
}

impl FromStr for AudioCodec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "aac" => AudioCodec::Aac,
            "ac3" => AudioCodec::Ac3,
            "eac3" => AudioCodec::Eac3,
            "mp3" => AudioCodec::Mp3,
            "opus" => AudioCodec::Opus,
            "vorbis" => AudioCodec::Vorbis,
            "flac" => AudioCodec::Flac,
            "truehd" => AudioCodec::Truehd,
            "dts" => AudioCodec::Dts,
            "pcm" => AudioCodec::Pcm,
            other => return Err(format!("unknown audio codec {other}")),
        })
    }
}

/// The curve mapping HDR down to SDR when a re-encode has to. A matter of taste
/// more than correctness, so an administrator picks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum TonemapCurve {
    /// Filmic, with soft highlights.
    #[default]
    Hable,
    /// Keeps midtones close to the source and compresses only the highlights.
    Mobius,
    Reinhard,
    /// Only on the GPU path; the CPU path uses [`TonemapCurve::Hable`] instead.
    Spline,
    /// Only on the GPU path; the CPU path uses [`TonemapCurve::Hable`] instead.
    #[graphql(name = "BT2390")]
    Bt2390,
}

impl TonemapCurve {
    /// Its name to libplacebo's `tonemapping`.
    pub fn gpu_name(self) -> &'static str {
        match self {
            Self::Hable => "hable",
            Self::Mobius => "mobius",
            Self::Reinhard => "reinhard",
            Self::Spline => "spline",
            Self::Bt2390 => "bt.2390",
        }
    }

    /// Its name to ffmpeg's `tonemap` filter, which has fewer.
    pub fn cpu_name(self) -> &'static str {
        match self {
            Self::Mobius => "mobius",
            Self::Reinhard => "reinhard",
            Self::Hable | Self::Spline | Self::Bt2390 => "hable",
        }
    }
}

/// H.264/H.265/AV1/VP9 codec profile tier. Drives direct-play
/// decisions (e.g. a 10-bit HEVC Main stream on a Main10-only client).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum VideoProfile {
    H264Baseline,
    H264ConstrainedBaseline,
    H264Main,
    H264High,
    #[graphql(name = "H264_HIGH10")]
    H264High10,
    HevcMain,
    #[graphql(name = "HEVC_MAIN10")]
    HevcMain10,
    HevcHigh,
    #[graphql(name = "HEVC_HIGH10")]
    HevcHigh10,
    #[graphql(name = "AV1_MAIN")]
    Av1Main,
    #[graphql(name = "AV1_HIGH")]
    Av1High,
    #[graphql(name = "AV1_PROFESSIONAL")]
    Av1Professional,
    #[graphql(name = "VP9_PROFILE0")]
    Vp9Profile0,
    #[graphql(name = "VP9_PROFILE1")]
    Vp9Profile1,
    #[graphql(name = "VP9_PROFILE2")]
    Vp9Profile2,
    #[graphql(name = "VP9_PROFILE3")]
    Vp9Profile3,
}

impl VideoProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            VideoProfile::H264Baseline => "h264_baseline",
            VideoProfile::H264ConstrainedBaseline => "h264_constrained_baseline",
            VideoProfile::H264Main => "h264_main",
            VideoProfile::H264High => "h264_high",
            VideoProfile::H264High10 => "h264_high10",
            VideoProfile::HevcMain => "hevc_main",
            VideoProfile::HevcMain10 => "hevc_main10",
            VideoProfile::HevcHigh => "hevc_high",
            VideoProfile::HevcHigh10 => "hevc_high10",
            VideoProfile::Av1Main => "av1_main",
            VideoProfile::Av1High => "av1_high",
            VideoProfile::Av1Professional => "av1_professional",
            VideoProfile::Vp9Profile0 => "vp9_profile0",
            VideoProfile::Vp9Profile1 => "vp9_profile1",
            VideoProfile::Vp9Profile2 => "vp9_profile2",
            VideoProfile::Vp9Profile3 => "vp9_profile3",
        }
    }
}

impl FromStr for VideoProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "h264_baseline" => VideoProfile::H264Baseline,
            "h264_constrained_baseline" => VideoProfile::H264ConstrainedBaseline,
            "h264_main" => VideoProfile::H264Main,
            "h264_high" => VideoProfile::H264High,
            "h264_high10" => VideoProfile::H264High10,
            "hevc_main" => VideoProfile::HevcMain,
            "hevc_main10" => VideoProfile::HevcMain10,
            "hevc_high" => VideoProfile::HevcHigh,
            "hevc_high10" => VideoProfile::HevcHigh10,
            "av1_main" => VideoProfile::Av1Main,
            "av1_high" => VideoProfile::Av1High,
            "av1_professional" => VideoProfile::Av1Professional,
            "vp9_profile0" => VideoProfile::Vp9Profile0,
            "vp9_profile1" => VideoProfile::Vp9Profile1,
            "vp9_profile2" => VideoProfile::Vp9Profile2,
            "vp9_profile3" => VideoProfile::Vp9Profile3,
            other => return Err(format!("unknown video profile {other}")),
        })
    }
}

/// Pixel format / chroma subsampling carried by a video stream.
/// Matched to the FFmpeg pix_fmt names so the probe pipeline can
/// hand back a value directly via `as_str()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    #[graphql(name = "YUV420P")]
    Yuv420p,
    #[graphql(name = "YUV420P10LE")]
    Yuv420p10le,
    #[graphql(name = "YUV422P")]
    Yuv422p,
    #[graphql(name = "YUV422P10LE")]
    Yuv422p10le,
    #[graphql(name = "YUV444P")]
    Yuv444p,
    #[graphql(name = "YUV444P10LE")]
    Yuv444p10le,
    Gray,
    #[graphql(name = "YUV420P12LE")]
    Yuv420p12le,
    #[graphql(name = "YUV422P12LE")]
    Yuv422p12le,
    #[graphql(name = "YUV444P12LE")]
    Yuv444p12le,
}

impl PixelFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            PixelFormat::Yuv420p => "yuv420p",
            PixelFormat::Yuv420p10le => "yuv420p10le",
            PixelFormat::Yuv422p => "yuv422p",
            PixelFormat::Yuv422p10le => "yuv422p10le",
            PixelFormat::Yuv444p => "yuv444p",
            PixelFormat::Yuv444p10le => "yuv444p10le",
            PixelFormat::Gray => "gray",
            PixelFormat::Yuv420p12le => "yuv420p12le",
            PixelFormat::Yuv422p12le => "yuv422p12le",
            PixelFormat::Yuv444p12le => "yuv444p12le",
        }
    }
}

impl FromStr for PixelFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "yuv420p" => PixelFormat::Yuv420p,
            "yuv420p10le" => PixelFormat::Yuv420p10le,
            "yuv422p" => PixelFormat::Yuv422p,
            "yuv422p10le" => PixelFormat::Yuv422p10le,
            "yuv444p" => PixelFormat::Yuv444p,
            "yuv444p10le" => PixelFormat::Yuv444p10le,
            "gray" => PixelFormat::Gray,
            "yuv420p12le" => PixelFormat::Yuv420p12le,
            "yuv422p12le" => PixelFormat::Yuv422p12le,
            "yuv444p12le" => PixelFormat::Yuv444p12le,
            other => return Err(format!("unknown pixel format {other}")),
        })
    }
}

/// Transfer characteristics (a.k.a. transfer function, "gamma curve").
/// `Sdr` is the Rec.709 / sRGB default; the rest are the HDR and
/// log-curve variants the probe pipeline distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum HdrTransfer {
    Sdr,
    Pq,
    Hlg,
    #[graphql(name = "SLOG3")]
    SLog3,
    #[graphql(name = "SLOG2")]
    SLog2,
    LogC,
    Bt2100,
    Bt1886,
}

impl HdrTransfer {
    pub fn as_str(self) -> &'static str {
        match self {
            HdrTransfer::Sdr => "sdr",
            HdrTransfer::Pq => "pq",
            HdrTransfer::Hlg => "hlg",
            HdrTransfer::SLog3 => "slog3",
            HdrTransfer::SLog2 => "slog2",
            HdrTransfer::LogC => "logc",
            HdrTransfer::Bt2100 => "bt2100",
            HdrTransfer::Bt1886 => "bt1886",
        }
    }
}

impl FromStr for HdrTransfer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "sdr" => HdrTransfer::Sdr,
            "pq" => HdrTransfer::Pq,
            "hlg" => HdrTransfer::Hlg,
            "slog3" => HdrTransfer::SLog3,
            "slog2" => HdrTransfer::SLog2,
            "logc" => HdrTransfer::LogC,
            "bt2100" => HdrTransfer::Bt2100,
            "bt1886" => HdrTransfer::Bt1886,
            other => return Err(format!("unknown transfer characteristics {other}")),
        })
    }
}

/// Color primaries (a.k.a. color space). Defines the gamut; the
/// HDR10 / Dolby Vision static-metadata path uses `Bt2020`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum ColorPrimaries {
    Bt709,
    Bt2020,
    Bt601Ntsc,
    Bt601Pal,
    SmpteC,
    DciP3,
    DisplayP3,
    Bt470M,
    Bt470Bg,
    Xyz,
    Film,
}

impl ColorPrimaries {
    pub fn as_str(self) -> &'static str {
        match self {
            ColorPrimaries::Bt709 => "bt709",
            ColorPrimaries::Bt2020 => "bt2020",
            ColorPrimaries::Bt601Ntsc => "bt601_ntsc",
            ColorPrimaries::Bt601Pal => "bt601_pal",
            ColorPrimaries::SmpteC => "smpte_c",
            ColorPrimaries::DciP3 => "dci_p3",
            ColorPrimaries::DisplayP3 => "display_p3",
            ColorPrimaries::Bt470M => "bt470m",
            ColorPrimaries::Bt470Bg => "bt470bg",
            ColorPrimaries::Xyz => "xyz",
            ColorPrimaries::Film => "film",
        }
    }
}

impl FromStr for ColorPrimaries {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "bt709" => ColorPrimaries::Bt709,
            "bt2020" => ColorPrimaries::Bt2020,
            "bt601_ntsc" => ColorPrimaries::Bt601Ntsc,
            "bt601_pal" => ColorPrimaries::Bt601Pal,
            "smpte_c" => ColorPrimaries::SmpteC,
            "dci_p3" => ColorPrimaries::DciP3,
            "display_p3" => ColorPrimaries::DisplayP3,
            "bt470m" => ColorPrimaries::Bt470M,
            "bt470bg" => ColorPrimaries::Bt470Bg,
            "xyz" => ColorPrimaries::Xyz,
            "film" => ColorPrimaries::Film,
            other => return Err(format!("unknown color primaries {other}")),
        })
    }
}

/// Matrix coefficients. Defines YCbCr ↔ RGB conversion. `Bt2020Ncl`
/// is the non-constant-luminance variant the HDR path uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum MatrixCoefficients {
    Bt709,
    Bt470Bg,
    Bt601Ntsc,
    Bt601Pal,
    Smpte240M,
    YCgCo,
    Bt2020Ncl,
    Bt2020Cl,
    Smpte2085,
    ChromaDerivedNc,
    ChromaDerivedCl,
    Ictcp,
}

impl MatrixCoefficients {
    pub fn as_str(self) -> &'static str {
        match self {
            MatrixCoefficients::Bt709 => "bt709",
            MatrixCoefficients::Bt470Bg => "bt470bg",
            MatrixCoefficients::Bt601Ntsc => "bt601_ntsc",
            MatrixCoefficients::Bt601Pal => "bt601_pal",
            MatrixCoefficients::Smpte240M => "smpte240m",
            MatrixCoefficients::YCgCo => "ycgco",
            MatrixCoefficients::Bt2020Ncl => "bt2020_ncl",
            MatrixCoefficients::Bt2020Cl => "bt2020_cl",
            MatrixCoefficients::Smpte2085 => "smpte2085",
            MatrixCoefficients::ChromaDerivedNc => "chroma_derived_nc",
            MatrixCoefficients::ChromaDerivedCl => "chroma_derived_cl",
            MatrixCoefficients::Ictcp => "ictcp",
        }
    }
}

impl FromStr for MatrixCoefficients {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "bt709" => MatrixCoefficients::Bt709,
            "bt470bg" => MatrixCoefficients::Bt470Bg,
            "bt601_ntsc" => MatrixCoefficients::Bt601Ntsc,
            "bt601_pal" => MatrixCoefficients::Bt601Pal,
            "smpte240m" => MatrixCoefficients::Smpte240M,
            "ycgco" => MatrixCoefficients::YCgCo,
            "bt2020_ncl" => MatrixCoefficients::Bt2020Ncl,
            "bt2020_cl" => MatrixCoefficients::Bt2020Cl,
            "smpte2085" => MatrixCoefficients::Smpte2085,
            "chroma_derived_nc" => MatrixCoefficients::ChromaDerivedNc,
            "chroma_derived_cl" => MatrixCoefficients::ChromaDerivedCl,
            "ictcp" => MatrixCoefficients::Ictcp,
            other => return Err(format!("unknown matrix coefficients {other}")),
        })
    }
}

/// Dolby Vision profile tag. Stored as the integer profile number
/// (5, 7, 8, 9) in the `editions.dovi_profile` INTEGER column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum DolbyVisionProfile {
    Profile5,
    Profile7,
    Profile8,
    Profile9,
}

impl DolbyVisionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            DolbyVisionProfile::Profile5 => "profile5",
            DolbyVisionProfile::Profile7 => "profile7",
            DolbyVisionProfile::Profile8 => "profile8",
            DolbyVisionProfile::Profile9 => "profile9",
        }
    }

    /// Profile number as the integer the spec + storage column use.
    pub fn as_u8(self) -> u8 {
        match self {
            DolbyVisionProfile::Profile5 => 5,
            DolbyVisionProfile::Profile7 => 7,
            DolbyVisionProfile::Profile8 => 8,
            DolbyVisionProfile::Profile9 => 9,
        }
    }
}

impl FromStr for DolbyVisionProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "profile5" => DolbyVisionProfile::Profile5,
            "profile7" => DolbyVisionProfile::Profile7,
            "profile8" => DolbyVisionProfile::Profile8,
            "profile9" => DolbyVisionProfile::Profile9,
            other => return Err(format!("unknown Dolby Vision profile {other}")),
        })
    }
}

impl From<DolbyVisionProfile> for u8 {
    fn from(p: DolbyVisionProfile) -> Self {
        p.as_u8()
    }
}

impl TryFrom<u8> for DolbyVisionProfile {
    type Error = String;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Ok(match v {
            5 => DolbyVisionProfile::Profile5,
            7 => DolbyVisionProfile::Profile7,
            8 => DolbyVisionProfile::Profile8,
            9 => DolbyVisionProfile::Profile9,
            other => return Err(format!("unknown Dolby Vision profile {other}")),
        })
    }
}

/// Speaker / channel layout of an audio track. `ObjectBased` is the
/// JOC / Atmos case (bed+objects rather than a fixed channel count).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum ChannelLayout {
    Mono,
    Stereo,
    Three,
    Four,
    Five,
    Six,
    #[graphql(name = "SEVEN_ONE")]
    Seven1,
    Eight,
    ObjectBased,
}

impl ChannelLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            ChannelLayout::Mono => "mono",
            ChannelLayout::Stereo => "stereo",
            ChannelLayout::Three => "3.0",
            ChannelLayout::Four => "4.0",
            ChannelLayout::Five => "5.0",
            ChannelLayout::Six => "6.0",
            ChannelLayout::Seven1 => "7.1",
            ChannelLayout::Eight => "8.0",
            ChannelLayout::ObjectBased => "object_based",
        }
    }
}

impl FromStr for ChannelLayout {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "mono" => ChannelLayout::Mono,
            "stereo" => ChannelLayout::Stereo,
            "3.0" => ChannelLayout::Three,
            "4.0" => ChannelLayout::Four,
            "5.0" => ChannelLayout::Five,
            "6.0" => ChannelLayout::Six,
            "7.1" => ChannelLayout::Seven1,
            "8.0" => ChannelLayout::Eight,
            "object_based" => ChannelLayout::ObjectBased,
            other => return Err(format!("unknown channel layout {other}")),
        })
    }
}

/// Audio object coding. Drives the Atmos / JOC decision in
/// `noflex-transcoder::decide` — a client that doesn't accept
/// the object-based form of an Atmos track must transcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum AudioObjectType {
    /// None / unspecified. The track has a fixed channel layout
    /// and is not carrying bed+objects.
    None,
    /// Joint Object Coding (Dolby Digital Plus with Atmos).
    Joc,
    /// ADM (ITU-R BS.2125) — broadcast / streaming object audio.
    Adm,
    /// MPEG-H 3D Audio (Fraunhofer).
    MpegH,
}

impl AudioObjectType {
    pub fn as_str(self) -> &'static str {
        match self {
            AudioObjectType::None => "none",
            AudioObjectType::Joc => "joc",
            AudioObjectType::Adm => "adm",
            AudioObjectType::MpegH => "mpeg_h",
        }
    }
}

impl FromStr for AudioObjectType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "none" => AudioObjectType::None,
            "joc" => AudioObjectType::Joc,
            "adm" => AudioObjectType::Adm,
            "mpeg_h" => AudioObjectType::MpegH,
            other => return Err(format!("unknown audio object type {other}")),
        })
    }
}

/// Subtitle codec carried by a `SubtitleTrack`. Covers text subs
/// (WebVTT, TTML, SRT, ASS/SSA) and image-based subs (PGS, DVB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleCodec {
    WebVtt,
    Ttml,
    Srt,
    SubRip,
    Ass,
    Ssa,
    PgsImage,
    DvbSub,
    MovText,
}

impl SubtitleCodec {
    pub fn as_str(self) -> &'static str {
        match self {
            SubtitleCodec::WebVtt => "web_vtt",
            SubtitleCodec::Ttml => "ttml",
            SubtitleCodec::Srt => "srt",
            SubtitleCodec::SubRip => "sub_rip",
            SubtitleCodec::Ass => "ass",
            SubtitleCodec::Ssa => "ssa",
            SubtitleCodec::PgsImage => "pgs_image",
            SubtitleCodec::DvbSub => "dvb_sub",
            SubtitleCodec::MovText => "mov_text",
        }
    }
}

impl FromStr for SubtitleCodec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "web_vtt" => SubtitleCodec::WebVtt,
            "ttml" => SubtitleCodec::Ttml,
            "srt" => SubtitleCodec::Srt,
            "sub_rip" => SubtitleCodec::SubRip,
            "ass" => SubtitleCodec::Ass,
            "ssa" => SubtitleCodec::Ssa,
            "pgs_image" => SubtitleCodec::PgsImage,
            "dvb_sub" => SubtitleCodec::DvbSub,
            "mov_text" => SubtitleCodec::MovText,
            other => return Err(format!("unknown subtitle codec {other}")),
        })
    }
}
