// SPDX-License-Identifier: AGPL-3.0-only
//
// Streaming covers direct play, HLS job serving, and the playback-session
// control plane. The session plane lives behind
// [`ServerPlaybackSessions`], wired into the GraphQL
// `startPlaybackSession`/`stopPlaybackSession` mutations; the byte-serving
// functions are wrapped by thin axum handlers in `main.rs` that only do
// auth and path parsing. The integration tests in `tests/it/stream.rs` call
// them directly, mirroring how `download_serve` is structured.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_graphql::ID;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use noflex_auth::{JwtConfig, Principal, issue_media_token, verify_media_token};
use noflex_core::{
    AudioCodec, ChannelId, DomainEvent, DomainEventBus, EditionId, MediaFileId, PlayTarget,
    SessionId, SubtitleCodec, UserId, VideoCodec, WorkId,
};
use noflex_graphql::{
    ActiveStreamRecord, PlaybackRouteKind, PlaybackSession, PlaybackSessionService, PlaybackStart,
    PlaybackState, SmartDownloadFile, StreamQuality, SubtitleDelivery, TranscodeProgress,
};
use noflex_plugin_host::SubtitleService;
use noflex_storage::{
    Storage,
    domain::{MediaFile, OptimizedVersion, OptimizedVersionStatus, SubtitleTrack},
};
use noflex_transcoder::{
    AudioPlan, ClientCapabilities, DecideOptions, EncodedVideo, Encoder, FetchError, Fetched,
    FfmpegJobManager, HlsSegmentContainer, HwBackend, JobHandle, JobStatus, LadderEntry,
    PlaybackDecision, PlaybackDecisionInfo, PlaybackRoute, PlaylistType, SegmentConfig,
    SegmentGrid, SegmentedRequest, SubtitleBurnSource, TonemapOptions, TranscodeInput,
    TranscodeJobRegistry, TranscodePlan, TranscodeReasons, TranscodeRequest, Transcoder, VideoPlan,
    decide, fit_pixels,
    hls::HlsVariant,
    optimize::version_plays,
    probe::ProbeReport,
    scaled_bitrate_kbps,
    segmented::{audio_for_runs, variant_playlist},
    subtitles,
};
use time::OffsetDateTime;
use tokio::io::AsyncReadExt;

use crate::{
    config::TranscoderConfig,
    file_serve::{OpenedFile, RootCapability, ServeOpts, open_from_roots, serve_file},
};

// Header-less players fetch the minted URLs directly, so the TTL spans a
// long viewing session.
const MEDIA_TOKEN_TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// Downloads run in the background of a device that may wait days for Wi-Fi,
/// and their retries reuse the URL they were given. The token still dies with
/// the login it's bound to.
const DOWNLOAD_TOKEN_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// `?token=` on byte routes: a scoped media token standing in for a
/// session credential on players that can't set headers.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenQuery {
    pub token: Option<String>,
}

/// Resolve the caller of a byte route. A header-authenticated principal
/// wins; otherwise `?token=` must verify, name `target`, its `sub` must
/// still be a user, and its session (if bound to one) must still be live
/// and theirs; that user becomes the principal with their current admin
/// flag. The token replaces the header credential, not
/// authorization: callers must still run the returned principal's
/// access checks.
///
/// Also returns the verified token (only when it belongs to the
/// returned principal) so playlist handlers can propagate it to segment
/// URIs. Fails with `401` when neither credential is usable.
pub async fn byte_route_principal(
    storage: &dyn Storage,
    principal: &Principal,
    jwt: &JwtConfig,
    token: Option<&str>,
    target: &str,
) -> Result<(Principal, Option<String>), StatusCode> {
    let verified = token.and_then(|raw| {
        verify_media_token(jwt, raw)
            .filter(|claims| claims.target == target)
            .map(|claims| (raw.to_string(), claims))
    });
    if principal.is_authenticated() {
        let token = verified
            .filter(|(_, claims)| claims.user == principal.user_id)
            .map(|(raw, _)| raw);
        return Ok((*principal, token));
    }
    let (raw, claims) = verified.ok_or(StatusCode::UNAUTHORIZED)?;
    if let Some(session) = claims.session {
        let live = storage
            .get_session(session)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_some_and(|row| {
                row.user_id == claims.user
                    && row.expires_at > OffsetDateTime::now_utc()
                    && row.locked_at.is_none()
            });
        if !live {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    let user = storage
        .get_user(claims.user)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok((
        Principal {
            user_id: claims.user,
            is_admin: user.is_admin,
            session_id: claims.session,
        },
        Some(raw),
    ))
}

fn request_user(principal: &Principal) -> Result<noflex_storage::RequestUser, StatusCode> {
    if !principal.is_authenticated() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(noflex_storage::RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    })
}

fn visible(result: noflex_storage::StorageResult<bool>) -> Result<(), StatusCode> {
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// `401` for an anonymous principal, `404` when the media file is
/// missing or outside the principal's access (admins see everything).
pub async fn media_access(
    storage: &dyn Storage,
    principal: &Principal,
    media: MediaFileId,
) -> Result<(), StatusCode> {
    let user = request_user(principal)?;
    visible(noflex_storage::access::user_can_access_media_file(storage, user, media).await)
}

/// As [`media_access`], for a work.
pub async fn work_access(
    storage: &dyn Storage,
    principal: &Principal,
    work: WorkId,
) -> Result<(), StatusCode> {
    let user = request_user(principal)?;
    visible(noflex_storage::access::user_can_access_work(storage, user, work).await)
}

/// As [`media_access`], for an edition.
pub async fn edition_access(
    storage: &dyn Storage,
    principal: &Principal,
    edition: EditionId,
) -> Result<(), StatusCode> {
    let user = request_user(principal)?;
    visible(noflex_storage::access::user_can_access_edition(storage, user, edition).await)
}

/// Production [`MediaUrlSigner`](noflex_storage::MediaUrlSigner):
/// trickplay tokens live [`MEDIA_TOKEN_TTL`]; image tokens are
/// hour-bucketed so image URLs stay cacheable.
pub struct JwtMediaUrlSigner {
    jwt: Arc<JwtConfig>,
}

impl JwtMediaUrlSigner {
    pub fn new(jwt: Arc<JwtConfig>) -> Self {
        Self { jwt }
    }
}

impl noflex_storage::MediaUrlSigner for JwtMediaUrlSigner {
    fn sign_trickplay_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        edition_id: EditionId,
    ) -> Result<String, String> {
        issue_media_token(
            &self.jwt,
            user,
            session,
            &format!("trickplay:{edition_id}"),
            MEDIA_TOKEN_TTL,
        )
        .map_err(|error| error.to_string())
    }

    fn sign_image_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        target: noflex_storage::ImageTarget,
    ) -> Result<String, String> {
        noflex_auth::issue_bucketed_media_token(
            &self.jwt,
            user,
            session,
            &image_token_target(target),
            time::OffsetDateTime::now_utc(),
        )
        .map_err(|error| error.to_string())
    }

    fn sign_channel_logo_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        channel: ChannelId,
    ) -> Result<String, String> {
        noflex_auth::issue_bucketed_media_token(
            &self.jwt,
            user,
            session,
            &channel_logo_token_target(&channel.to_string()),
            time::OffsetDateTime::now_utc(),
        )
        .map_err(|error| error.to_string())
    }

    fn sign_avatar_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        avatar_of: UserId,
    ) -> Result<String, String> {
        noflex_auth::issue_bucketed_media_token(
            &self.jwt,
            user,
            session,
            &avatar_token_target(&avatar_of.to_string()),
            time::OffsetDateTime::now_utc(),
        )
        .map_err(|error| error.to_string())
    }

    fn sign_artwork_candidate_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        candidate: &str,
    ) -> Result<String, String> {
        noflex_auth::issue_bucketed_media_token(
            &self.jwt,
            user,
            session,
            &artwork_candidate_token_target(candidate),
            time::OffsetDateTime::now_utc(),
        )
        .map_err(|error| error.to_string())
    }
}

/// Token target for an offered artwork candidate's preview.
pub fn artwork_candidate_token_target(candidate: &str) -> String {
    format!("img:candidate:{candidate}")
}

/// Token target for a user's picture.
pub fn avatar_token_target(user: &str) -> String {
    format!("img:avatar:{user}")
}

/// Token target for a Live TV channel's proxied logo.
pub fn channel_logo_token_target(channel: &str) -> String {
    format!("img:channel:{channel}")
}

/// Token target naming every image of `target`.
pub fn image_token_target(target: noflex_storage::ImageTarget) -> String {
    format!("img:{}:{}", target.kind_str(), target.id())
}

