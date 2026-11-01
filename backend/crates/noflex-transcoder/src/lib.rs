// SPDX-License-Identifier: AGPL-3.0-only
//
// ffmpeg subprocess job manager + HLS segment planner.
//
// This crate owns the *decision* and *control* surfaces of transcoding.
// The actual ffmpeg subprocess wiring is intentionally behind the
// `Transcoder` trait so the rest of the codebase can be tested with an
// in-memory mock and we don't pull ffmpeg into the unit-test loop.

#![forbid(unsafe_code)]
#![warn(clippy::all)]

use std::path::PathBuf;

use async_trait::async_trait;
use noflex_core::ids::MediaFileId;

pub mod decision;
pub mod encoder;
pub mod explain;
pub mod ffmpeg_argv;
pub mod hls;
pub mod hwdetect;
pub mod job;
pub mod probe;
pub mod reasons;
pub mod spawn;
pub mod subtitles;

pub use decision::{
    AudioCodecSupport, AudioPlan, ClientCapabilities, DecideOptions, DirectPlayProfile, HlsProfile,
    HlsSegmentContainer, PlaybackDecision, PlaybackRoute, TranscodePlan, VideoCodecSupport,
    VideoPlan, VideoRange, decide,
};
pub use encoder::{Encoder, HwBackend};
pub use explain::{
    PlaybackDecisionInfo, PlaybackPlanKind, PlaybackReason, PlaybackRouteKind, StreamSummary,
};
pub use ffmpeg_argv::{
    HDR_TONEMAP_FILTER, InputError, LadderEntry, NETWORK_INPUT_SCHEMES, NETWORK_PROTOCOL_WHITELIST,
    PlaylistType, SegmentConfig, SubtitleBurnSource, TranscodeInput, TranscodeRequest,
    build_ffmpeg_argv, is_network_url, scaled_bitrate_kbps,
};
pub use hls::EncodedVideo;
pub use hwdetect::HwDetect;
pub use job::{FfmpegJobManager, JobProgress, JobStatus, TranscodeJob, TranscodeJobRegistry};
pub use probe::{
    AudioStreamProbe, MasteringDisplay, ProbeError, ProbeReport, ProbeSideData,
    SubtitleStreamProbe, VideoStreamProbe, map_audio_codec, map_video_codec,
    map_video_profile_for_codec,
};
pub use reasons::{TranscodeReason, TranscodeReasons};
pub use spawn::{SpawnedJob, spawn};

#[async_trait]
pub trait Transcoder: Send + Sync {
    async fn start_job(
        &self,
        media: MediaFileId,
        request: TranscodeRequest,
    ) -> anyhow::Result<JobHandle>;
    async fn stop_job(&self, job: &JobHandle) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub struct JobHandle {
    pub id: String,
    pub output_dir: PathBuf,
}
