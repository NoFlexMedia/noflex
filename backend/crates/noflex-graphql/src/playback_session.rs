// SPDX-License-Identifier: AGPL-3.0-only
//
// Playback-session control plane. The GraphQL mutations in
// `playback.rs` are a thin shell over [`PlaybackSessionService`], the
// seam the server's stream runtime plugs into: the service owns probe
// reads, the direct-vs-transcode decision, HLS job lifecycle and the
// active-stream registry; the schema only shapes what it hands back.

use async_graphql::{ID, SimpleObject};
use noflex_core::{MediaFileId, PlayTarget, SessionId, SubtitleCodec, UserId, WorkId};
use noflex_storage::domain::{User, Work};
pub use noflex_transcoder::PlaybackRouteKind;
use noflex_transcoder::{ClientCapabilities, PlaybackDecisionInfo};
use time::OffsetDateTime;

pub use crate::playback::PlaybackState;

/// A text subtitle track the client renders itself, converted
/// server-side to a format it declared in `subtitleCodecs`.
#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
pub struct SubtitleDelivery {
    /// Index into the media file's `subtitleTracks`; the same index
    /// `burnSubtitle` takes.
    pub index: u32,
    pub language: Option<String>,
    pub title: Option<String>,
    pub forced: bool,
    pub default: bool,
    /// Format the `url` serves.
    pub codec: SubtitleCodec,
    /// Token-bearing, so `<track src>` works without headers.
    pub url: String,
}

/// What `startPlaybackSession` hands the client.
#[derive(SimpleObject, Debug, Clone)]
pub struct PlaybackSession {
    /// Handle for `stopPlaybackSession`; equals `jobId` on the
    /// transcode route.
    pub session_id: ID,
    /// Expiring grant to report only this playback's progress without a login
    /// credential. Absent for sessionless callers and live TV.
    pub progress_token: Option<String>,
    pub route: PlaybackRouteKind,
    /// Where to point the player.
    pub url: String,
    /// The whole file behind `url`, on the optimized route only, for a player
    /// that seeks in a file itself rather than through HLS.
    pub file_url: Option<String>,
    /// The HLS job, on the transcode route only.
    pub job_id: Option<ID>,
    /// Source position of the stream's time zero: the transcode's start
    /// offset, `0` for direct play. Player time + this = media time.
    pub start_position_ms: i64,
    pub decision: PlaybackDecisionInfo,
    /// Empty when the client declared no subtitle format the server
    /// can convert to, or a subtitle is burned in.
    pub subtitles: Vec<SubtitleDelivery>,
    /// What this file can be transcoded to for this client, best first: the
    /// steps a quality menu offers. Pinning one is asking again with its
    /// `bitrateBps` as `capabilities.maxBitrateBps`. Empty for a file with no
    /// video.
    pub qualities: Vec<StreamQuality>,
    /// Whether `url` is a ladder of every quality up to the one chosen, which
    /// the player's own adaptive bitrate moves along.
    pub adaptive: bool,
}

/// One step of the transcode ladder a session offers.
#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
pub struct StreamQuality {
    /// `4K`, `1440p`, `1080p`, `720p`, `480p` or `360p`: the 16:9 frame it fits.
    pub label: String,
    pub width: u32,
    pub height: u32,
    /// Video and audio together.
    pub bitrate_bps: u64,
}

/// Live state of a transcode job.
#[derive(SimpleObject, Debug, Clone, PartialEq)]
pub struct TranscodeProgress {
    pub job_id: ID,
    /// `running`, `completed`, `failed` or `cancelled`.
    pub status: String,
    /// ffmpeg video encoder, or `copy`.
    pub encoder: String,
    pub hardware: bool,
    pub fps: Option<f64>,
    /// Realtime multiple; below 1.0 the client will stall.
    pub speed: Option<f64>,
    /// Output timestamp reached, in milliseconds.
    pub out_time_ms: Option<i64>,
    pub error: Option<String>,
}

/// One active playback as the stream runtime tracks it. Identities are
/// resolved to `User` / `Work` at the GraphQL layer.
#[derive(Debug, Clone)]
pub struct ActiveStreamRecord {
    pub id: String,
    pub user_id: UserId,
    pub work_id: Option<WorkId>,
    /// Synthetic for live TV, which has no media file.
    pub media_file_id: MediaFileId,
    pub channel_id: Option<noflex_core::ChannelId>,
    pub device: Option<String>,
    pub started_at: OffsetDateTime,
    pub last_active_at: OffsetDateTime,
    pub position_ms: Option<i64>,
    pub decision: PlaybackDecisionInfo,
    /// `None` for direct play.
    pub transcode: Option<TranscodeProgress>,
}

