// SPDX-License-Identifier: AGPL-3.0-only

use async_graphql::Enum;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Why a session could not direct play, and why each stream the
    /// HLS route re-encodes could not be copied. Names follow
    /// Jellyfin's `TranscodeReason` so admins can read them across
    /// servers.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct TranscodeReasons: u32 {
        const ContainerNotSupported        = 1 << 0;
        const VideoCodecNotSupported       = 1 << 1;
        const VideoProfileNotSupported     = 1 << 2;
        const VideoLevelNotSupported       = 1 << 3;
        const VideoBitDepthNotSupported    = 1 << 4;
        const PixelFormatNotSupported      = 1 << 5;
        const VideoResolutionNotSupported  = 1 << 6;
        const VideoRangeTypeNotSupported   = 1 << 7;
        const AudioCodecNotSupported       = 1 << 8;
        const AudioChannelsNotSupported    = 1 << 9;
        const ContainerBitrateExceedsLimit = 1 << 10;
        const SubtitleBurnIn               = 1 << 11;
        const LiveSource                   = 1 << 12;
        /// The client asked for an audio track other than the one a browser
        /// plays by default, which progressive playback can't switch to.
        const AudioTrackChosen             = 1 << 13;

        /// Reasons that force the video stream to be re-encoded.
        const VIDEO = Self::VideoCodecNotSupported.bits()
            | Self::VideoProfileNotSupported.bits()
            | Self::VideoLevelNotSupported.bits()
            | Self::VideoBitDepthNotSupported.bits()
            | Self::PixelFormatNotSupported.bits()
            | Self::VideoResolutionNotSupported.bits()
            | Self::VideoRangeTypeNotSupported.bits()
            | Self::ContainerBitrateExceedsLimit.bits()
            | Self::SubtitleBurnIn.bits();

        /// Reasons that force the audio stream to be re-encoded.
        const AUDIO = Self::AudioCodecNotSupported.bits()
            | Self::AudioChannelsNotSupported.bits();
    }
}

/// One [`TranscodeReasons`] bit as a GraphQL enum value.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TranscodeReason {
    ContainerNotSupported,
    VideoCodecNotSupported,
    VideoProfileNotSupported,
    VideoLevelNotSupported,
    VideoBitDepthNotSupported,
    PixelFormatNotSupported,
    VideoResolutionNotSupported,
    VideoRangeTypeNotSupported,
    AudioCodecNotSupported,
    AudioChannelsNotSupported,
    ContainerBitrateExceedsLimit,
    SubtitleBurnIn,
    LiveSource,
    AudioTrackChosen,
}

/// Bit order is the order `codes()`/`describe()` report in.
const TABLE: &[(TranscodeReasons, TranscodeReason, &str)] = &[
    (
        TranscodeReasons::ContainerNotSupported,
        TranscodeReason::ContainerNotSupported,
        "container not supported for direct play",
    ),
    (
        TranscodeReasons::VideoCodecNotSupported,
        TranscodeReason::VideoCodecNotSupported,
        "video codec not supported",
    ),
    (
        TranscodeReasons::VideoProfileNotSupported,
        TranscodeReason::VideoProfileNotSupported,
        "video profile not supported",
    ),
    (
        TranscodeReasons::VideoLevelNotSupported,
        TranscodeReason::VideoLevelNotSupported,
        "video level too high",
    ),
    (
        TranscodeReasons::VideoBitDepthNotSupported,
        TranscodeReason::VideoBitDepthNotSupported,
        "video bit depth not supported",
    ),
    (
        TranscodeReasons::PixelFormatNotSupported,
        TranscodeReason::PixelFormatNotSupported,
        "chroma subsampling not supported",
    ),
    (
        TranscodeReasons::VideoResolutionNotSupported,
        TranscodeReason::VideoResolutionNotSupported,
        "resolution exceeds client max",
    ),
    (
        TranscodeReasons::VideoRangeTypeNotSupported,
        TranscodeReason::VideoRangeTypeNotSupported,
        "HDR / Dolby Vision range not supported",
    ),
    (
        TranscodeReasons::AudioCodecNotSupported,
        TranscodeReason::AudioCodecNotSupported,
        "audio codec not supported",
    ),
    (
        TranscodeReasons::AudioChannelsNotSupported,
        TranscodeReason::AudioChannelsNotSupported,
        "audio channel count not supported",
    ),
    (
        TranscodeReasons::ContainerBitrateExceedsLimit,
        TranscodeReason::ContainerBitrateExceedsLimit,
        "bitrate exceeds client max",
    ),
    (
        TranscodeReasons::SubtitleBurnIn,
        TranscodeReason::SubtitleBurnIn,
        "subtitle burn-in requested",
    ),
    (
        TranscodeReasons::LiveSource,
        TranscodeReason::LiveSource,
        "live sources are always transcoded",
    ),
    (
        TranscodeReasons::AudioTrackChosen,
        TranscodeReason::AudioTrackChosen,
        "an audio track other than the default was chosen",
    ),
];

impl TranscodeReason {
    pub fn description(self) -> &'static str {
        TABLE
            .iter()
            .find(|(_, code, _)| *code == self)
            .map_or("", |(_, _, text)| text)
    }
}

impl TranscodeReasons {
    /// Set bits as enum codes, lowest bit first.
    pub fn codes(&self) -> Vec<TranscodeReason> {
        TABLE
            .iter()
            .filter(|(bit, _, _)| self.contains(*bit))
            .map(|(_, code, _)| *code)
            .collect()
    }

    /// Human-readable text per set bit, lowest bit first.
    pub fn describe(&self) -> Vec<&'static str> {
        TABLE
            .iter()
            .filter(|(bit, _, _)| self.contains(*bit))
            .map(|(_, _, text)| *text)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_describes_to_nothing() {
        let r = TranscodeReasons::empty();
        assert!(r.describe().is_empty());
        assert!(r.codes().is_empty());
    }

    #[test]
    fn every_bit_has_a_table_row() {
        let all = TranscodeReasons::all();
        assert_eq!(all.codes().len(), TABLE.len());
        for (bit, code, text) in TABLE {
            assert_eq!(bit.codes(), vec![*code]);
            assert_eq!(code.description(), *text);
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn describe_reports_in_bit_order() {
        let r = TranscodeReasons::AudioCodecNotSupported | TranscodeReasons::ContainerNotSupported;
        assert_eq!(
            r.describe(),
            vec![
                "container not supported for direct play",
                "audio codec not supported"
            ]
        );
        assert_eq!(
            r.codes(),
            vec![
                TranscodeReason::ContainerNotSupported,
                TranscodeReason::AudioCodecNotSupported
            ]
        );
    }

    #[test]
    fn video_and_audio_groups_are_disjoint() {
        assert!(
            TranscodeReasons::VIDEO
                .intersection(TranscodeReasons::AUDIO)
                .is_empty()
        );
        assert!(!TranscodeReasons::VIDEO.contains(TranscodeReasons::ContainerNotSupported));
    }

    #[test]
    fn round_trips_through_serde() {
        let r = TranscodeReasons::all();
        let json = serde_json::to_string(&r).unwrap();
        let back: TranscodeReasons = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