/// Append `?token=` to every URI of an m3u8 body: URI lines and the
/// `URI="…"` attribute of tags (the fMP4 init segment's `EXT-X-MAP`).
/// hls.js resolves relative URIs against the playlist URL and drops the
/// query string, so the token on `master.m3u8` never reaches segment
/// requests without this rewrite.
pub fn propagate_token_to_playlist(body: &str, token: &str) -> String {
    let with_token = |uri: &str| {
        let sep = if uri.contains('?') { '&' } else { '?' };
        format!("{uri}{sep}token={token}")
    };
    body.lines()
        .map(|line| {
            let bare = line.strip_suffix('\r').unwrap_or(line);
            if bare.is_empty() {
                line.to_string()
            } else if bare.starts_with('#') {
                match bare.find("URI=\"") {
                    Some(at) => {
                        let start = at + "URI=\"".len();
                        match bare[start..].find('"') {
                            Some(len) => format!(
                                "{}{}{}",
                                &bare[..start],
                                with_token(&bare[start..start + len]),
                                &bare[start + len..]
                            ),
                            None => line.to_string(),
                        }
                    }
                    None => line.to_string(),
                }
            } else {
                with_token(bare)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn open_library_file(
    storage: &Arc<dyn Storage>,
    media: MediaFileId,
    path: &str,
) -> Result<OpenedFile, StatusCode> {
    let work = storage
        .work_for_media_file(media)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let library = storage
        .get_library(work.library_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    open_from_roots(&library.paths, path)
        .await
        .map_err(|error| {
            tracing::warn!(%error, %media, path, "rejected library file");
            StatusCode::NOT_FOUND
        })
}

/// Direct play: stream the media file itself, with byte-range
/// support so players can seek without a transcode.
pub async fn serve_media_file(
    storage: &Arc<dyn Storage>,
    id: MediaFileId,
    range_header: Option<&str>,
) -> Response {
    let media = match storage.media_file_by_id(id).await {
        Ok(Some(m)) => m,
        Ok(None) => return (StatusCode::NOT_FOUND, "media file not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "storage error reading media file");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };

    let opened = match open_library_file(storage, media.id, &media.path).await {
        Ok(opened) => opened,
        Err(status) => return status.into_response(),
    };

    serve_file(opened, range_header, ServeOpts::default()).await
}

/// Serve subtitle track `index` of a media file as WebVTT. Sidecars are
/// converted in-process; muxed-in text tracks are extracted with ffmpeg
/// once and cached under `cache_dir`, keyed by the file's size and mtime
/// so a replaced file never serves stale cues. Downloaded tracks are read
/// from `downloads`' cache, which counts as a use; without it they're `404`.
/// Bitmap tracks are `404` (they can only be burned in).
pub async fn serve_subtitle(
    storage: &Arc<dyn Storage>,
    ffmpeg: &Path,
    cache_dir: &Path,
    downloads: Option<&SubtitleService>,
    id: MediaFileId,
    index: usize,
) -> Response {
    let media = match storage.media_file_by_id(id).await {
        Ok(Some(m)) => m,
        Ok(None) => return (StatusCode::NOT_FOUND, "media file not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "storage error reading media file");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };
    let tracks = match subtitle_tracks(storage.as_ref(), &media).await {
        Ok(tracks) => tracks,
        Err(e) => {
            tracing::error!(error = %e, "storage error reading downloaded subtitles");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };

    let Some(track) = tracks.get(index) else {
        return (StatusCode::NOT_FOUND, "subtitle track not found").into_response();
    };
    let vtt = match (track.path.as_deref(), track.stream_index) {
        (None, None) if track.downloaded.is_some() => {
            let (Some(downloaded), Some(downloads)) = (&track.downloaded, downloads) else {
                return (StatusCode::NOT_FOUND, "subtitle track not found").into_response();
            };
            let text = match tokio::fs::read(downloads.path(downloaded)).await {
                Ok(bytes) => subtitles::decode_text(&bytes),
                Err(error) => {
                    tracing::warn!(%error, subtitle = %downloaded.id, "downloaded subtitle unreadable");
                    return (StatusCode::NOT_FOUND, "subtitle track not found").into_response();
                }
            };
            downloads.record_use(downloaded).await;
            match subtitles::to_webvtt(downloaded.format, &text) {
                Some(vtt) => vtt,
                None => {
                    return (StatusCode::NOT_FOUND, "subtitle track not convertible")
                        .into_response();
                }
            }
        }
        (Some(path), _) => {
            let Some(codec) =
                sidecar_codec(&track.codec).filter(|c| subtitles::converts_to_webvtt(*c))
            else {
                return (StatusCode::NOT_FOUND, "subtitle track not convertible").into_response();
            };
            let opened = match open_library_file(storage, media.id, path).await {
                Ok(opened) => opened,
                Err(status) => return status.into_response(),
            };
            let mut bytes = Vec::new();
            if let Err(error) = opened.into_file().read_to_end(&mut bytes).await {
                tracing::error!(%error, path, "read subtitle failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response();
            }
            match subtitles::to_webvtt(codec, &subtitles::decode_text(&bytes)) {
                Some(vtt) => vtt,
                None => {
                    return (StatusCode::NOT_FOUND, "subtitle track not convertible")
                        .into_response();
                }
            }
        }
        (None, Some(stream)) => {
            if !sidecar_codec(&track.codec).is_some_and(subtitles::extracts_to_webvtt) {
                return (StatusCode::NOT_FOUND, "subtitle track not convertible").into_response();
            }
            // Validates the media path against the library roots before
            // ffmpeg is pointed at it.
            if let Err(status) = open_library_file(storage, media.id, &media.path).await {
                return status.into_response();
            }
            match extract_embedded_vtt(ffmpeg, cache_dir, &media, stream).await {
                Ok(vtt) => vtt,
                Err(error) => {
                    tracing::warn!(%error, media = %media.id, stream, "subtitle extraction failed");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "subtitle extraction failed",
                    )
                        .into_response();
                }
            }
        }
        (None, None) => {
            return (StatusCode::NOT_FOUND, "subtitle track not found").into_response();
        }
    };
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/vtt; charset=utf-8")],
        vtt,
    )
        .into_response()
}

/// Extraction reads the whole container, so a large remux can take a
/// while; past this the request fails rather than pinning a process.
const SUBTITLE_EXTRACT_TIMEOUT: Duration = Duration::from_secs(180);

async fn extract_embedded_vtt(
    ffmpeg: &Path,
    cache_dir: &Path,
    media: &MediaFile,
    stream: u32,
) -> anyhow::Result<String> {
    let dir = cache_dir.join("subtitles");
    let cached = dir.join(format!(
        "{}-{stream}-{}-{}.vtt",
        media.id,
        media.size_bytes,
        media.mtime.unix_timestamp()
    ));
    if let Ok(vtt) = tokio::fs::read_to_string(&cached).await {
        return Ok(vtt);
    }
    let input = TranscodeInput::File(PathBuf::from(&media.path));
    let output = tokio::time::timeout(
        SUBTITLE_EXTRACT_TIMEOUT,
        tokio::process::Command::new(ffmpeg)
            .args(subtitles::extract_webvtt_argv(&input, stream))
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out after {SUBTITLE_EXTRACT_TIMEOUT:?}"))??;
    if !output.status.success() {
        anyhow::bail!(
            "ffmpeg exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let vtt = String::from_utf8_lossy(&output.stdout).into_owned();
    // Write-then-rename so a concurrent reader never sees a partial file;
    // a failed cache write only costs a re-extraction next time.
    if tokio::fs::create_dir_all(&dir).await.is_ok() {
        let partial = cached.with_extension(format!("{}.part", uuid::Uuid::now_v7()));
        if tokio::fs::write(&partial, &vtt).await.is_ok()
            && tokio::fs::rename(&partial, &cached).await.is_err()
        {
            let _ = tokio::fs::remove_file(&partial).await;
        }
    }
    Ok(vtt)
}

/// The scanner records sidecar codecs as `SubtitleCodec::as_str`; `srt`
/// and `vtt` are accepted as the extension spellings.
/// Every subtitle track of `media` as `subtitleTracks` lists them, so the
/// indexes clients send mean the same here.
async fn subtitle_tracks(
    storage: &dyn Storage,
    media: &MediaFile,
) -> noflex_storage::StorageResult<Vec<SubtitleTrack>> {
    let downloaded = storage.downloaded_subtitles_for_media(media.id).await?;
    Ok(media.subtitle_tracks_with(&downloaded))
}

fn sidecar_codec(raw: &str) -> Option<SubtitleCodec> {
    match raw {
        "srt" => Some(SubtitleCodec::SubRip),
        "vtt" | "webvtt" => Some(SubtitleCodec::WebVtt),
        other => other.parse().ok(),
    }
}

/// Direct sessions have no server process to watch: one that hasn't
/// reported progress in this long is treated as gone. The player
/// heartbeats every 15s.
const DIRECT_SESSION_IDLE: time::Duration = time::Duration::minutes(3);

/// A finished VOD transcode keeps its session while the client is
/// still fetching segments from it.
const FINISHED_TRANSCODE_IDLE: time::Duration = time::Duration::minutes(5);

/// An on-demand session runs until stopped; one nobody has fetched from in
/// this long was left paused or abandoned, and goes with its segments.
const ON_DEMAND_IDLE: time::Duration = time::Duration::minutes(30);

/// One registered playback, keyed by session id.
#[derive(Debug, Clone)]
struct ActiveSession {
    user: UserId,
    work: Option<WorkId>,
    edition: Option<EditionId>,
    media_file: MediaFileId,
    channel: Option<ChannelId>,
    device: Option<String>,
    started_at: OffsetDateTime,
    last_active_at: OffsetDateTime,
    position_ms: Option<i64>,
    /// What the last progress report said, so a pause is told once.
    paused: bool,
    decision: PlaybackDecisionInfo,
    /// Equal to the session id when set.
    job: Option<String>,
}

/// Playback-session control plane behind the GraphQL session mutations
/// and the admin `activeStreams` query. Owns the direct-vs-HLS
/// decision, the HLS job lifecycle and the registry of active sessions;
/// returned URLs carry a scoped media token so the byte routes stay
/// authenticated for players that can't set headers.
pub struct ServerPlaybackSessions {
    storage: Arc<dyn Storage>,
    transcoder: Arc<FfmpegJobManager>,
    cfg: Arc<TranscoderConfig>,
    /// Codecs to encode to live, preferred first.
    encode_video: Vec<VideoCodec>,
    /// Tone-map HDR on the GPU; see [`noflex_transcoder::hwdetect::gpu_tonemap_works`].
    gpu_tonemap: bool,
    /// Startup-resolved hardware family.
    backend: HwBackend,
    /// Detected VAAPI render node, threaded into the filter graph.
    vaapi_device: Option<PathBuf>,
    jwt: Arc<JwtConfig>,
    events: Option<DomainEventBus>,
    /// Downloaded subtitles, to burn in; none without it.
    subtitles: Option<Arc<SubtitleService>>,
    transcode_accounting: tokio::sync::Mutex<TranscodeAccounting>,
    active: tokio::sync::Mutex<HashMap<String, ActiveSession>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackJobTarget {
    Work(WorkId),
    LiveChannel(ChannelId),
}

#[derive(Debug, Clone, Copy)]
struct PlaybackJobAccess {
    owner: UserId,
    target: PlaybackJobTarget,
}

#[derive(Default)]
struct TranscodeAccounting {
    jobs: HashMap<String, PlaybackJobAccess>,
    reserved: HashMap<UserId, u32>,
}

#[async_trait::async_trait]
pub trait PlaybackJobAuthorizer: Send + Sync {
    async fn authorize_job(&self, principal: &Principal, job_id: &str) -> Result<(), StatusCode>;
}

impl ServerPlaybackSessions {
    /// A ready version of `media` that `capabilities` can play without any
    /// re-encoding. A version of an older copy of the file doesn't count.
    async fn playable_version(
        &self,
        media: &MediaFile,
        capabilities: &ClientCapabilities,
    ) -> anyhow::Result<Option<OptimizedVersion>> {
        let mut playable = self
            .storage
            .optimized_versions_for_media(media.id)
            .await?
            .into_iter()
            .filter(|version| {
                version.status == OptimizedVersionStatus::Ready
                    && version.source_size == media.size_bytes
                    && version.source_mtime == media.mtime
                    && version
                        .probe
                        .clone()
                        .and_then(|probe| serde_json::from_value::<ProbeReport>(probe).ok())
                        .is_some_and(|probe| version_plays(&probe, capabilities))
            })
            .collect::<Vec<_>>();
        // The largest is the highest quality this client can play.
        playable.sort_by_key(|version| std::cmp::Reverse(version.size_bytes));
        Ok(playable.into_iter().next())
    }

    pub fn new(
        storage: Arc<dyn Storage>,
        transcoder: Arc<FfmpegJobManager>,
        cfg: Arc<TranscoderConfig>,
        backend: HwBackend,
        vaapi_device: Option<PathBuf>,
        jwt: Arc<JwtConfig>,
    ) -> Self {
        // Until told what actually works here: H.264 first, the only encoder
        // sure to keep up, then whatever else the ladder offers.
        let mut encode_video = vec![VideoCodec::H264];
        for rung in &cfg.ladder {
            if !encode_video.contains(&rung.video_codec) {
                encode_video.push(rung.video_codec);
            }
        }
        Self {
            storage,
            transcoder,
            cfg,
            backend,
            vaapi_device,
            jwt,
            encode_video,
            gpu_tonemap: false,
            events: None,
            subtitles: None,
            transcode_accounting: tokio::sync::Mutex::new(TranscodeAccounting::default()),
            active: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Codecs to encode to live, preferred first; see
    /// [`crate::hwaccel::live_codec_order`].
    pub fn with_encode_video(mut self, encode_video: Vec<VideoCodec>) -> Self {
        self.encode_video = encode_video;
        self
    }

    /// Tone-map HDR on the GPU where the encoder allows it.
    pub fn with_gpu_tonemap(mut self, gpu_tonemap: bool) -> Self {
        self.gpu_tonemap = gpu_tonemap;
        self
    }

    /// Domain-event sink for session start, pause, resume and stop. Optional
    /// so tests and lean embeddings stay silent.
    pub fn with_events(mut self, events: DomainEventBus) -> Self {
        self.events = Some(events);
        self
    }

    /// Where downloaded subtitles are, so they're delivered and burned in as
    /// sidecars are. Without it they're listed but can't be played.
    pub fn with_subtitles(mut self, subtitles: Arc<SubtitleService>) -> Self {
        self.subtitles = Some(subtitles);
        self
    }

    fn media_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        target: &str,
    ) -> anyhow::Result<String> {
        issue_media_token(&self.jwt, user, session, target, MEDIA_TOKEN_TTL)
            .map_err(|e| anyhow::anyhow!("mint media token: {e}"))
    }

    fn download_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        target: &str,
    ) -> anyhow::Result<String> {
        issue_media_token(&self.jwt, user, session, target, DOWNLOAD_TOKEN_TTL)
            .map_err(|e| anyhow::anyhow!("mint download token: {e}"))
    }

    /// Resolve edition → work for event payloads. Best-effort: the
    /// session itself already succeeded, so a dangling row downgrades
    /// to a warn and a skipped event.
    async fn resolve_event_work(&self, edition: EditionId) -> Option<(WorkId, String)> {
        let edition = match self.storage.get_edition(edition).await {
            Ok(Some(e)) => e,
            Ok(None) => {
                tracing::warn!("event: edition not found; skipping");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "event: edition lookup failed; skipping");
                return None;
            }
        };
        match self.storage.get_work(edition.work_id).await {
            Ok(Some(work)) => Some((work.id, work.title)),
            Ok(None) => {
                tracing::warn!("event: work not found; skipping");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "event: work lookup failed; skipping");
                None
            }
        }
    }

    async fn emit_stopped(&self, user: UserId, edition: EditionId, media_file: MediaFileId) {
        if let Some(bus) = &self.events
            && let Some((work, title)) = self.resolve_event_work(edition).await
        {
            bus.emit(DomainEvent::PlaybackStopped {
                user,
                work,
                title,
                media_file,
            });
        }
    }

    async fn reserve_transcode(&self, user: UserId, limit: Option<u32>) -> anyhow::Result<()> {
        let mut accounting = self.transcode_accounting.lock().await;
        let mut running = 0_u32;
        for (id, access) in &accounting.jobs {
            if access.owner == user
                && self
                    .transcoder
                    .registry()
                    .get(id)
                    .await
                    .is_some_and(|job| job.status == JobStatus::Running)
            {
                running += 1;
            }
        }
        let reserved = accounting.reserved.get(&user).copied().unwrap_or(0);
        if limit.is_some_and(|limit| running.saturating_add(reserved) >= limit) {
            anyhow::bail!("concurrent transcode limit reached");
        }
        *accounting.reserved.entry(user).or_default() += 1;
        Ok(())
    }

    async fn finish_reservation(&self, user: UserId, job: Option<(String, PlaybackJobTarget)>) {
        let mut accounting = self.transcode_accounting.lock().await;
        if let Some(count) = accounting.reserved.get_mut(&user) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                accounting.reserved.remove(&user);
            }
        }
        if let Some((job, target)) = job {
            accounting.jobs.insert(
                job,
                PlaybackJobAccess {
                    owner: user,
                    target,
                },
            );
        }
    }

    #[doc(hidden)]
    pub async fn playback_job_target(&self, job_id: &str) -> Option<PlaybackJobTarget> {
        self.transcode_accounting
            .lock()
            .await
            .jobs
            .get(job_id)
            .map(|access| access.target)
    }

    /// Drop direct sessions that stopped heartbeating and transcodes
    /// whose job died, was stopped, or finished and went quiet.
    async fn prune(&self) {
        let now = OffsetDateTime::now_utc();
        let snapshot: Vec<(String, Option<String>, OffsetDateTime)> = self
            .active
            .lock()
            .await
            .iter()
            .map(|(id, s)| (id.clone(), s.job.clone(), s.last_active_at))
            .collect();
        let mut dead = Vec::new();
        for (id, job, last) in snapshot {
            let idle = now - last;
            let alive = match job {
                None => idle < DIRECT_SESSION_IDLE,
                Some(job) => match self
                    .transcoder
                    .registry()
                    .get(&job)
                    .await
                    .map(|j| (j.status, j.on_demand))
                {
                    Some((JobStatus::Running, true)) => {
                        let alive = idle < ON_DEMAND_IDLE;
                        if !alive
                            && let Err(error) = self
                                .transcoder
                                .stop_job(&JobHandle {
                                    id: job,
                                    output_dir: PathBuf::new(),
                                })
                                .await
                        {
                            tracing::warn!(%error, "stopping an idle on-demand session failed");
                        }
                        alive
                    }
                    Some((JobStatus::Running, false)) => true,
                    Some((JobStatus::Completed, _)) => idle < FINISHED_TRANSCODE_IDLE,
                    _ => false,
                },
            };
            if !alive {
                dead.push(id);
            }
        }
        if !dead.is_empty() {
            let mut active = self.active.lock().await;
            for id in dead {
                active.remove(&id);
            }
        }
    }

    async fn register(&self, id: String, session: ActiveSession) {
        self.prune().await;
        self.active.lock().await.insert(id, session);
    }

    /// Server-side encode preferences: `encode_video`, then the configured
    /// ladder's audio codecs, with AAC as the universal fallback.
    fn decide_options(&self, burn_subtitle: bool) -> DecideOptions {
        let encode_video = self.encode_video.clone();
        let mut encode_audio: Vec<AudioCodec> = Vec::new();
        for entry in &self.cfg.ladder {
            if !encode_audio.contains(&entry.audio_codec) {
                encode_audio.push(entry.audio_codec);
            }
        }
        if !encode_audio.contains(&AudioCodec::Aac) {
            encode_audio.push(AudioCodec::Aac);
        }
        DecideOptions {
            burn_subtitle,
            encode_video,
            encode_audio,
            audio_stream: None,
        }
    }

    /// Every step `probe` could be transcoded to for this client, best first:
    /// toward what `decision` encodes to, or else what a forced encode would.
    /// Above the file's own bitrate a step isn't one down from the original.
    fn ladder_for(
        &self,
        probe: &ProbeReport,
        decision: &PlaybackDecision,
        capabilities: &ClientCapabilities,
        options: &DecideOptions,
        max_bitrate_bps: Option<u64>,
    ) -> Vec<LadderStep> {
        let encode = |decision: &PlaybackDecision| match &decision.route {
            PlaybackRoute::Transcoding { plan, .. } => match plan.video {
                VideoPlan::Encode {
                    codec,
                    width,
                    height,
                    ..
                } => Some((codec, width, height)),
                _ => None,
            },
            _ => None,
        };
        let (target, original) = match encode(decision) {
            Some(target) => (Some(target), false),
            None => {
                let forced = DecideOptions {
                    burn_subtitle: true,
                    ..options.clone()
                };
                (encode(&decide(probe, capabilities, &forced)), true)
            }
        };
        let Some((codec, width, height)) = target else {
            return Vec::new();
        };
        let mut steps = ladder_steps(&self.cfg.ladder, codec, width, height, max_bitrate_bps);
        if original && let Some(source) = probe.format_bit_rate {
            steps.retain(|step| step.bitrate_bps < source);
        }
        steps
    }

    /// Text tracks among `tracks` (`media`'s, as [`subtitle_tracks`] lists
    /// them) the client can render once converted to a format it declared.
    /// The URL reuses the direct-file token scope, which the subtitle route
    /// checks.
    fn subtitle_deliveries(
        &self,
        user: UserId,
        login: Option<SessionId>,
        media: &MediaFile,
        tracks: &[SubtitleTrack],
        caps: &ClientCapabilities,
    ) -> anyhow::Result<Vec<SubtitleDelivery>> {
        if !caps.subtitle_codecs.contains(&SubtitleCodec::WebVtt) {
            return Ok(Vec::new());
        }
        let deliverable: Vec<(usize, &SubtitleTrack)> = tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| match (&t.path, t.stream_index, &t.downloaded) {
                (Some(_), _, _) => {
                    sidecar_codec(&t.codec).is_some_and(subtitles::converts_to_webvtt)
                }
                (None, Some(_), _) => {
                    sidecar_codec(&t.codec).is_some_and(subtitles::extracts_to_webvtt)
                }
                (None, None, Some(downloaded)) => {
                    self.subtitles.is_some() && subtitles::converts_to_webvtt(downloaded.format)
                }
                (None, None, None) => false,
            })
            .collect();
        if deliverable.is_empty() {
            return Ok(Vec::new());
        }
        let token = self.media_token(user, login, &format!("file:{}", media.id))?;
        Ok(deliverable
            .into_iter()
            .filter_map(|(i, t)| {
                Some(SubtitleDelivery {
                    index: u32::try_from(i).ok()?,
                    language: t.language.clone(),
                    title: t.title.clone(),
                    forced: t.forced,
                    default: t.default,
                    codec: SubtitleCodec::WebVtt,
                    url: format!("/stream/{}/subtitles/{i}?token={token}", media.id),
                })
            })
            .collect())
    }

    /// Start a live-channel transcode. `input` is the channel's stream
    /// URL; anything but http(s) is refused before ffmpeg sees it.
    pub async fn start_live(
        &self,
        user: UserId,
        channel: ChannelId,
        input: &str,
    ) -> anyhow::Result<PlaybackSession> {
        let input = TranscodeInput::url(input)?;
        let is_admin = self
            .storage
            .get_user(user)
            .await?
            .is_some_and(|user| user.is_admin);
        let policy = if is_admin {
            noflex_storage::domain::UserRestrictions::unrestricted(user)
        } else {
            self.storage
                .get_user_restrictions(user)
                .await?
                .unwrap_or_else(|| noflex_storage::domain::UserRestrictions::unrestricted(user))
        };
        self.reserve_transcode(user, policy.max_concurrent_transcodes)
            .await?;
        // Tuner feeds are re-encoded to the ladder; their probe isn't known up
        // front, and broadcast tops out at 1080p.
        let ladder = pick_ladder(
            &self.cfg.ladder,
            VideoCodec::H264,
            1920 * 1080,
            policy.max_stream_bitrate_bps,
        );
        let (width, height) = fit_pixels(1920, 1080, ladder.max_pixels);
        let plan = TranscodePlan {
            segment_container: HlsSegmentContainer::Ts,
            video: VideoPlan::Encode {
                codec: VideoCodec::H264,
                width,
                height,
                tonemap: false,
                profile: noflex_core::VideoProfile::H264High,
            },
            audio: AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: 2,
            },
        };
        let encoder = pick_encoder(self.backend, VideoCodec::H264);
        let encoded = EncodedVideo {
            width: even(width),
            height,
            bitrate_bps: u64::from(scaled_bitrate_kbps(
                &ladder,
                u64::from(width) * u64::from(height),
            )) * 1000,
        };
        let mut request = TranscodeRequest::new(input, "index.m3u8", plan, encoder, ladder);
        request.reasons = TranscodeReasons::LiveSource;
        request.segment = SegmentConfig {
            playlist_type: PlaylistType::Live,
            ..SegmentConfig::default()
        };
        request.vaapi_device = self.vaapi_device.clone();
        request.variant = HlsVariant::for_plan(
            &ProbeReport::default(),
            &plan,
            None,
            Some(encoded),
            "index.m3u8",
        );
        let media_file = MediaFileId::new();
        let handle = match self.transcoder.start_job(media_file, request).await {
            Ok(handle) => handle,
            Err(error) => {
                self.finish_reservation(user, None).await;
                return Err(error);
            }
        };
        self.finish_reservation(
            user,
            Some((handle.id.clone(), PlaybackJobTarget::LiveChannel(channel))),
        )
        .await;
        let token = self.media_token(user, None, &format!("job:{}", handle.id))?;
        let decision = PlaybackDecisionInfo::live(&plan, encoder, Some(encoded));
        let now = OffsetDateTime::now_utc();
        self.register(
            handle.id.clone(),
            ActiveSession {
                user,
                work: None,
                edition: None,
                media_file,
                channel: Some(channel),
                device: None,
                started_at: now,
                last_active_at: now,
                position_ms: None,
                paused: false,
                decision: decision.clone(),
                job: Some(handle.id.clone()),
            },
        )
        .await;
        Ok(PlaybackSession {
            session_id: ID::from(handle.id.clone()),
            route: PlaybackRouteKind::Transcode,
            url: format!("/stream/job/{}/master.m3u8?token={token}", handle.id),
            file_url: None,
            job_id: Some(ID::from(handle.id)),
            start_position_ms: 0,
            decision,
            subtitles: Vec::new(),
            qualities: Vec::new(),
            adaptive: false,
        })
    }
}

#[async_trait::async_trait]
impl PlaybackJobAuthorizer for ServerPlaybackSessions {
    async fn authorize_job(&self, principal: &Principal, job_id: &str) -> Result<(), StatusCode> {
        if !principal.is_authenticated() {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let access = self
            .transcode_accounting
            .lock()
            .await
            .jobs
            .get(job_id)
            .copied()
            .ok_or(StatusCode::NOT_FOUND)?;
        if !principal.is_admin && access.owner != principal.user_id {
            return Err(StatusCode::NOT_FOUND);
        }
        match access.target {
            PlaybackJobTarget::LiveChannel(_) => {}
            PlaybackJobTarget::Work(work) => {
                noflex_storage::access::user_can_access_work(
                    self.storage.as_ref(),
                    noflex_storage::RequestUser {
                        user_id: principal.user_id,
                        is_admin: principal.is_admin,
                    },
                    work,
                )
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .then_some(())
                .ok_or(StatusCode::NOT_FOUND)?;
            }
        }
        // Every playlist / segment fetch counts as activity.
        if access.owner == principal.user_id
            && let Some(session) = self.active.lock().await.get_mut(job_id)
        {
            session.last_active_at = OffsetDateTime::now_utc();
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl PlaybackSessionService for ServerPlaybackSessions {
    async fn start(
        &self,
        user: UserId,
        login: Option<SessionId>,
        request: PlaybackStart,
    ) -> anyhow::Result<PlaybackSession> {
        let PlaybackStart {
            media_file_id,
            mut capabilities,
            burn_subtitle,
            device,
            start_position_ms,
            audio_stream,
            adaptive: request_adaptive,
        } = request;
        let start_offset_ms = start_position_ms.filter(|ms| *ms > 0);
        let media = self
            .storage
            .media_file_by_id(media_file_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "storage error reading media file");
                anyhow::anyhow!("storage error")
            })?
            .ok_or_else(|| anyhow::anyhow!("media file not found"))?;
        let work = self
            .storage
            .work_for_media_file(media_file_id)
            .await
            .map_err(|e| anyhow::anyhow!("storage error: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("media file not found"))?;
        if !self
            .storage
            .user_can_access_work(user, work.id)
            .await
            .map_err(|e| anyhow::anyhow!("storage error: {e}"))?
        {
            anyhow::bail!("media file not found");
        }
        let is_admin = self
            .storage
            .get_user(user)
            .await
            .map_err(|e| anyhow::anyhow!("storage error: {e}"))?
            .is_some_and(|user| user.is_admin);
        let policy = if is_admin {
            noflex_storage::domain::UserRestrictions::unrestricted(user)
        } else {
            self.storage
                .get_user_restrictions(user)
                .await
                .map_err(|e| anyhow::anyhow!("storage error: {e}"))?
                .unwrap_or_else(|| noflex_storage::domain::UserRestrictions::unrestricted(user))
        };
        if let Some(max) = policy.max_stream_bitrate_bps {
            capabilities.max_bitrate_bps = Some(
                capabilities
                    .max_bitrate_bps
                    .map_or(max, |client| client.min(max)),
            );
        }

        // The scanner persists the probe on the media row; rows
        // written before that (a legacy `{}`) fall back to a lazy
        // re-probe here.
        let probe = match serde_json::from_value::<ProbeReport>(media.probe.0.clone()) {
            Ok(p) => p,
            Err(_) => noflex_scanner::ffprobe::probe_media_file(Path::new(&media.path), None)
                .await
                .map_err(|e| anyhow::anyhow!("cannot probe media file: {e}"))?,
        };

        let tracks = subtitle_tracks(self.storage.as_ref(), &media)
            .await
            .map_err(|e| anyhow::anyhow!("storage error: {e}"))?;
        // Resolve a burn-in request before deciding: a bad index or a
        // non-sidecar track is a client error regardless of route.
        let burn_source = match burn_subtitle {
            Some(i) => {
                let i =
                    usize::try_from(i).map_err(|_| anyhow::anyhow!("subtitle track not found"))?;
                let track = tracks
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("subtitle track not found"))?;
                match (track.path.as_deref(), track.stream_index, &track.downloaded) {
                    (Some(p), _, _) => Some(SubtitleBurnSource::External(PathBuf::from(p))),
                    (None, Some(stream), _) => Some(SubtitleBurnSource::Embedded(
                        usize::try_from(stream)
                            .map_err(|_| anyhow::anyhow!("subtitle track not found"))?,
                    )),
                    // Burned in from the cache just as a sidecar is from beside the file.
                    (None, None, Some(downloaded)) => {
                        let downloads = self
                            .subtitles
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("subtitle track not found"))?;
                        downloads.record_use(downloaded).await;
                        Some(SubtitleBurnSource::External(downloads.path(downloaded)))
                    }
                    (None, None, None) => anyhow::bail!("subtitle track not found"),
                }
            }
            None => None,
        };

        let audio_stream = audio_stream
            .map(|stream| {
                usize::try_from(stream)
                    .ok()
                    .filter(|stream| *stream < probe.audio.len())
                    .ok_or_else(|| anyhow::anyhow!("audio track not found"))
            })
            .transpose()?;
        let options = DecideOptions {
            audio_stream,
            ..self.decide_options(burn_source.is_some())
        };
        let mut decision = decide(&probe, &capabilities, &options);
        let ladder = self.ladder_for(
            &probe,
            &decision,
            &capabilities,
            &options,
            policy.max_stream_bitrate_bps,
        );
        let qualities: Vec<StreamQuality> = ladder.iter().map(LadderStep::quality).collect();
        // Up to the quality asked for: a pinned one is its bitrate as the limit.
        let steps = within(&ladder, capabilities.max_bitrate_bps);
        if let PlaybackRoute::Transcoding { plan, .. } = &mut decision.route
            && let VideoPlan::Encode {
                width,
                height,
                profile,
                ..
            } = &mut plan.video
            && let Some(top) = steps.first()
        {
            (*width, *height) = (top.width, top.height);
            *profile = top.rung.cap_profile(*profile);
        }
        let subtitles = if burn_source.is_some() {
            Vec::new()
        } else {
            self.subtitle_deliveries(user, login, &media, &tracks, &capabilities)?
        };
        let now = OffsetDateTime::now_utc();
        let mut active = ActiveSession {
            user,
            work: Some(work.id),
            edition: Some(media.edition_id),
            media_file: media.id,
            channel: None,
            device,
            started_at: now,
            last_active_at: now,
            position_ms: None,
            paused: false,
            decision: PlaybackDecisionInfo::new(&probe, &decision, Encoder::Software264, None),
            job: None,
        };
        // Burned-in subtitles are the one thing a prepared version can't offer.
        let prepared = match (&decision.route, &burn_source) {
            (PlaybackRoute::Transcoding { .. }, None) => {
                self.playable_version(&media, &capabilities).await?
            }
            _ => None,
        };
        let session = match (&decision.route, prepared) {
            (_, Some(version)) => {
                let session_id = uuid::Uuid::now_v7().to_string();
                let token = self.media_token(user, login, &format!("optimized:{}", version.id))?;
                active.decision.route = PlaybackRouteKind::Optimized;
                self.register(session_id.clone(), active.clone()).await;
                PlaybackSession {
                    session_id: ID::from(session_id),
                    route: PlaybackRouteKind::Optimized,
                    url: format!("/optimized/{}/master.m3u8?token={token}", version.id),
                    file_url: Some(format!("/optimized/{}/file?token={token}", version.id)),
                    job_id: None,
                    start_position_ms: 0,
                    decision: active.decision,
                    subtitles,
                    qualities: qualities.clone(),
                    adaptive: false,
                }
            }
            (PlaybackRoute::Direct, None) => {
                let session_id = uuid::Uuid::now_v7().to_string();
                let token = self.media_token(user, login, &format!("file:{media_file_id}"))?;
                self.register(session_id.clone(), active.clone()).await;
                PlaybackSession {
                    session_id: ID::from(session_id),
                    route: PlaybackRouteKind::Direct,
                    url: format!("/stream/{media_file_id}/file?token={token}"),
                    file_url: None,
                    job_id: None,
                    start_position_ms: 0,
                    decision: active.decision,
                    subtitles,
                    qualities: qualities.clone(),
                    adaptive: false,
                }
            }
            (PlaybackRoute::Transcoding { plan, reasons }, None) => {
                self.reserve_transcode(user, policy.max_concurrent_transcodes)
                    .await?;
                let tonemap = match plan.video {
                    VideoPlan::Encode { tonemap: true, .. } => TonemapOptions {
                        gpu: self.gpu_tonemap,
                        curve: match self.storage.tonemap_curve().await {
                            Ok(curve) => curve.unwrap_or(self.cfg.tonemap_curve),
                            Err(e) => {
                                self.finish_reservation(user, None).await;
                                anyhow::bail!("storage error: {e}");
                            }
                        },
                    },
                    _ => TonemapOptions::default(),
                };
                let request = |plan: TranscodePlan, encoder: Encoder, ladder: LadderEntry| {
                    let mut req = TranscodeRequest::new(
                        TranscodeInput::File(PathBuf::from(&media.path)),
                        "index.m3u8",
                        plan,
                        encoder,
                        ladder,
                    );
                    req.reasons = *reasons;
                    req.vaapi_device = self.vaapi_device.clone();
                    req.tonemap = tonemap;
                    req.audio_stream = decision.audio_stream;
                    req.subtitle_burn = burn_source.clone();
                    req
                };
                // A video encode with a known length goes on a grid of
                // segments the player can seek anywhere in.
                let grid = probe
                    .duration_ms
                    .filter(|ms| *ms > 0)
                    .map(|ms| SegmentGrid {
                        duration_secs: ms as f64 / 1000.0,
                        segment_secs: SegmentConfig::default().duration_secs,
                    });
                let started = match (plan.video, grid) {
                    (VideoPlan::Encode { codec, .. }, Some(grid)) if !steps.is_empty() => {
                        let encoder = pick_encoder(self.backend, codec);
                        let channels = probe
                            .audio
                            .get(decision.audio_stream.unwrap_or(0))
                            .map_or(2, |track| track.channels);
                        let taken = if request_adaptive { steps.len() } else { 1 };
                        let variants: Vec<(TranscodeRequest, EncodedVideo)> = steps[..taken]
                            .iter()
                            .enumerate()
                            .map(|(index, step)| {
                                let mut plan = *plan;
                                plan.audio = audio_for_runs(plan.audio, channels);
                                if let VideoPlan::Encode {
                                    width,
                                    height,
                                    profile,
                                    ..
                                } = &mut plan.video
                                {
                                    (*width, *height) = (step.width, step.height);
                                    *profile = step.rung.cap_profile(*profile);
                                }
                                let encoded = step.encoded();
                                let mut req = request(plan, encoder, step.rung.clone());
                                req.variant = HlsVariant::for_plan(
                                    &probe,
                                    &plan,
                                    decision.audio_stream,
                                    Some(encoded),
                                    variant_playlist(index),
                                );
                                (req, encoded)
                            })
                            .collect();
                        let encoded = variants.first().map(|(_, encoded)| *encoded);
                        let first = grid.index_at(start_offset_ms.unwrap_or(0) as f64 / 1000.0);
                        let adaptive = variants.len() > 1;
                        self.transcoder
                            .start_segmented(
                                media.id,
                                SegmentedRequest {
                                    grid,
                                    variants: variants.into_iter().map(|(req, _)| req).collect(),
                                },
                                first,
                            )
                            .await
                            .map(|handle| {
                                let audio = audio_for_runs(plan.audio, channels);
                                (handle, encoder, encoded, 0, adaptive, Some(audio))
                            })
                    }
                    _ => {
                        let (encoder, encoded, ladder) = match plan.video {
                            VideoPlan::Encode {
                                codec,
                                width,
                                height,
                                ..
                            } => {
                                let pixels = u64::from(width) * u64::from(height);
                                let ladder = pick_ladder(
                                    &self.cfg.ladder,
                                    codec,
                                    pixels,
                                    capabilities.max_bitrate_bps,
                                );
                                let encoded = EncodedVideo {
                                    width: even(width),
                                    height,
                                    bitrate_bps: u64::from(scaled_bitrate_kbps(&ladder, pixels))
                                        * 1000,
                                };
                                (pick_encoder(self.backend, codec), Some(encoded), ladder)
                            }
                            _ => {
                                let pixels = probe
                                    .video
                                    .as_ref()
                                    .map_or(0, |v| u64::from(v.width) * u64::from(v.height));
                                let ladder = pick_ladder(
                                    &self.cfg.ladder,
                                    VideoCodec::H264,
                                    pixels,
                                    capabilities.max_bitrate_bps,
                                );
                                (pick_encoder(self.backend, ladder.video_codec), None, ladder)
                            }
                        };
                        let mut req = request(*plan, encoder, ladder);
                        // A copied video stream can only start on a keyframe, so
                        // snap to it and report that offset; otherwise the player's
                        // timeline (offset + stream time) would run up to a GOP
                        // ahead of the picture.
                        let start_offset_ms = match (start_offset_ms, &plan.video) {
                            (Some(ms), VideoPlan::Copy { .. }) => {
                                let target = ms as f64 / 1000.0;
                                match noflex_scanner::ffprobe::keyframe_at_or_before(
                                    Path::new(&media.path),
                                    target,
                                    None,
                                )
                                .await
                                {
                                    Ok(Some(keyframe)) => Some((keyframe * 1000.0).round() as i64),
                                    Ok(None) => Some(ms),
                                    Err(error) => {
                                        tracing::debug!(%error, "keyframe lookup failed; seeking unsnapped");
                                        Some(ms)
                                    }
                                }
                            }
                            (offset, _) => offset,
                        };
                        req.start_secs = start_offset_ms.map(|ms| ms as f64 / 1000.0);
                        req.variant = HlsVariant::for_plan(
                            &probe,
                            plan,
                            decision.audio_stream,
                            encoded,
                            "index.m3u8",
                        );
                        self.transcoder
                            .start_job(media.id, req)
                            .await
                            .map(|handle| {
                                (
                                    handle,
                                    encoder,
                                    encoded,
                                    start_offset_ms.unwrap_or(0),
                                    false,
                                    None,
                                )
                            })
                    }
                };
                let (handle, encoder, encoded, start_position_ms, adaptive, audio) = match started {
                    Ok(started) => started,
                    Err(e) => {
                        self.finish_reservation(user, None).await;
                        tracing::error!(error = %e, "transcode job start failed");
                        return Err(anyhow::anyhow!("transcode start failed"));
                    }
                };
                self.finish_reservation(
                    user,
                    Some((handle.id.clone(), PlaybackJobTarget::Work(work.id))),
                )
                .await;
                let token = self.media_token(user, login, &format!("job:{}", handle.id))?;
                let mut decision = decision.clone();
                if let Some(audio) = audio
                    && let PlaybackRoute::Transcoding { plan, .. } = &mut decision.route
                {
                    plan.audio = audio;
                }
                active.decision = PlaybackDecisionInfo::new(&probe, &decision, encoder, encoded);
                active.job = Some(handle.id.clone());
                self.register(handle.id.clone(), active.clone()).await;
                PlaybackSession {
                    session_id: ID::from(handle.id.clone()),
                    route: PlaybackRouteKind::Transcode,
                    url: format!("/stream/job/{}/master.m3u8?token={token}", handle.id),
                    file_url: None,
                    job_id: Some(ID::from(handle.id)),
                    start_position_ms,
                    decision: active.decision,
                    subtitles,
                    qualities,
                    adaptive,
                }
            }
        };
        if let Some(bus) = &self.events
            && let Some((work, title)) = self.resolve_event_work(media.edition_id).await
        {
            bus.emit(DomainEvent::PlaybackStarted {
                user,
                work,
                title,
                media_file: media_file_id,
            });
        }
        Ok(session)
    }

    /// End a session; for transcodes, stop the job. Stopping an
    /// already-terminal job is a no-op success — the registry keeps
    /// its terminal status.
    async fn stop(&self, user: UserId, session_id: &str) -> anyhow::Result<()> {
        let registered = self.active.lock().await.get(session_id).cloned();
        if let Some(session) = &registered
            && session.job.is_none()
        {
            if session.user != user {
                anyhow::bail!("session not found");
            }
            self.active.lock().await.remove(session_id);
            if let Some(edition) = session.edition {
                self.emit_stopped(user, edition, session.media_file).await;
            }
            return Ok(());
        }
        let owner = self
            .transcode_accounting
            .lock()
            .await
            .jobs
            .get(session_id)
            .map(|access| access.owner)
            .ok_or_else(|| anyhow::anyhow!("session not found"))?;
        if owner != user {
            anyhow::bail!("session not found");
        }
        let Some(job) = self.transcoder.registry().get(session_id).await else {
            anyhow::bail!("session not found");
        };
        let handle = JobHandle {
            id: job.id,
            output_dir: job.output_dir,
        };
        self.transcoder.stop_job(&handle).await.map_err(|e| {
            tracing::error!(error = %e, "stop transcode job failed");
            anyhow::anyhow!("stop failed")
        })?;
        self.transcode_accounting
            .lock()
            .await
            .jobs
            .remove(session_id);
        self.active.lock().await.remove(session_id);
        // Live jobs carry no edition; a session pruned from the
        // registry still resolves through its media file.
        let edition = match &registered {
            Some(session) => session.edition,
            None => match self.storage.media_file_by_id(job.media).await {
                Ok(media) => media.map(|m| m.edition_id),
                Err(e) => {
                    tracing::warn!(error = %e, "event: media file lookup failed; skipping");
                    None
                }
            },
        };
        if let Some(edition) = edition {
            self.emit_stopped(user, edition, job.media).await;
        }
        Ok(())
    }

    async fn heartbeat(
        &self,
        user: UserId,
        target: PlayTarget,
        position_ms: i64,
        state: PlaybackState,
    ) {
        let now = OffsetDateTime::now_utc();
        let mut ended = Vec::new();
        // (edition, media file, now paused) for each session that changed.
        let mut turned = Vec::new();
        {
            let mut active = self.active.lock().await;
            for (id, session) in active.iter_mut() {
                let matches = session.user == user
                    && match target {
                        PlayTarget::Work(work) => session.work == Some(work),
                        PlayTarget::Edition(edition) => session.edition == Some(edition),
                    };
                if !matches {
                    continue;
                }
                session.last_active_at = now;
                session.position_ms = Some(position_ms);
                let paused = match state {
                    PlaybackState::Playing => false,
                    PlaybackState::Paused => true,
                    PlaybackState::Stopped => session.paused,
                };
                if paused != session.paused {
                    session.paused = paused;
                    if let Some(edition) = session.edition {
                        turned.push((edition, session.media_file, paused));
                    }
                }
                if state == PlaybackState::Stopped && session.job.is_none() {
                    ended.push(id.clone());
                }
            }
            for id in &ended {
                active.remove(id);
            }
        }
        let Some(bus) = &self.events else {
            return;
        };
        let position_ms = u64::try_from(position_ms).unwrap_or(0);
        for (edition, media_file, paused) in turned {
            let Some((work, title)) = self.resolve_event_work(edition).await else {
                continue;
            };
            bus.emit(if paused {
                DomainEvent::PlaybackPaused {
                    user,
                    work,
                    title,
                    media_file,
                    position_ms,
                }
            } else {
                DomainEvent::PlaybackResumed {
                    user,
                    work,
                    title,
                    media_file,
                    position_ms,
                }
            });
        }
    }

    async fn smart_download(
        &self,
        user: UserId,
        login: Option<SessionId>,
        media: &MediaFile,
        capabilities: &ClientCapabilities,
    ) -> anyhow::Result<Option<SmartDownloadFile>> {
        let Ok(probe) = serde_json::from_value::<ProbeReport>(media.probe.0.clone()) else {
            return Ok(None);
        };
        let decision = decide(&probe, capabilities, &self.decide_options(false));
        let (url, size_bytes, optimized) = if matches!(decision.route, PlaybackRoute::Direct) {
            let token = self.download_token(user, login, &format!("file:{}", media.id))?;
            (
                format!("/stream/{}/file?token={token}", media.id),
                media.size_bytes,
                false,
            )
        } else {
            let Some(version) = self.playable_version(media, capabilities).await? else {
                return Ok(None);
            };
            let token = self.download_token(user, login, &format!("optimized:{}", version.id))?;
            (
                format!("/optimized/{}/file?token={token}", version.id),
                version.size_bytes.unwrap_or(0),
                true,
            )
        };
        let tracks = subtitle_tracks(self.storage.as_ref(), media)
            .await
            .map_err(|e| anyhow::anyhow!("storage error: {e}"))?;
        Ok(Some(SmartDownloadFile {
            url,
            size_bytes,
            optimized,
            subtitles: self.subtitle_deliveries(user, login, media, &tracks, capabilities)?,
        }))
    }

    async fn active_streams(&self) -> anyhow::Result<Vec<ActiveStreamRecord>> {
        self.prune().await;
        let sessions: Vec<(String, ActiveSession)> = self
            .active
            .lock()
            .await
            .iter()
            .map(|(id, s)| (id.clone(), s.clone()))
            .collect();
        let mut out = Vec::with_capacity(sessions.len());
        for (id, s) in sessions {
            let transcode = match &s.job {
                Some(job_id) => {
                    self.transcoder
                        .registry()
                        .get(job_id)
                        .await
                        .map(|job| TranscodeProgress {
                            job_id: ID::from(job_id.clone()),
                            status: job.status.as_str().to_string(),
                            encoder: s.decision.encoder.clone().unwrap_or_else(|| "copy".into()),
                            hardware: s.decision.hardware,
                            fps: job.progress.and_then(|p| p.fps),
                            speed: job.progress.and_then(|p| p.speed),
                            out_time_ms: job.progress.map(|p| (p.position_secs * 1000.0) as i64),
                            error: job.error,
                        })
                }
                None => None,
            };
            out.push(ActiveStreamRecord {
                id,
                user_id: s.user,
                work_id: s.work,
                media_file_id: s.media_file,
                channel_id: s.channel,
                device: s.device,
                started_at: s.started_at,
                last_active_at: s.last_active_at,
                position_ms: s.position_ms,
                decision: s.decision,
                transcode,
            });
        }
        out.sort_by_key(|r| r.started_at);
        Ok(out)
    }
}

/// Round down to even: 4:2:0 encoders reject odd dimensions.
fn even(v: u32) -> u32 {
    (v & !1).max(2)
}

/// The rung for a `pixels`-sized encode in `codec`; see [`LadderEntry::pick`].
/// When a bitrate limit rules out every rung, or none are configured, one is
/// made up at the limit so a stream can still start.
fn pick_ladder(
    configured: &[LadderEntry],
    codec: VideoCodec,
    pixels: u64,
    max_bitrate_bps: Option<u64>,
) -> LadderEntry {
    LadderEntry::pick(configured, codec, pixels, max_bitrate_bps)
        .cloned()
        .unwrap_or_else(|| LadderEntry {
            // Between 480p and 4K.
            max_pixels: pixels.clamp(409_920, 8_294_400),
            video_codec: VideoCodec::H264,
            video_profile: noflex_core::VideoProfile::H264High,
            audio_codec: AudioCodec::Aac,
            container: "mpegts".to_string(),
            bitrate_kbps: max_bitrate_bps
                .map(|bps| u32::try_from(bps / 1000).unwrap_or(u32::MAX))
                .unwrap_or(8_000),
        })
}

/// One step of the transcode ladder a file is offered at.
#[derive(Debug, Clone)]
struct LadderStep {
    rung: LadderEntry,
    width: u32,
    height: u32,
    bitrate_bps: u64,
    label: &'static str,
}

/// The 16:9 frames steps are named after and fitted to, largest first.
const FRAMES: [(&str, u32, u32); 6] = [
    ("4K", 3840, 2160),
    ("1440p", 2560, 1440),
    ("1080p", 1920, 1080),
    ("720p", 1280, 720),
    ("480p", 854, 480),
    ("360p", 640, 360),
];

impl LadderStep {
    fn encoded(&self) -> EncodedVideo {
        EncodedVideo {
            width: self.width,
            height: self.height,
            bitrate_bps: self.bitrate_bps,
        }
    }

    fn quality(&self) -> StreamQuality {
        StreamQuality {
            label: self.label.to_string(),
            width: self.width,
            height: self.height,
            bitrate_bps: self.bitrate_bps,
        }
    }
}

/// The smallest frame `width`×`height` fits, a little over allowed so a
/// 1920×1088 encode is still 1080p.
fn frame_label(width: u32, height: u32) -> &'static str {
    FRAMES
        .iter()
        .rev()
        .find(|(_, w, h)| width * 20 <= w * 21 && height * 20 <= h * 21)
        .map_or("4K", |(label, ..)| label)
}

/// What `codec` at `width`×`height` steps down through on `ladder`: the
/// rung for that size, then each smaller frame on its own rung, rated by
/// area, skipping a step that saves less than a fifth. Rungs over
/// `max_bitrate_bps` are out, as for [`pick_ladder`].
fn ladder_steps(
    ladder: &[LadderEntry],
    codec: VideoCodec,
    width: u32,
    height: u32,
    max_bitrate_bps: Option<u64>,
) -> Vec<LadderStep> {
    let top = pick_ladder(
        ladder,
        codec,
        u64::from(width) * u64::from(height),
        max_bitrate_bps,
    );
    let (width, height) = fit_pixels(width, height, top.max_pixels);
    let mut sizes = vec![(even(width), even(height))];
    for (_, frame_width, frame_height) in FRAMES {
        let scale = (f64::from(frame_width) / f64::from(width))
            .min(f64::from(frame_height) / f64::from(height));
        if scale >= 1.0 {
            continue;
        }
        let size = (
            even((f64::from(width) * scale).round() as u32),
            even((f64::from(height) * scale).round() as u32),
        );
        if sizes.last().is_some_and(|last| size.1 < last.1) {
            sizes.push(size);
        }
    }
    let mut steps: Vec<LadderStep> = Vec::new();
    for (width, height) in sizes {
        let pixels = u64::from(width) * u64::from(height);
        let rung = pick_ladder(ladder, codec, pixels, max_bitrate_bps);
        let bitrate_bps = u64::from(scaled_bitrate_kbps(&rung, pixels)) * 1000;
        if steps
            .last()
            .is_some_and(|last| bitrate_bps * 5 > last.bitrate_bps * 4)
        {
            continue;
        }
        steps.push(LadderStep {
            rung,
            width,
            height,
            bitrate_bps,
            label: frame_label(width, height),
        });
    }
    steps
}

/// The steps at or under `max_bitrate_bps`. Below the smallest, that step
/// at the limit, so a stream can still start.
fn within(steps: &[LadderStep], max_bitrate_bps: Option<u64>) -> Vec<LadderStep> {
    let Some(max) = max_bitrate_bps else {
        return steps.to_vec();
    };
    let under: Vec<LadderStep> = steps
        .iter()
        .filter(|step| step.bitrate_bps <= max)
        .cloned()
        .collect();
    if !under.is_empty() {
        return under;
    }
    steps
        .last()
        .map(|smallest| {
            let mut step = smallest.clone();
            step.rung.bitrate_kbps = u32::try_from(max / 1000).unwrap_or(u32::MAX).max(1);
            step.rung.max_pixels = u64::from(step.width) * u64::from(step.height);
            step.bitrate_bps = u64::from(step.rung.bitrate_kbps) * 1000;
            step
        })
        .into_iter()
        .collect()
}

/// Map the resolved backend + target codec onto an `Encoder`.
/// VideoToolbox and MediaFoundation have no AV1 encoder, so AV1
/// falls back to software there; codecs outside h264/hevc/av1 have
/// no hardware mapping at all and always encode as software h264.
pub(crate) fn pick_encoder(backend: HwBackend, codec: VideoCodec) -> Encoder {
    match backend {
        HwBackend::Software => software_for(codec),
        HwBackend::Vaapi => match codec {
            VideoCodec::H264 => Encoder::Vaapi264,
            VideoCodec::H265 => Encoder::Vaapi265,
            VideoCodec::Av1 => Encoder::VaapiAv1,
            other => software_for(other),
        },
        HwBackend::VideoToolbox => match codec {
            VideoCodec::H264 => Encoder::VideoToolbox264,
            VideoCodec::H265 => Encoder::VideoToolbox265,
            other => software_for(other),
        },
        HwBackend::Mf => match codec {
            VideoCodec::H264 => Encoder::Mf264,
            VideoCodec::H265 => Encoder::Mf265,
            other => software_for(other),
        },
        HwBackend::Nvenc => match codec {
            VideoCodec::H264 => Encoder::Nvenc264,
            VideoCodec::H265 => Encoder::Nvenc265,
            VideoCodec::Av1 => Encoder::NvencAv1,
            other => software_for(other),
        },
        HwBackend::Qsv => match codec {
            VideoCodec::H264 => Encoder::Qsv264,
            VideoCodec::H265 => Encoder::Qsv265,
            VideoCodec::Av1 => Encoder::QsvAv1,
            other => software_for(other),
        },
        HwBackend::Amf => match codec {
            VideoCodec::H264 => Encoder::Amf264,
            VideoCodec::H265 => Encoder::Amf265,
            VideoCodec::Av1 => Encoder::AmfAv1,
            other => software_for(other),
        },
    }
}

fn software_for(codec: VideoCodec) -> Encoder {
    match codec {
        VideoCodec::H265 => Encoder::Software265,
        VideoCodec::Av1 => Encoder::SoftwareAv1,
        _ => Encoder::Software264,
    }
}

/// Render the master playlist for a running (or finished) transcode
/// job from the variant recorded on its request. A verified media
/// `token` is propagated to the variant URI so the sub-playlist request
/// stays authenticated.
pub async fn master_playlist(
    registry: &TranscodeJobRegistry,
    job_id: &str,
    token: Option<&str>,
) -> Response {
    let Some(job) = registry.get(job_id).await else {
        return (StatusCode::NOT_FOUND, "job not found").into_response();
    };
    let body = noflex_transcoder::hls::render_master(&job.variants);
    let body = match token {
        Some(token) => propagate_token_to_playlist(&body, token),
        None => body,
    };
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/vnd.apple.mpegurl",
        )],
        body,
    )
        .into_response()
}

/// ffmpeg writes the media playlist a beat after the session starts.
const PLAYLIST_GRACE: std::time::Duration = std::time::Duration::from_secs(10);
const PLAYLIST_POLL: std::time::Duration = std::time::Duration::from_millis(150);

/// HLS artifacts a job dir may serve: playlists, TS / fMP4 media
/// segments and the fMP4 init segment.
fn is_hls_artifact(filename: &str) -> bool {
    [".m3u8", ".ts", ".m4s", ".mp4"]
        .iter()
        .any(|ext| filename.ends_with(ext))
}

/// Serve one artifact (playlist or segment) out of a job's output dir.
/// A linear job's segments may not exist yet mid-transcode — clients
/// poll, so a missing file is a plain 404, not an error. An on-demand
/// session's are encoded for the request, which waits for them.
pub async fn serve_job_file(
    transcoder: &FfmpegJobManager,
    cache_root: &Path,
    job_id: &str,
    filename: &str,
    token: Option<&str>,
) -> Response {
    // The filename becomes a path under the job dir; refuse
    // anything that could escape it or isn't an HLS artifact.
    if !is_hls_artifact(filename) || filename.contains('/') || filename.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }
    if let Some(fetched) = transcoder.fetch_segmented(job_id, filename).await {
        return match fetched {
            Ok(Fetched::Playlist(body)) => playlist_response(match token {
                Some(token) => propagate_token_to_playlist(&body, token),
                None => body,
            }),
            Ok(Fetched::File(path)) => match RootCapability::new(cache_root).await {
                Ok(root) => match root.open(&path).await {
                    Ok(opened) => serve_file(opened, None, ServeOpts::default()).await,
                    Err(error) => {
                        tracing::warn!(%error, path = %path.display(), "on-demand segment unreadable");
                        (StatusCode::NOT_FOUND, "file not found").into_response()
                    }
                },
                Err(error) => {
                    tracing::error!(%error, job_id, "transcode output directory unavailable");
                    (StatusCode::INTERNAL_SERVER_ERROR, "file unavailable").into_response()
                }
            },
            Err(FetchError::NotFound) => (StatusCode::NOT_FOUND, "file not found").into_response(),
            // Players retry a 503 as they would a slow segment.
            Err(FetchError::Unavailable) => {
                (StatusCode::SERVICE_UNAVAILABLE, "segment not ready").into_response()
            }
        };
    }
    let registry = transcoder.registry();
    let Some(job) = registry.get(job_id).await else {
        return (StatusCode::NOT_FOUND, "job not found").into_response();
    };
    let path = job.output_dir.join(filename);
    let root = match RootCapability::new(cache_root).await {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(%error, job_id, "transcode output directory unavailable");
            return (StatusCode::INTERNAL_SERVER_ERROR, "file unavailable").into_response();
        }
    };
    // A playlist request against a live job waits briefly rather than
    // handing every client a 404 to retry.
    let deadline = tokio::time::Instant::now() + PLAYLIST_GRACE;
    let opened = loop {
        match root.open(&path).await {
            Ok(opened) => break opened,
            Err(error) => {
                let waiting = filename.ends_with(".m3u8")
                    && tokio::time::Instant::now() < deadline
                    && registry.get(job_id).await.is_some();
                if !waiting {
                    tracing::debug!(%error, path = %path.display(), "rejected transcode artifact");
                    return (StatusCode::NOT_FOUND, "file not found").into_response();
                }
                tokio::time::sleep(PLAYLIST_POLL).await;
            }
        }
    };
    // Playlists are tiny text: when a media token authenticated this
    // request, rewrite segment URIs to carry it (see
    // `propagate_token_to_playlist`) instead of streaming the file.
    if filename.ends_with(".m3u8")
        && let Some(token) = token
    {
        let mut body = String::new();
        let mut file = opened.into_file();
        if let Err(error) = file.read_to_string(&mut body).await {
            tracing::error!(%error, path = %path.display(), "read playlist failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response();
        }
        return playlist_response(propagate_token_to_playlist(&body, token));
    }
    serve_file(opened, None, ServeOpts::default()).await
}

fn playlist_response(body: String) -> Response {
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/vnd.apple.mpegurl",
        )],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_reaches_init_segment_and_segments() {
        let body = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.0,\nsegment_00000.m4s\n";
        let out = propagate_token_to_playlist(body, "t");
        assert_eq!(
            out,
            "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4?token=t\"\n#EXTINF:4.0,\nsegment_00000.m4s?token=t"
        );
    }

    #[test]
    fn hls_artifacts_cover_fmp4() {
        for ok in [
            "index.m3u8",
            "segment_00001.ts",
            "segment_00001.m4s",
            "init.mp4",
        ] {
            assert!(is_hls_artifact(ok), "{ok}");
        }
        assert!(!is_hls_artifact("movie.mkv"));
    }

    #[test]
    fn sidecar_codec_accepts_scanner_and_extension_spellings() {
        assert_eq!(sidecar_codec("sub_rip"), Some(SubtitleCodec::SubRip));
        assert_eq!(sidecar_codec("srt"), Some(SubtitleCodec::SubRip));
        assert_eq!(sidecar_codec("web_vtt"), Some(SubtitleCodec::WebVtt));
        assert_eq!(sidecar_codec("ass"), Some(SubtitleCodec::Ass));
        assert_eq!(sidecar_codec("pgs"), None);
    }

    #[test]
    fn even_dimensions() {
        assert_eq!(even(1281), 1280);
        assert_eq!(even(0), 2);
    }
}