/// An active playback, for the admin dashboard.
#[derive(SimpleObject, Debug, Clone)]
pub struct ActiveStream {
    /// Job id for transcodes; a generated id for direct sessions.
    pub id: ID,
    pub user: User,
    pub work: Option<Work>,
    pub media_file_id: ID,
    /// Set for live TV streams.
    pub channel_id: Option<ID>,
    /// Client/device name from the caller's auth session, if known.
    pub device: Option<String>,
    pub started_at: OffsetDateTime,
    /// Bumped by progress reports and segment fetches.
    pub last_active_at: OffsetDateTime,
    pub position_ms: Option<i64>,
    pub decision: PlaybackDecisionInfo,
    /// `None` for direct play.
    pub transcode: Option<TranscodeProgress>,
}

/// Identity and reporting context resolved from a playback progress grant.
#[derive(Debug, Clone)]
pub struct PlaybackProgress {
    pub user: UserId,
    pub target: PlayTarget,
    pub device: Option<String>,
}

/// See [`PlaybackSessionService::smart_download`].
#[derive(Debug, Clone)]
pub struct SmartDownloadFile {
    pub url: String,
    pub size_bytes: u64,
    pub optimized: bool,
    pub subtitles: Vec<SubtitleDelivery>,
}

/// What a client asks [`PlaybackSessionService::start`] for.
#[derive(Debug, Clone)]
pub struct PlaybackStart {
    pub media_file_id: MediaFileId,
    pub capabilities: ClientCapabilities,
    /// Index into the media file's subtitle tracks to burn in; forces a
    /// video re-encode.
    pub burn_subtitle: Option<u32>,
    /// Client/device label for the active-streams view.
    pub device: Option<String>,
    /// Start a transcode at this source position (direct play ignores
    /// it; the client seeks).
    pub start_position_ms: Option<i64>,
    /// Index into the media file's audio streams to play instead of its
    /// default.
    pub audio_stream: Option<u32>,
    /// Transcode to the whole ladder up to the chosen quality rather than to
    /// that quality alone. The original still plays when the client can play it.
    pub adaptive: bool,
}

impl PlaybackStart {
    /// A plain request: no burn-in, device label, or start offset.
    pub fn new(media_file_id: MediaFileId, capabilities: ClientCapabilities) -> Self {
        Self {
            media_file_id,
            capabilities,
            burn_subtitle: None,
            device: None,
            start_position_ms: None,
            audio_stream: None,
            adaptive: false,
        }
    }
}

/// Server-side session lifecycle, implemented by the stream runtime in
/// `noflex-server` and wired into [`crate::SchemaContext`]. Behind a
/// trait so the schema crate never touches ffmpeg or the job registry
/// directly — and so tests can stub the whole plane.
#[async_trait::async_trait]
pub trait PlaybackSessionService: Send + Sync {
    /// Decide direct vs. transcode for `request` against the client's
    /// capabilities, start whatever the route needs (nothing for direct,
    /// an HLS job for transcode) and register the session for `user`.
    /// `login` is the caller's session, which the returned URLs' media
    /// tokens are bound to. Fails for unknown or inaccessible media, a
    /// bad subtitle index, or an exhausted transcode quota.
    async fn start(
        &self,
        user: UserId,
        login: Option<SessionId>,
        request: PlaybackStart,
    ) -> anyhow::Result<PlaybackSession>;

    /// End a session by the `session_id` `start` returned, stopping its
    /// job if any. Fails when the session isn't `user`'s; stopping an
    /// already-finished transcode succeeds.
    async fn stop(&self, user: UserId, session_id: &str) -> anyhow::Result<()>;

    /// A progress report from `user` for `target`: keeps their matching
    /// sessions alive, tells scrobblers when one turns between playing and
    /// paused, and `Stopped` ends their direct sessions. Never fails the
    /// report.
    async fn heartbeat(
        &self,
        user: UserId,
        target: PlayTarget,
        position_ms: i64,
        state: PlaybackState,
    ) {
        let _ = (user, target, position_ms, state);
    }

    /// Validate a narrow progress grant against its live login, playback and
    /// current media access, then heartbeat only that playback. `Stopped`
    /// cleans up that session's job, if any. Never accepts a general credential.
    async fn report_granted_progress(
        &self,
        token: &str,
        position_ms: i64,
        state: PlaybackState,
    ) -> anyhow::Result<PlaybackProgress> {
        let _ = (token, position_ms, state);
        anyhow::bail!("playback progress grants unavailable")
    }

    /// Every session still active, idle ones pruned first.
    async fn active_streams(&self) -> anyhow::Result<Vec<ActiveStreamRecord>>;

    /// What a device with `capabilities` should download for `media`: the file
    /// itself when it plays directly, otherwise a ready optimized version it can
    /// play. `None` when neither exists yet. The URL's media token is bound to
    /// `login`, as for playback.
    async fn smart_download(
        &self,
        user: UserId,
        login: Option<SessionId>,
        media: &noflex_storage::domain::MediaFile,
        capabilities: &ClientCapabilities,
    ) -> anyhow::Result<Option<SmartDownloadFile>> {
        let _ = (user, login, media, capabilities);
        Ok(None)
    }
}
