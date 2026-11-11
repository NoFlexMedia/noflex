// SPDX-License-Identifier: AGPL-3.0-only
//
// Storage-shape domain types. Flat, owned, serde-friendly. Backends translate
// between these and their on-disk representation; the service layer translates
// between these and the GraphQL / Jellyfin-shim wire types.

use async_graphql::{Enum, SimpleObject, connection::Connection};
use noflex_core::{
    ApiKeyId, ChannelId, CollectionId, DeviceCodeId, DeviceId, EditionId, EpgProgramId,
    ExportJobId, ExternalId, ExternalIdKind, ImageKind, InviteId, LibraryId, LinkedServerId,
    LiveTvSourceId, MediaFileId, MetadataStatus, OptimizedVersionId, PlaySessionId, PlayTargetKind,
    PlaylistId, RecordingId, RecordingScheduleId, SessionId, TagId, TagTarget, UserId, VideoCodec,
    WatchRoomId, WorkField, WorkId, WorkKind,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// A logical work — the abstract thing a user thinks of as "the movie" or
/// "the episode," regardless of how many files it's stored as.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct Work {
    pub id: WorkId,
    pub kind: WorkKind,
    pub parent_id: Option<WorkId>,
    pub library_id: LibraryId,
    pub title: String,
    pub sort_title: String,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    pub overview: Option<String>,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub community_rating: Option<f32>,
    pub external_ids: Vec<ExternalId>,
    pub index_number: Option<i32>,
    /// Inclusive last index in a range. `None` for a normal single-indexed
    /// Work (e.g. one Episode = one number). `Some(end)` when this Work
    /// represents a range — multi-episode files like `s01e01-e02` land here
    /// with `index_number = 1, index_end = Some(2)`.
    pub index_end: Option<i32>,
    pub parent_index: Option<i32>,
    /// Confidence on the canonical fields (`title`, `year`, `overview`).
    /// Scanner-created rows are always [`MetadataStatus::Unconfirmed`];
    /// providers and user edits escalate.
    pub metadata_status: MetadataStatus,
    /// Fields the user has locked; provider refreshes must not
    /// overwrite them. Invariant: sorted + deduped (enforced by
    /// writers, not the type).
    #[serde(default)]
    pub locked_fields: Vec<WorkField>,
    /// The plugin whose fetch last supplied this work's fields. `None`
    /// until a provider matches it. Clients read [`Work::attributions`].
    #[serde(default)]
    #[graphql(skip)]
    pub metadata_provider: Option<String>,
    pub added_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// An edition is a specific cut of a work. Most works have one ("Default");
/// collector-grade libraries have multiple ("Theatrical", "Director's Cut",
/// "Extended"). Encodes of one cut, like 4K HDR and 1080p SDR, are that edition's
/// [`MediaFile`]s, and what's true of a picture or a track is recorded on them.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct Edition {
    pub id: EditionId,
    pub work_id: WorkId,
    pub name: String,
    pub runtime_ms: Option<i64>,
    pub intro_start_ms: Option<i64>,
    pub intro_end_ms: Option<i64>,
    pub credits_start_ms: Option<i64>,
    pub chapters: Vec<Chapter>,
}

#[async_graphql::ComplexObject]
impl Edition {
    async fn trickplay(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<TrickplayAsset>> {
        let storage = ctx
            .data::<std::sync::Arc<dyn crate::Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let user = crate::access::request_user(ctx)?;
        if !crate::access::user_can_access_edition(storage.as_ref(), user, self.id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        {
            return Ok(None);
        }
        storage
            .get_trickplay(self.id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}

/// Mints user-bound media tokens for protected byte routes. A token
/// stands in for `user`'s header credential only; the routes still run
/// `user`'s access checks. A token minted with a `session` dies with it.
pub trait MediaUrlSigner: Send + Sync {
    /// Token scoped to one edition's trickplay routes.
    fn sign_trickplay_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        edition_id: EditionId,
    ) -> Result<String, String>;
    /// Token scoped to every image of `target`. Must be deterministic
    /// for a given `(user, session, target)` over a bounded window so
    /// image URLs stay HTTP-cacheable.
    fn sign_image_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        target: ImageTarget,
    ) -> Result<String, String>;
    /// Token for a Live TV channel's proxied logo, deterministic like
    /// [`MediaUrlSigner::sign_image_token`].
    fn sign_channel_logo_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        channel: ChannelId,
    ) -> Result<String, String>;
    /// Token for a user's picture, deterministic like
    /// [`MediaUrlSigner::sign_image_token`].
    fn sign_avatar_token(
        &self,
        user: UserId,
        session: Option<SessionId>,
        avatar_of: UserId,
    ) -> Result<String, String>;
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn trickplay_url(
    ctx: &async_graphql::Context<'_>,
    edition_id: EditionId,
    path: String,
) -> async_graphql::Result<String> {
    let user = crate::access::request_user(ctx)?;
    let signer = ctx
        .data_opt::<std::sync::Arc<dyn MediaUrlSigner>>()
        .ok_or_else(|| async_graphql::Error::new("media URL signer not configured"))?;
    let token = signer
        .sign_trickplay_token(user.user_id, crate::access::caller_session(ctx), edition_id)
        .map_err(async_graphql::Error::new)?;
    Ok(format!("{path}?token={}", percent_encode(&token)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct TrickplaySheet {
    #[graphql(skip)]
    pub edition_id: EditionId,
    pub index: u32,
    #[graphql(skip)]
    pub path: String,
    pub tile_count: u32,
}

#[async_graphql::ComplexObject]
impl TrickplaySheet {
    async fn url(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<String> {
        trickplay_url(
            ctx,
            self.edition_id,
            format!("/trickplay/{}/sheets/{}", self.edition_id, self.index),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct TrickplayChapterCue {
    #[graphql(skip)]
    pub edition_id: EditionId,
    pub position_ms: i64,
    pub title: Option<String>,
    pub cue_start_ms: i64,
    pub cue_end_ms: i64,
    pub sheet_index: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[async_graphql::ComplexObject]
impl TrickplayChapterCue {
    async fn sheet_url(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<String> {
        trickplay_url(
            ctx,
            self.edition_id,
            format!("/trickplay/{}/sheets/{}", self.edition_id, self.sheet_index),
        )
    }
}

/// Durable sprite manifest for one edition. Paths are server-internal;
/// clients receive the authenticated serving URLs instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct TrickplayAsset {
    pub edition_id: EditionId,
    #[graphql(skip)]
    pub vtt_path: String,
    pub thumb_width: u32,
    pub thumb_height: u32,
    pub tile_columns: u32,
    pub tile_rows: u32,
    pub interval_ms: u64,
    pub source_media_mtime: OffsetDateTime,
    /// Identity of the source media file: `"{path}\n{size_bytes}\n{mtime}"`
    /// with `mtime` rendered as UTC RFC 3339 with exactly nine fractional
    /// digits and a `Z` suffix. Producers must build it with
    /// `noflex_storage::canonical::trickplay_source_fingerprint`; backends
    /// may recompute it from stored media columns to detect staleness.
    pub source_fingerprint: String,
    pub generated_at: OffsetDateTime,
    pub sheets: Vec<TrickplaySheet>,
    pub chapter_cues: Vec<TrickplayChapterCue>,
}

#[async_graphql::ComplexObject]
impl TrickplayAsset {
    async fn vtt_url(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<String> {
        trickplay_url(
            ctx,
            self.edition_id,
            format!("/trickplay/{}/index.vtt", self.edition_id),
        )
    }
}

/// Write-side payload for plugin-driven intro/credits detection: the
/// marker columns a detector result overwrites on an [`Edition`].
/// `None` clears the column — a detector that finds no intro writes
/// all-`None` rather than leaving a stale window behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
pub struct IntroMarkers {
    pub intro_start_ms: Option<i64>,
    pub intro_end_ms: Option<i64>,
    pub credits_start_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
pub struct SubtitleTrack {
    pub language: Option<String>,
    pub codec: String,
    pub default: bool,
    pub forced: bool,
    /// `true` for sidecar files, `false` for muxed-in.
    #[serde(default)]
    pub external: bool,
    /// Codec profile (e.g. `"ssa"`, `"ass"`, `"hdmv-pgs"`).
    #[serde(default)]
    pub codec_profile: Option<String>,
    /// Precomputed: `true` if the track can be remuxed to fmp4 WebVTT
    /// directly, `false` if it must be transcoded from a bitmap
    /// format.
    #[serde(default)]
    pub srt_compatible: bool,
    /// PGS image count (image-based subs only).
    #[serde(default)]
    pub image_count: Option<u32>,
    /// Human-readable label, e.g. `"SDH"` or `"Signs/Songs"`.
    #[serde(default)]
    pub title: Option<String>,
    /// Absolute path of the sidecar file for external tracks;
    /// `None` for muxed-in streams.
    #[serde(default)]
    pub path: Option<String>,
    /// Position among the container's subtitle streams (ffmpeg's
    /// `0:s:N`) for muxed-in tracks; `None` for sidecars.
    #[serde(default)]
    pub stream_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct Chapter {
    pub position_ms: i64,
    pub title: Option<String>,
}

/// A physical media file on disk. Multiple files per edition = multi-part
/// (CD1/CD2). Multiple editions per work = the multi-version case.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct MediaFile {
    pub id: MediaFileId,
    pub edition_id: EditionId,
    pub path: String,
    pub part_index: u32,
    pub size_bytes: u64,
    pub mtime: OffsetDateTime,
    pub container: Option<String>,
    /// ffprobe output (codec, tracks, HDR profile, etc.) as opaque JSON.
    /// `Json` is `#[serde(transparent)]`, so the stored column shape is
    /// unchanged.
    pub probe: async_graphql::Json<serde_json::Value>,
    /// External subtitle sidecars the scanner discovered next to
    /// this file. Muxed-in subtitle streams are not duplicated
    /// here — they stay in `probe`.
    #[serde(default)]
    pub subtitle_tracks: Vec<SubtitleTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct User {
    pub id: UserId,
    /// What they sign in with. Unique, and changeable, so nothing else keys on it.
    pub username: String,
    /// The name they've chosen. Read through `name`, which covers not having one.
    #[graphql(skip)]
    pub display_name: Option<String>,
    /// Argon2id-hashed password. Empty string for OIDC/LDAP-only users.
    /// Never leaves the server — skipped on the GQL surface.
    #[graphql(skip)]
    pub password_hash: String,
    pub is_admin: bool,
    pub role: noflex_core::UserRole,
    pub created_at: OffsetDateTime,
    /// When their picture last changed; `None` without one. Written only through
    /// [`Storage::set_user_avatar`](crate::Storage::set_user_avatar).
    #[graphql(skip)]
    pub avatar_updated_at: Option<OffsetDateTime>,
    /// Whether a PIN opens their profile on a device they share. Setting or
    /// removing the PIN changes it; saving the user never does.
    pub has_pin: bool,
}

impl User {
    /// What to call them: their chosen name, or their username without one.
    pub fn shown_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.username)
    }

    fn signed_avatar_url(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<String>> {
        self.avatar_url_for(
            ctx.data_opt::<std::sync::Arc<dyn MediaUrlSigner>>()
                .map(std::sync::Arc::as_ref),
            crate::access::request_user(ctx)?.user_id,
            crate::access::caller_session(ctx),
        )
        .map_err(async_graphql::Error::new)
    }

    /// Their picture's URL as `viewer` may fetch it, relative to this server;
    /// `None` without a picture. The token dies with `session` when one is
    /// given, and there's none at all without a `signer`.
    ///
    /// # Errors
    ///
    /// The signer's, when it can't mint a token.
    pub fn avatar_url_for(
        &self,
        signer: Option<&dyn MediaUrlSigner>,
        viewer: UserId,
        session: Option<SessionId>,
    ) -> Result<Option<String>, String> {
        let Some(at) = self.avatar_updated_at else {
            return Ok(None);
        };
        let path = format!("/avatars/{}?v={}", self.id, at.unix_timestamp());
        let Some(signer) = signer else {
            return Ok(Some(path));
        };
        let token = signer.sign_avatar_token(viewer, session, self.id)?;
        Ok(Some(format!("{path}&token={}", percent_encode(&token))))
    }

    pub fn has_role(&self, role: noflex_core::UserRole) -> bool {
        self.role == role
    }

    pub fn role_as_text(&self) -> &'static str {
        match self.role {
            noflex_core::UserRole::Admin => "admin",
            noflex_core::UserRole::User => "user",
            noflex_core::UserRole::Guest => "guest",
        }
    }
}

/// What a deleted account left that doesn't cascade from its user row, for the
/// server to clear in the background. Written by
/// [`Storage::delete_account`](crate::Storage::delete_account) in the same
/// transaction that deletes the user, so one never exists without the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountPurge {
    /// The user, who no longer exists.
    pub user_id: UserId,
    /// Their export copies and recordings, as stored. An export copy another
    /// user's job shares is still theirs, and stays.
    pub files: Vec<String>,
    pub requested_at: OffsetDateTime,
}

/// The server erasing itself back to first-run setup, once its last
/// administrator deleted their account. Pending from
/// [`Storage::request_server_reset`](crate::Storage::request_server_reset) until
/// [`Storage::finish_server_reset`](crate::Storage::finish_server_reset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerReset {
    /// Folders holding optimized versions that the configuration doesn't name:
    /// where an administrator pointed smart transcoding.
    pub folders: Vec<String>,
    pub requested_at: OffsetDateTime,
}

/// Per-user content and playback policy. A user with no stored policy is
/// unrestricted. Library grants apply independently, and administrators
/// bypass this policy entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
pub struct UserRestrictions {
    pub user_id: UserId,
    /// Ids of `parental`-namespace tags the user may watch. A rated work is
    /// visible only if every parental tag on it is listed. Empty means
    /// every rating is allowed; unrated works are governed by
    /// `allow_unrated` alone. A work with no parental tag of its own is
    /// rated as its nearest rated ancestor, so an episode takes its season's
    /// or its series'.
    pub allowed_parental_rating_ids: Vec<TagId>,
    /// Whether works that neither carry nor inherit a parental tag are
    /// visible.
    pub allow_unrated: bool,
    pub max_stream_bitrate_bps: Option<u64>,
    pub max_concurrent_transcodes: Option<u32>,
}

impl UserRestrictions {
    pub fn unrestricted(user_id: UserId) -> Self {
        Self {
            user_id,
            allowed_parental_rating_ids: Vec::new(),
            allow_unrated: true,
            max_stream_bitrate_bps: None,
            max_concurrent_transcodes: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum WatchRoomState {
    Waiting,
    Playing,
    Paused,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct WatchRoom {
    pub id: WatchRoomId,
    pub host_user_id: UserId,
    pub media_file_id: MediaFileId,
    pub state: WatchRoomState,
    pub position_ms: u64,
    pub sequence: u64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct WatchRoomMember {
    pub room_id: WatchRoomId,
    pub user_id: UserId,
    pub joined_at: OffsetDateTime,
}

/// One viewing of a work, assembled from the player's progress
/// heartbeats. `watched_ms` is wall-clock time spent playing, not the
/// distance the playhead moved, so seeking doesn't inflate it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct PlaySession {
    pub id: PlaySessionId,
    pub user_id: UserId,
    /// The work that played: a movie, an episode, a track.
    pub work_id: WorkId,
    /// Top of `work_id`'s parent chain (the series of an episode, the
    /// artist of a track); `work_id` itself for a movie.
    pub root_work_id: WorkId,
    pub library_id: LibraryId,
    pub started_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    pub watched_ms: u64,
    pub position_ms: u64,
    /// Whether the last heartbeat reported playback (not paused).
    pub playing: bool,
    /// The player reported a stop; later heartbeats open a new viewing.
    pub ended: bool,
    pub device: Option<String>,
}

#[async_graphql::ComplexObject]
impl PlaySession {
    async fn user(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Option<User>> {
        complex_storage(ctx)?
            .get_user(self.user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    async fn library(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<Library>> {
        complex_storage(ctx)?
            .get_library(self.library_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Null when the work is gone or the viewer can't access it.
    async fn work(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Option<Work>> {
        visible_work_by_id(ctx, self.work_id).await
    }

    /// Null when the work is gone or the viewer can't access it.
    async fn root_work(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<Work>> {
        visible_work_by_id(ctx, self.root_work_id).await
    }
}

async fn visible_work_by_id(
    ctx: &async_graphql::Context<'_>,
    id: WorkId,
) -> async_graphql::Result<Option<Work>> {
    let storage = complex_storage(ctx)?;
    let user = crate::access::request_user(ctx)?;
    let Some(work) = storage
        .get_work(id)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?
    else {
        return Ok(None);
    };
    crate::access::visible_work(storage.as_ref(), user, work)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct UserPreference {
    pub user_id: UserId,
    pub key: String,
    pub value: async_graphql::Json<serde_json::Value>,
    pub updated_at: OffsetDateTime,
}

/// Host-side progress + health for one plugin background task. Written
/// only by the plugin-host scheduler — plugins never see `Storage` — so
/// it carries no GQL derive. `cursor` is an opaque token the plugin
/// handed back on its last run; `None` means the task has never
/// checkpointed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginTaskState {
    pub plugin_id: String,
    pub task: String,
    pub cursor: Option<String>,
    pub last_run_at: Option<OffsetDateTime>,
    pub consecutive_failures: u32,
}

/// One dynamically enqueued payload for a plugin background task
/// (host-tasks.enqueue). Dequeue is FIFO per (plugin, task) and
/// destructive: the returned row is deleted in the same call. `id` is
/// backend-assigned and monotonically increasing per table, so it
/// defines the queue order; `enqueued_at` is stamped host-side at
/// enqueue time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginTaskPayload {
    pub id: i64,
    pub plugin_id: String,
    pub task: String,
    pub payload: Option<String>,
    pub enqueued_at: OffsetDateTime,
}

/// One per-user plugin credential (OAuth token, API key) written by
/// the host after a plugin link flow. Plugins never see `Storage` —
/// the host reads these on a plugin's behalf. Deliberately carries no
/// GQL derive: `value` is a secret and must never reach the GraphQL
/// surface; the GQL layer shapes its own view type if it ever exposes
/// link state. `plugin_id` has no FK — plugins are not storage rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPluginSecret {
    pub user_id: UserId,
    pub plugin_id: String,
    pub key: String,
    pub value: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A verified identity owned by an external-auth plugin. Identity lookup is
/// exclusively by `(plugin_id, issuer, subject)`; profile fields are display
/// metadata and never participate in account matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
pub struct ExternalIdentity {
    pub plugin_id: String,
    pub issuer: String,
    pub subject: String,
    pub user_id: UserId,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub linked_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, SimpleObject)]
pub struct UserData {
    pub played: bool,
    pub position_ms: Option<i64>,
    pub play_count: u32,
    pub rating: Option<f32>,
    pub favorite: bool,
    pub last_played: Option<OffsetDateTime>,
}

/// A top-level work a user has played or started; see
/// [`Storage::watched_top_level_works`][crate::Storage::watched_top_level_works].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WatchedWork {
    pub work: WorkId,
    /// The latest activity on the work or any of its descendants.
    pub last_played: OffsetDateTime,
    /// The work or any descendant was played to completion.
    pub finished: bool,
    /// The furthest saved position on the work or any descendant.
    pub position_ms: Option<i64>,
    /// The work or any descendant is a favorite.
    pub favorite: bool,
    /// The highest of the user's ratings on the work and its descendants.
    pub rating: Option<f32>,
}

/// An invitation to create an ordinary account on this server.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct Invite {
    pub id: InviteId,
    pub created_by: UserId,
    /// What an account made from this invite can access.
    pub library_ids: Vec<LibraryId>,
    pub max_uses: u32,
    pub uses: u32,
    pub expires_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

/// Another NoFlex server a user connects to. Clients talk to it directly; this
/// server only remembers the link so it follows the user between devices.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct LinkedServer {
    pub id: LinkedServerId,
    #[graphql(skip)]
    pub user_id: UserId,
    /// Scheme, host and port only.
    pub origin: String,
    pub name: String,
    /// The user's revocable key on that server.
    pub api_key: String,
    pub sync_enabled: bool,
    /// Watch activity before this has been carried across.
    pub synced_until: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

/// A client registered for smart transcoding and downloads. `id` is chosen by
/// the client, so it survives signing out.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct Device {
    pub id: DeviceId,
    #[graphql(skip)]
    pub user_id: UserId,
    pub name: String,
    /// The app it's running, as a session's `client` is. Together with `name`, what
    /// clients read to tell what kind of device this is.
    pub client: Option<String>,
    /// What the client measured it can play, in the shape of the transcoder's
    /// `ClientCapabilities`. Opaque here, since storage sits below it.
    #[graphql(skip)]
    pub capabilities: async_graphql::Json<serde_json::Value>,
    /// Prepare optimized versions for this device while the server is idle.
    pub smart_transcode: bool,
    /// Lowers what's prepared for this device; unset means its best.
    pub max_bitrate_bps: Option<u64>,
    /// Restricts the codecs prepared for this device; empty means any it
    /// supports.
    pub allowed_video_codecs: Vec<VideoCodec>,
    /// Keep likely-watched titles on this device.
    pub smart_downloads: bool,
    pub download_budget_bytes: Option<u64>,
    /// Enforced by the client, which alone knows its connection.
    pub downloads_wifi_only: bool,
    pub created_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
}

/// Where an [`OptimizedVersion`] is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum OptimizedVersionStatus {
    Queued,
    Running,
    Ready,
    Failed,
}

/// Smart transcoding limits an administrator set from the web UI. `None` leaves
/// a limit to the server's configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartTranscodingSettings {
    /// Where new versions are written; ones already made stay where they are.
    pub dir: Option<String>,
    /// Combined size of finished versions.
    pub max_storage_gb: Option<u64>,
    /// Size of any one version, estimated before preparing it.
    pub max_version_gb: Option<u64>,
}

/// The model a vector index is built with. Vectors from different models can't be
/// compared, so anything that changes a model's output must change this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingModel {
    /// Vectors are stored and searched under this.
    pub namespace: String,
    pub model_id: String,
    pub dimensions: u32,
}

/// An OpenAI-compatible embeddings API an administrator chose over the local
/// embedder. Titles, overviews, credits and search text are sent to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct EmbeddingEndpoint {
    /// The API's base, such as `https://api.openai.com/v1`.
    pub url: String,
    /// Sent as a bearer token. Write-only: clients see only `has_api_key`.
    #[graphql(skip)]
    pub api_key: Option<String>,
    pub model: String,
    /// What the model returns, measured when the endpoint was saved.
    pub dimensions: u32,
    /// Asked of models that can shorten their output; most can't.
    pub request_dimensions: Option<u32>,
    /// Put before each query; many models were trained expecting one.
    pub query_prefix: String,
    /// Put before each document.
    pub document_prefix: String,
}

#[async_graphql::ComplexObject]
impl EmbeddingEndpoint {
    async fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }
}

/// What the planner last made of a [`SmartTranscodeMark`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartTranscodeOutcome {
    /// Versions are queued, running or ready.
    Planned,
    /// Every device this is for already plays the original.
    NotNeeded,
    /// Larger than an administrator allows a version to be.
    TooLarge,
    /// Marked for the viewer's devices, and none has smart transcoding on.
    NoDevices,
}

/// A viewer marking a title they're going to watch: a hard signal to prepare
/// it, ahead of anything the planner guesses at, until they unmark it or watch
/// it. One per user, work and scope.
#[derive(Debug, Clone, PartialEq)]
pub struct SmartTranscodeMark {
    pub user_id: UserId,
    pub work_id: WorkId,
    /// The one device this is for, whatever its smart transcoding setting.
    /// `None` is all of the viewer's devices with smart transcoding on.
    pub device_id: Option<DeviceId>,
    /// Its place in the queue, which is first come, first served across users.
    pub created_at: OffsetDateTime,
    /// `None` until the planner has looked at it.
    pub outcome: Option<SmartTranscodeOutcome>,
    /// The files it stands for right now, set by the planner. A series moves on
    /// to later episodes as it's watched.
    pub media_file_ids: Vec<MediaFileId>,
}

/// A prepared copy of a media file, derived from it and never a media file
/// itself. `plan`, `index` and `probe` are the transcoder's types, opaque here.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct OptimizedVersion {
    pub id: OptimizedVersionId,
    pub media_file_id: MediaFileId,
    /// What the version contains; unique per media file.
    pub variant_key: String,
    pub status: OptimizedVersionStatus,
    /// Lower runs sooner.
    pub priority: i32,
    #[graphql(skip)]
    pub plan: serde_json::Value,
    /// The source this was made from; a version of an older file is stale.
    #[graphql(skip)]
    pub source_size: u64,
    #[graphql(skip)]
    pub source_mtime: OffsetDateTime,
    /// 0.0 to 1.0 while running.
    pub progress: f32,
    pub attempts: u32,
    #[graphql(skip)]
    pub path: Option<String>,
    pub size_bytes: Option<u64>,
    /// Fragment byte ranges, once ready.
    #[graphql(skip)]
    pub index: Option<serde_json::Value>,
    /// What the finished file contains, once ready.
    #[graphql(skip)]
    pub probe: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
    pub last_used_at: Option<OffsetDateTime>,
    /// Since when nothing has wanted this finished copy. A copy left unwanted and
    /// unplayed for a day is swept up.
    pub unwanted_since: Option<OffsetDateTime>,
}

#[async_graphql::ComplexObject]
impl OptimizedVersion {
    /// The title this is a version of; null once it's gone, or for a viewer who
    /// can't see it.
    #[graphql(complexity = 3)]
    async fn work(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Option<Work>> {
        let storage = complex_storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        let work = self
            .source_work(storage.as_ref())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        match work {
            Some(work)
                if crate::access::user_can_access_work(storage.as_ref(), user, work.id)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))? =>
            {
                Ok(Some(work))
            }
            _ => Ok(None),
        }
    }

    /// What was done to the source, in words.
    async fn description(&self) -> String {
        describe_variant(&self.variant_key)
    }

    /// Who and which devices this was prepared for, and why. Administrator only.
    #[graphql(complexity = 20)]
    async fn targets(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<OptimizedTarget>> {
        if !crate::access::request_user(ctx)?.is_admin {
            return Err(async_graphql::Error::new("administrator only"));
        }
        self.resolve_targets(complex_storage(ctx)?.as_ref())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Viewers' marks wanting this, which an administrator can take back.
    /// Administrator only.
    #[graphql(complexity = 20)]
    async fn marks(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<OptimizedVersionMark>> {
        if !crate::access::request_user(ctx)?.is_admin {
            return Err(async_graphql::Error::new("administrator only"));
        }
        self.resolve_marks(complex_storage(ctx)?.as_ref())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}

/// A viewer's mark that's keeping a version wanted. A mark on a series or season
/// names that, not the episode the version is of.
#[derive(Debug, Clone, SimpleObject)]
pub struct OptimizedVersionMark {
    pub user: User,
    pub work: Work,
    /// The one device it's for. Null is all of the viewer's with it on.
    pub device: Option<Device>,
    pub marked_at: OffsetDateTime,
}

/// Why a version exists for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum OptimizedReason {
    /// The viewer marked the title, for this device or all of theirs.
    Marked,
    /// The device has smart transcoding or smart downloads on, and the server
    /// expects it to be watched there.
    Predicted,
    /// It was wanted when planned, and neither is true any more.
    Earlier,
}

/// One device a version was prepared for.
#[derive(Debug, Clone, SimpleObject)]
pub struct OptimizedTarget {
    pub user: User,
    /// Null for a viewer who marked it with no device to reach.
    pub device: Option<Device>,
    pub reason: OptimizedReason,
}

impl OptimizedVersion {
    /// The devices the plan names, each with why, and any viewer who marked it
    /// without one. The plan is the transcoder's, so only its device ids are read.
    async fn resolve_targets(
        &self,
        storage: &dyn crate::Storage,
    ) -> crate::StorageResult<Vec<OptimizedTarget>> {
        let marks = storage
            .list_smart_transcode_marks()
            .await?
            .into_iter()
            .filter(|mark| mark.media_file_ids.contains(&self.media_file_id))
            .collect::<Vec<_>>();
        let planned = self.plan["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| serde_json::from_value::<DeviceId>(id.clone()).ok());
        let mut seen = std::collections::HashSet::new();
        let mut targets = Vec::new();
        for id in planned {
            if !seen.insert(id) {
                continue;
            }
            let Some(device) = storage.get_device(id).await? else {
                continue;
            };
            let Some(user) = storage.get_user(device.user_id).await? else {
                continue;
            };
            let marked = marks.iter().any(|mark| {
                mark.user_id == device.user_id && mark.device_id.is_none_or(|one| one == id)
            });
            let reason = if marked {
                OptimizedReason::Marked
            } else if device.smart_transcode || device.smart_downloads {
                OptimizedReason::Predicted
            } else {
                OptimizedReason::Earlier
            };
            targets.push(OptimizedTarget {
                user,
                device: Some(device),
                reason,
            });
        }
        for mark in &marks {
            if targets.iter().any(|target| {
                target.user.id == mark.user_id && target.reason == OptimizedReason::Marked
            }) {
                continue;
            }
            if let Some(user) = storage.get_user(mark.user_id).await? {
                targets.push(OptimizedTarget {
                    user,
                    device: None,
                    reason: OptimizedReason::Marked,
                });
            }
        }
        targets.sort_by(|a, b| {
            let device =
                |target: &OptimizedTarget| target.device.as_ref().map(|device| device.name.clone());
            a.user
                .shown_name()
                .cmp(b.user.shown_name())
                .then_with(|| device(a).cmp(&device(b)))
        });
        Ok(targets)
    }

    async fn resolve_marks(
        &self,
        storage: &dyn crate::Storage,
    ) -> crate::StorageResult<Vec<OptimizedVersionMark>> {
        let mut marks = Vec::new();
        for mark in storage.list_smart_transcode_marks().await? {
            if !mark.media_file_ids.contains(&self.media_file_id) {
                continue;
            }
            let (Some(user), Some(work)) = (
                storage.get_user(mark.user_id).await?,
                storage.get_work(mark.work_id).await?,
            ) else {
                continue;
            };
            let device = match mark.device_id {
                Some(id) => storage.get_device(id).await?,
                None => None,
            };
            marks.push(OptimizedVersionMark {
                user,
                work,
                device,
                marked_at: mark.created_at,
            });
        }
        Ok(marks)
    }

    /// The title whose file this was made from.
    pub async fn source_work(
        &self,
        storage: &dyn crate::Storage,
    ) -> crate::StorageResult<Option<Work>> {
        let Some(media) = storage.media_file_by_id(self.media_file_id).await? else {
            return Ok(None);
        };
        let Some(edition) = storage.get_edition(media.edition_id).await? else {
            return Ok(None);
        };
        storage.get_work(edition.work_id).await
    }
}

/// Words for a variant key, which `OptimizeTarget::key` in `noflex-transcoder`
/// builds: `<video>/<audio>[+<audio>…]`, as `copy-h264/aac2` or
/// `av1-720p-1556k-av1main-tonemap/copy`.
fn describe_variant(key: &str) -> String {
    let (video, audio) = key.split_once('/').unwrap_or((key, ""));
    let codec = |name: &str| match name {
        "h264" => "H.264".to_string(),
        "h265" => "H.265".to_string(),
        other => other.to_uppercase(),
    };
    let converted_audio = audio
        .split('+')
        .find(|track| !matches!(*track, "" | "copy" | "none"))
        .map(|track| {
            let digits = track
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(track.len());
            let (name, channels) = track.split_at(digits);
            let layout = match channels {
                "1" => "mono".to_string(),
                "2" => "stereo".to_string(),
                "6" => "5.1".to_string(),
                "8" => "7.1".to_string(),
                "" => String::new(),
                other => format!("{other}-channel"),
            };
            format!("{} {layout}", codec(name)).trim().to_string()
        });
    let mut parts = video.split('-');
    let first = parts.next().unwrap_or_default();
    if first == "copy" || first == "none" {
        return match converted_audio {
            Some(audio) => format!("Video kept, audio converted to {audio}"),
            None => "Repackaged, nothing re-encoded".to_string(),
        };
    }
    let height = parts.next().unwrap_or_default();
    let tonemapped = video.ends_with("-tonemap");
    let target = format!(
        "{} {height}{}",
        codec(first),
        if tonemapped { " SDR" } else { "" }
    );
    match converted_audio {
        Some(audio) => format!("Converted to {target} with {audio}"),
        None => format!("Converted to {target}, audio kept"),
    }
}

/// How many versions there are of each kind, without reading any of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, SimpleObject)]
pub struct OptimizedVersionStats {
    pub ready: u32,
    pub queued: u32,
    pub running: u32,
    pub failed: u32,
    /// Disk the ready ones use.
    pub ready_bytes: u64,
}

/// The facts recommendations compare a top-level work by.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkFeatures {
    pub work: WorkId,
    pub kind: WorkKind,
    pub year: Option<i32>,
    pub community_rating: Option<f32>,
    /// The longest edition's runtime; `None` for works without their own
    /// editions, such as series.
    pub runtime_ms: Option<i64>,
    pub added_at: OffsetDateTime,
    pub tags: Vec<TagId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaRemoval {
    /// Works pruned because their last media-bearing descendant was removed.
    pub removed_works: Vec<WorkId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, SimpleObject)]
pub struct Library {
    pub id: LibraryId,
    pub name: String,
    pub kind: LibraryKind,
    pub paths: Vec<String>,
    #[serde(default)]
    pub config: LibraryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, SimpleObject)]
pub struct LibraryConfig {
    /// BCP-47 language tags in priority order (e.g. `["en", "ja"]`).
    #[serde(default)]
    pub preferred_languages: Vec<String>,
    /// Provider plugin ids in priority order (e.g. `["tmdb", "tvdb"]`).
    #[serde(default)]
    pub preferred_providers: Vec<String>,
    /// Normalized relative paths under each library root that the scanner
    /// skips recursively (e.g. `["incoming/.staging", "failed"]`).
    #[serde(default)]
    pub excluded_folders: Vec<String>,
    /// When true the long-lived watcher runs against the library roots.
    #[serde(default = "default_true")]
    pub watch_enabled: bool,
    #[serde(default)]
    pub scan_schedule: ScanSchedule,
    /// Which artwork readers see when a work has both provider and
    /// local images of a kind. Read on every image lookup, so changing
    /// it needs no rescan.
    #[serde(default)]
    pub artwork_preference: ArtworkPreference,
    /// Whether this library's works count as anime when browsing by
    /// [`ContentCategory`]. Read at query time, so changing it needs no
    /// rescan.
    #[serde(default)]
    pub content_category: LibraryCategory,
}

fn default_true() -> bool {
    true
}

/// Matches the serde and column defaults: a new library is watched.
impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            preferred_languages: Vec::new(),
            preferred_providers: Vec::new(),
            excluded_folders: Vec::new(),
            watch_enabled: true,
            scan_schedule: ScanSchedule::default(),
            artwork_preference: ArtworkPreference::default(),
            content_category: LibraryCategory::default(),
        }
    }
}

/// Tag namespace for [`ContentCategory`] classification.
pub const CATEGORY_TAG_NAMESPACE: &str = "category";
/// Slug of the tag in [`CATEGORY_TAG_NAMESPACE`] that marks a work as anime.
pub const ANIME_TAG_SLUG: &str = "anime";

/// A library-level override of per-work anime classification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum LibraryCategory {
    /// A work is anime when it carries the anime category tag.
    #[default]
    Auto,
    /// Every work here is anime.
    Anime,
    /// Nothing here is anime, whatever its tags say.
    General,
}

/// What viewers browse by, across libraries. Every top-level work belongs to
/// at most one category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum ContentCategory {
    /// Movies that aren't anime.
    Movies,
    /// Series that aren't anime.
    Shows,
    /// Anime movies and series.
    Anime,
    Music,
    Audiobooks,
    Photos,
}

impl ContentCategory {
    pub const ALL: [Self; 6] = [
        Self::Movies,
        Self::Shows,
        Self::Anime,
        Self::Music,
        Self::Audiobooks,
        Self::Photos,
    ];

    /// The work kinds this category can contain; anime classification
    /// narrows `Movies`, `Shows` and `Anime` further.
    pub fn kinds(self) -> &'static [WorkKind] {
        match self {
            Self::Movies => &[WorkKind::Movie],
            Self::Shows => &[WorkKind::Series],
            Self::Anime => &[WorkKind::Movie, WorkKind::Series],
            Self::Music => &[WorkKind::Artist, WorkKind::Album, WorkKind::Track],
            Self::Audiobooks => &[WorkKind::AudioBook],
            Self::Photos => &[WorkKind::Photo],
        }
    }
}

/// How many top-level works in a category a caller can access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, SimpleObject)]
pub struct ContentCategoryCount {
    pub category: ContentCategory,
    pub work_count: u64,
}

/// Library-level choice between metadata-provider artwork and artwork
/// found next to the media. Either way the other source fills kinds the
/// preferred one lacks, and generated art comes last.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum ArtworkPreference {
    #[default]
    PreferProvider,
    PreferLocal,
}

impl ArtworkPreference {
    /// The image source order readers pass to `Storage::image` and
    /// `Storage::list_images`.
    pub fn source_order(self) -> &'static [ImageSource] {
        match self {
            ArtworkPreference::PreferProvider => &[
                ImageSource::Provider,
                ImageSource::Local,
                ImageSource::Generated,
            ],
            ArtworkPreference::PreferLocal => &[
                ImageSource::Local,
                ImageSource::Provider,
                ImageSource::Generated,
            ],
        }
    }
}

impl ImageTarget {
    /// The source order for reading this target's images: its library's
    /// [`ArtworkPreference`] for works and editions, the default
    /// preference for tags (people span libraries) and for targets whose
    /// work or library no longer exists. Read fresh on every call, so a
    /// changed preference applies at once.
    pub async fn source_order<S: crate::Storage + ?Sized>(
        self,
        storage: &S,
    ) -> crate::StorageResult<&'static [ImageSource]> {
        let work = match self {
            ImageTarget::Work(id) => Some(id),
            ImageTarget::Edition(id) => storage.get_edition(id).await?.map(|e| e.work_id),
            ImageTarget::Tag(_) => None,
        };
        let library = match work {
            Some(id) => storage.get_work(id).await?.map(|w| w.library_id),
            None => None,
        };
        let preference = match library {
            Some(id) => storage
                .get_library(id)
                .await?
                .map(|l| l.config.artwork_preference)
                .unwrap_or_default(),
            None => ArtworkPreference::default(),
        };
        Ok(preference.source_order())
    }
}

/// Batches library lookups for resolvers that need a library's config
/// per row (e.g. `Work.images` reading the artwork preference). One
/// listing serves a whole batch; missing libraries are absent.
pub struct LibraryLoader {
    storage: std::sync::Arc<dyn crate::Storage>,
}

impl LibraryLoader {
    /// Loads through `storage`; holds no cache across requests.
    pub fn new(storage: std::sync::Arc<dyn crate::Storage>) -> Self {
        Self { storage }
    }
}

impl async_graphql::dataloader::Loader<LibraryId> for LibraryLoader {
    type Value = Library;
    type Error = String;

    async fn load(
        &self,
        keys: &[LibraryId],
    ) -> Result<std::collections::HashMap<LibraryId, Library>, Self::Error> {
        Ok(self
            .storage
            .list_libraries()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|lib| keys.contains(&lib.id))
            .map(|lib| (lib.id, lib))
            .collect())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
pub enum ScanSchedule {
    #[default]
    Disabled,
    Hourly,
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Enum)]
pub enum LibraryKind {
    #[default]
    Movies,
    Shows,
    Music,
    Audiobooks,
    Photos,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct Tag {
    pub id: TagId,
    pub namespace: String,
    pub parent_id: Option<TagId>,
    pub slug: String,
    pub display: String,
    /// Free-form per-tag metadata (e.g., role/character for people, photo URL).
    // `Json` is `#[serde(transparent)]`, so the stored column shape is
    // unchanged.
    pub metadata: async_graphql::Json<serde_json::Value>,
}

/// A tag with its work-assignment count — the browse surface for one
/// namespace (genres, people, …). `work_count` counts only
/// `target_kind = 'work'` assignments.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct TagSummary {
    pub tag: Tag,
    pub work_count: u32,
}

#[async_graphql::ComplexObject]
impl TagSummary {
    /// The tag's id, which a listing shows once, so it identifies the summary too.
    async fn id(&self) -> TagId {
        self.tag.id
    }
}

/// One tag assignment row for a target: the tag plus the weight and
/// provenance source recorded when it was attached. Internal read type
/// — provider refresh replaces only `"provider:*"`-sourced assignments,
/// leaving user-set ones alone — so it carries no GQL derive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagAssignment {
    pub tag: Tag,
    pub weight: f32,
    pub source: String,
    /// What's true of the tag on this target alone; `{}` when nothing is.
    /// People carry `order` and `credits`: see [`Credit`].
    pub metadata: async_graphql::Json<serde_json::Value>,
}

/// One part a person played in a work. Someone who directed and starred has
/// two.
#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct Credit {
    /// The person's tag; its `works` is their filmography.
    pub person_id: TagId,
    pub name: String,
    /// `Director`, `Writer`, `Producer`, `Composer` or `Actor`, as the provider
    /// reports it. Null when it doesn't say.
    pub role: Option<String>,
    pub character: Option<String>,
}

impl Credit {
    /// `assignments`' people as credits in billing order. People assigned before
    /// assignments carried credits have only the role on their tag, which may be
    /// from another work, so it's the role alone and they sort last.
    pub fn from_assignments(assignments: Vec<TagAssignment>) -> Vec<Self> {
        let text = |value: &serde_json::Value, key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .map(str::to_string)
        };
        let mut people = assignments
            .into_iter()
            .filter(|assignment| assignment.tag.namespace == "person")
            .collect::<Vec<_>>();
        people.sort_by_key(|assignment| {
            (
                assignment
                    .metadata
                    .0
                    .get("order")
                    .and_then(|order| order.as_u64())
                    .unwrap_or(u64::MAX),
                assignment.tag.display.clone(),
            )
        });
        let mut credits = Vec::new();
        for person in people {
            let parts = person.metadata.0.get("credits").and_then(|c| c.as_array());
            let credit = |role, character| Credit {
                person_id: person.tag.id,
                name: person.tag.display.clone(),
                role,
                character,
            };
            match parts {
                Some(parts) => credits.extend(
                    parts
                        .iter()
                        .map(|part| credit(text(part, "role"), text(part, "character"))),
                ),
                None => credits.push(credit(text(&person.tag.metadata.0, "role"), None)),
            }
        }
        credits
    }
}

/// What an image is attached to. Broader than `TagTarget` because images also
/// hang off people (a `Tag` in the `person` namespace).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ImageTarget {
    Work(WorkId),
    Edition(EditionId),
    Tag(TagId),
}

impl ImageTarget {
    pub fn kind_str(&self) -> &'static str {
        match self {
            ImageTarget::Work(_) => "work",
            ImageTarget::Edition(_) => "edition",
            ImageTarget::Tag(_) => "tag",
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            ImageTarget::Work(id) => id.into_uuid(),
            ImageTarget::Edition(id) => id.into_uuid(),
            ImageTarget::Tag(id) => id.into_uuid(),
        }
    }
}

/// Where an image came from. A target holds at most one image per
/// (kind, source), so local and provider artwork of one kind coexist and
/// readers pick between them by a source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum ImageSource {
    /// Found next to the media by the scanner; owned by scan reconcile.
    Local,
    /// Fetched from a metadata provider plugin into the image cache.
    Provider,
    /// Rendered by the server (e.g. episode frame grabs) as a last resort.
    Generated,
}

impl ImageSource {
    /// Every source, local first: the order readers used before sources
    /// existed (a local file always won).
    pub const ALL: &'static [ImageSource] = &[
        ImageSource::Local,
        ImageSource::Provider,
        ImageSource::Generated,
    ];

    /// The lowercase name stored by backends and used in `?source=`.
    pub fn as_str(self) -> &'static str {
        match self {
            ImageSource::Local => "local",
            ImageSource::Provider => "provider",
            ImageSource::Generated => "generated",
        }
    }

    /// Parses [`ImageSource::as_str`]'s spelling; `None` for anything else.
    // Inherent rather than `FromStr`, matching `ImageKind::from_str`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "local" => ImageSource::Local,
            "provider" => ImageSource::Provider,
            "generated" => ImageSource::Generated,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct Image {
    // The route segments derive from the target; exposing the tagged
    // enum itself would leak an input-ish union onto the read surface.
    #[graphql(skip)]
    pub target: ImageTarget,
    pub kind: ImageKind,
    pub source: ImageSource,
    pub path: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub language: Option<String>,
    /// The plugin behind a [`ImageSource::Provider`] row; `None` for any
    /// other source. Clients read [`Work::attributions`].
    #[graphql(skip)]
    pub provider: Option<String>,
}

/// How a metadata provider asks to be credited wherever its data shows.
// The logo's bytes stay out of this row: clients fetch them from `logo_url`,
// and `Storage::provider_attribution_logo` reads them for that route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct ProviderAttribution {
    /// The plugin's id, as in `MetadataMatch.provider` and `Plugin.id`.
    pub provider: String,
    /// The plugin's display name.
    pub name: String,
    /// The credit, shown as the provider wrote it.
    pub text: String,
    /// Where following the credit leads.
    pub url: Option<String>,
    /// `image/svg+xml` or a raster type; null without a logo.
    pub logo_mime: Option<String>,
    /// Changes with the logo's bytes, so its URL can be cached for good.
    #[graphql(skip)]
    pub logo_digest: Option<String>,
}

#[async_graphql::ComplexObject]
impl ProviderAttribution {
    /// Where the logo is served from, relative to this server. It needs no
    /// credential, so `<img src>` works as is. Null without a logo.
    async fn logo_url(&self) -> Option<String> {
        self.logo_digest
            .as_ref()
            .map(|digest| format!("/attributions/{}/logo?v={digest}", self.provider))
    }
}

/// A plugin's logo, for the route that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionLogo {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl Image {
    /// Keeps, per kind, the row from the earliest source in `order`,
    /// dropping rows whose source `order` omits. Output is ordered by kind
    /// text, the order [`crate::Storage::list_images`] promises. Backends
    /// that cannot resolve in the query share this so every engine picks
    /// identically.
    pub fn resolve_effective(rows: Vec<Image>, order: &[ImageSource]) -> Vec<Image> {
        let rank = |source: ImageSource| order.iter().position(|s| *s == source);
        let mut best: Vec<(usize, Image)> = Vec::new();
        for row in rows {
            let Some(r) = rank(row.source) else {
                continue;
            };
            match best.iter_mut().find(|(_, b)| b.kind == row.kind) {
                Some(slot) if r < slot.0 => *slot = (r, row),
                Some(_) => {}
                None => best.push((r, row)),
            }
        }
        let mut out: Vec<Image> = best.into_iter().map(|(_, image)| image).collect();
        out.sort_by(|a, b| a.kind.as_str().cmp(b.kind.as_str()));
        out
    }
}

#[async_graphql::ComplexObject]
impl Image {
    /// Serving route for this image, pinned to its source with
    /// `?source=` (so the URL changes when the effective source does)
    /// and carrying a `&token=` that authenticates the caller without a
    /// header (so `<img src>` works) and stays identical for about an
    /// hour so browsers can cache it. Clients append `&w=`/`&h=` (and
    /// optionally `&fill=true`) to resize server-side. Without a
    /// configured signer the token is omitted and the route needs a
    /// header credential.
    async fn url(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<String> {
        let storage = complex_storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        let visible = match self.target {
            ImageTarget::Work(id) => {
                crate::access::user_can_access_work(storage.as_ref(), user, id).await
            }
            ImageTarget::Edition(id) => {
                crate::access::user_can_access_edition(storage.as_ref(), user, id).await
            }
            ImageTarget::Tag(_) => Ok(true),
        }
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if !visible {
            return Err(async_graphql::Error::new("not found"));
        }
        let path = format!(
            "/images/{}/{}/{}?source={}",
            self.target.kind_str(),
            self.target.id(),
            self.kind.as_str(),
            self.source.as_str()
        );
        let Some(signer) = ctx.data_opt::<std::sync::Arc<dyn MediaUrlSigner>>() else {
            return Ok(path);
        };
        let token = signer
            .sign_image_token(
                user.user_id,
                crate::access::caller_session(ctx),
                self.target,
            )
            .map_err(async_graphql::Error::new)?;
        Ok(format!("{path}&token={}", percent_encode(&token)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct Session {
    pub id: SessionId,
    pub user_id: UserId,
    pub client: Option<String>,
    pub device: Option<String>,
    pub device_id: Option<String>,
    pub app_version: Option<String>,
    pub remote_addr: Option<String>,
    pub created_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    /// Absolute expiry of the session's refresh token; past this the
    /// session is dead even if nobody explicitly revoked it.
    pub expires_at: OffsetDateTime,
    /// SHA-256 hex of the current refresh token. Rotated on every
    /// refresh; `None` means the session can't be refreshed. Never
    /// leaves the server — skipped on the GQL surface.
    #[graphql(skip)]
    pub refresh_token_hash: Option<String>,
    /// When the session was left waiting for its user's PIN; unset while it's
    /// open. A locked session authenticates nothing and can't refresh until the
    /// PIN opens it. Locking and unlocking change it; saving the session never
    /// does.
    pub locked_at: Option<OffsetDateTime>,
}

/// What a refresh-token hash resolves to. See
/// [`Storage::session_by_refresh_token`](crate::Storage::session_by_refresh_token).
#[derive(Debug, Clone)]
pub enum RefreshTokenLookup {
    /// The hash is the session's current refresh token.
    Current(Session),
    /// The hash was once current for a live session and has since been
    /// rotated away.
    Retired(RetiredRefreshToken),
    /// Never issued, or its session no longer exists.
    Unknown,
}

/// A refresh token its session has rotated away from.
#[derive(Debug, Clone)]
pub struct RetiredRefreshToken {
    /// The owning session as it stands now, current hash included.
    pub session: Session,
    /// When the rotation that retired this token committed.
    pub retired_at: OffsetDateTime,
    /// Whether no later rotation has happened, i.e. the session's current
    /// token directly replaced this one.
    pub latest: bool,
}

/// A long-lived programmatic credential. The key hash never leaves the
/// backend, so it isn't on this type — authentication resolves a row by
/// hash and the caller checks `revoked_at`.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct ApiKey {
    pub id: ApiKeyId,
    pub user_id: UserId,
    pub name: String,
    pub created_at: OffsetDateTime,
    /// `None` while the key is active; `Some` stamps the revocation.
    pub revoked_at: Option<OffsetDateTime>,
}

/// A pending device-linking request (Plex-PIN-style). `user_id` is
/// `None` until a signed-in user authorizes the code; past `expires_at`
/// the row is dead regardless of authorization state.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct DeviceCode {
    pub id: DeviceCodeId,
    pub code: String,
    pub user_id: Option<UserId>,
    pub client: Option<String>,
    pub device: Option<String>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanEvent {
    pub library_id: LibraryId,
    pub started_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
    pub added: u32,
    pub updated: u32,
    pub removed: u32,
    pub errored: u32,
}

/// Append-only activity log entry. Server actions, scan
/// events, and play events all funnel through this shape.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct ActivityEntry {
    #[graphql(skip_output, derived(owned, name = "id", into = "async_graphql::ID"))]
    pub id: i64,
    pub kind: ActivityKind,
    pub actor: Option<String>,
    pub subject: Option<String>,
    pub detail: Option<String>,
    pub occurred_at: OffsetDateTime,
}

/// One row of a per-user play history page. `target_kind`
/// and `target_id` together identify the Edition or Work the play
/// attached to (split for GQL-friendliness); `data` is the full
/// `UserData` snapshot at page-read time. A viewer has one row per
/// target, so the target's id, exposed as `id`, identifies the row too.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(name = "HistoryRow", complex)]
pub struct HistoryRow {
    pub target_kind: PlayTargetKind,
    #[graphql(skip_output, derived(owned, name = "id", into = "async_graphql::ID"))]
    pub target_id: Uuid,
    pub data: UserData,
    /// Backend-minted position just after this row; passing it to
    /// `Storage::list_play_history` resumes with the next older row.
    #[graphql(skip)]
    #[serde(skip)]
    pub cursor: crate::query::PageCursor,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryPage {
    pub rows: Vec<HistoryRow>,
    /// Opaque cursor for the page after this one; `None` when no
    /// older rows remain.
    pub next_cursor: Option<String>,
}

/// Canonical representation of a library scan's status. Rendered
/// as a flat GQL object with the closed [`ScanState`] enum, RFC 3339
/// timestamp strings, and an optional [`ScanSummary`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, SimpleObject)]
#[graphql(name = "LibraryScanStatus")]
pub struct LibraryScanStatus {
    pub state: ScanState,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub summary: Option<ScanSummary>,
    pub error: Option<String>,
}

/// Per-scan delta counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, SimpleObject)]
pub struct ScanSummary {
    pub added: i32,
    pub updated: i32,
    pub removed: i32,
    pub errored: i32,
}

/// Taxonomy of append-only activity log entries. The
/// GraphQL `Activity.kind` field renders this as a GQL enum
/// (`SCAN_COMPLETED`, `PLAY_PROGRESS`, …). `Custom` is the
/// catch-all for plugins that emit their own event names —
/// the GQL `From<String>` falls through to it.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum ActivityKind {
    ScanStarted,
    ScanCompleted,
    PlayStarted,
    PlayProgress,
    PlayCompleted,
    ItemAdded,
    ItemUpdated,
    ItemRemoved,
    LibraryCreated,
    LibraryUpdated,
    LibraryDeleted,
    /// A rotated-away refresh token was presented; its session was
    /// revoked. `actor` is the session's user, `subject` the session.
    RefreshTokenReuse,
    /// An administrator approved a device code as someone else, signing a
    /// device in for them. `actor` is the administrator, `subject` the
    /// user the device now belongs to.
    DeviceApprovedFor,
    Custom,
}

impl ActivityKind {
    /// Wire/spelling form. Storage persists the `to_string()` of this
    /// (e.g. `scan.started`) so the GQL filter on the activity query can
    /// compare against the canonical variant.
    pub fn as_str(self) -> &'static str {
        match self {
            ActivityKind::ScanStarted => "scan.started",
            ActivityKind::ScanCompleted => "scan.completed",
            ActivityKind::PlayStarted => "play.started",
            ActivityKind::PlayProgress => "play.progress",
            ActivityKind::PlayCompleted => "play.completed",
            ActivityKind::ItemAdded => "item.added",
            ActivityKind::ItemUpdated => "item.updated",
            ActivityKind::ItemRemoved => "item.removed",
            ActivityKind::LibraryCreated => "library.created",
            ActivityKind::LibraryUpdated => "library.updated",
            ActivityKind::LibraryDeleted => "library.deleted",
            ActivityKind::RefreshTokenReuse => "auth.refresh_token_reuse",
            ActivityKind::DeviceApprovedFor => "auth.device_approved_for",
            ActivityKind::Custom => "custom",
        }
    }
}

impl std::str::FromStr for ActivityKind {
    type Err = std::convert::Infallible;
    /// Parse a DB-stored kind back into the typed enum. Accepts
    /// both the canonical snake_case spelling written by
    /// [`ActivityKind::as_str`] and a few legacy PascalCase /
    /// dotted variants that older noflex builds and a legacy plugin SDK wrote
    /// to the `activity_log.kind` column.
    /// Unknown values fall through to [`ActivityKind::Custom`]
    /// so a stray plugin-emitted row never breaks the activity
    /// query.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "ScanStarted" | "scan.started" => ActivityKind::ScanStarted,
            "ScanCompleted" | "scan.completed" | "scan.complete" => ActivityKind::ScanCompleted,
            "PlayStarted" | "play.started" => ActivityKind::PlayStarted,
            "PlayProgress" | "play.progress" => ActivityKind::PlayProgress,
            "PlayCompleted" | "play.completed" => ActivityKind::PlayCompleted,
            "ItemAdded" | "item.added" => ActivityKind::ItemAdded,
            "ItemUpdated" | "item.updated" => ActivityKind::ItemUpdated,
            "ItemRemoved" | "item.removed" => ActivityKind::ItemRemoved,
            "LibraryCreated" | "library.created" => ActivityKind::LibraryCreated,
            "LibraryUpdated" | "library.updated" => ActivityKind::LibraryUpdated,
            "LibraryDeleted" | "library.deleted" => ActivityKind::LibraryDeleted,
            "auth.refresh_token_reuse" => ActivityKind::RefreshTokenReuse,
            "auth.device_approved_for" => ActivityKind::DeviceApprovedFor,
            _ => ActivityKind::Custom,
        })
    }
}

/// GraphQL mirror of [`noflex_scanner::ScanStatus`]'s variant
/// tag. The full status (timestamps, summary, error message)
/// is rendered on `LibraryScanStatus` itself; this enum is
/// the closed set of legal values for the `state` field.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum ScanState {
    #[default]
    Idle,
    Running,
    Completed,
    Failed,
}

// ---- Plex-style export jobs --------------------------------------------
//
// A user kicks off an `exportJob(editionId, format=MP4)` mutation; the
// background worker (`noflex-server::download::DownloadWorker`) consumes
// the queue, runs ffmpeg, and stamps the job with `output_path` on
// completion. The user then hits `GET /download/{job_id}/file` (with
// HTTP range support) to pull the bytes for offline playback.
//
// Mirrors Plex's `/SyncJobs` endpoint.

/// Container the user asked the worker to produce. MP4 is the
/// only "fast" target — copy-remux with `+faststart` for direct
/// browser playback. MKV is allowed for users who want lossless
/// archival; it goes through the same remux path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[graphql(name = "MP4")]
    Mp4,
    Mkv,
}

/// Closed set of durable queue states. `CancelRequested` keeps cancellation
/// observable until the worker holding the lease has stopped its subprocess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum ExportJobStatus {
    Queued,
    Running,
    CancelRequested,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct ExportJob {
    pub id: ExportJobId,
    pub user_id: UserId,
    pub edition_id: EditionId,
    pub format: ExportFormat,
    pub status: ExportJobStatus,
    /// 0.0..1.0; the worker updates this every progress tick
    /// (every ~5s) by reading ffmpeg's `time=` stderr field.
    pub progress: f32,
    /// Absolute path to the produced file. Set by the worker on
    /// successful completion; `None` while queued/running.
    pub output_path: Option<String>,
    /// Worker-stamped failure message. `None` unless status is
    /// `Failed`. Truncated stderr from the ffmpeg subprocess.
    pub error: Option<String>,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
}

// ---- Collections and playlists -----------------------------------------
//
// Collections are admin-curated, library-scoped groupings of works
// ("Studio Ghibli", "Oscar Winners 2024"); membership is an explicit
// ordered list, not a saved filter. Playlists are user-owned ordered
// sequences of play targets (works or editions). Both keep their item
// lists in side tables (`collection_items` / `playlist_items`) so the
// row types here stay flat.

/// An admin-curated grouping of works within one library. The ordered
/// membership is exposed on GQL as the complex `items` field below,
/// resolved through `Storage::collection_items` — not part of this
/// row type.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct Collection {
    pub id: CollectionId,
    pub library_id: LibraryId,
    pub title: String,
    pub sort_title: String,
    pub overview: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A user-owned ordered list of play targets. Like [`Collection`], the
/// items live outside the row and surface on GQL as a complex field.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct Playlist {
    pub id: PlaylistId,
    pub owner_id: UserId,
    pub title: String,
    pub overview: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// One resolved playlist entry. Exactly one of `work` / `edition` is
/// set; `kind` says which, so clients don't have to probe both.
/// Items live as bare play targets in storage; this is the hydrated
/// GQL read shape. A playlist holds a target once, so the target's id,
/// exposed as `id`, identifies the entry too.
#[derive(Debug, Clone, SimpleObject)]
pub struct PlaylistItem {
    #[graphql(skip_output, derived(owned, name = "id", into = "async_graphql::ID"))]
    pub target_id: Uuid,
    pub kind: PlayTargetKind,
    pub work: Option<Work>,
    pub edition: Option<Edition>,
}

// The complex fields pull `Arc<dyn Storage>` out of the request
// context, same as a resolver — the orphan rule forces them to live
// here rather than in `noflex-graphql`, because
// `impl ComplexObject for Collection` needs a local type.
// `pub(crate)` so other modules in this crate (e.g. `search`) can
// reuse the same wiring for their ComplexObject impls.
pub(crate) fn complex_storage<'a>(
    ctx: &'a async_graphql::Context<'a>,
) -> async_graphql::Result<&'a std::sync::Arc<dyn crate::Storage>> {
    ctx.data::<std::sync::Arc<dyn crate::Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

/// Batches edition → owning-work lookups for resolvers that hop from an
/// edition id to its work (e.g. `HistoryRow.work`). Missing editions are
/// absent from the result map. No access filtering: pair with
/// [`crate::search::WorkLoader`], which applies it.
pub struct EditionWorkLoader {
    storage: std::sync::Arc<dyn crate::Storage>,
}

impl EditionWorkLoader {
    pub fn new(storage: std::sync::Arc<dyn crate::Storage>) -> Self {
        Self { storage }
    }
}

impl async_graphql::dataloader::Loader<EditionId> for EditionWorkLoader {
    type Value = WorkId;
    type Error = String;

    async fn load(
        &self,
        keys: &[EditionId],
    ) -> Result<std::collections::HashMap<EditionId, WorkId>, Self::Error> {
        Ok(self
            .storage
            .get_editions(keys)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|edition| (edition.id, edition.work_id))
            .collect())
    }
}

/// `work_id` hydrated through the request's access-filtering
/// [`crate::search::WorkLoader`]; `None` when missing or not visible.
async fn load_visible_work(
    ctx: &async_graphql::Context<'_>,
    work_id: WorkId,
) -> async_graphql::Result<Option<Work>> {
    let loader = ctx
        .data::<async_graphql::dataloader::DataLoader<crate::search::WorkLoader>>()
        .map_err(|_| async_graphql::Error::new("work loader not configured"))?;
    loader
        .load_one(crate::search::WorkLoadKey {
            work_id,
            user: crate::access::request_user(ctx)?,
            access_checked: false,
        })
        .await
        .map_err(async_graphql::Error::new)
        .map(Option::flatten)
}

#[async_graphql::ComplexObject]
impl Work {
    /// Who to credit for what this work shows: the provider of its metadata,
    /// then the providers of any artwork in use from elsewhere. A provider
    /// that asked for no credit is left out.
    #[graphql(complexity = 5)]
    async fn attributions(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<ProviderAttribution>> {
        crate::access::request_user(ctx)?;
        let storage = complex_storage(ctx)?;
        let mut providers: Vec<String> = self.metadata_provider.iter().cloned().collect();
        let library = storage
            .get_library(self.library_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let order = library
            .map(|library| library.config.artwork_preference)
            .unwrap_or_default();
        let images = storage
            .list_images(ImageTarget::Work(self.id), order.source_order())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        for provider in images.into_iter().filter_map(|image| image.provider) {
            if !providers.contains(&provider) {
                providers.push(provider);
            }
        }
        if providers.is_empty() {
            return Ok(Vec::new());
        }
        let known = storage
            .list_provider_attributions()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(providers
            .iter()
            .filter_map(|id| known.iter().find(|a| a.provider == *id).cloned())
            .collect())
    }

    /// People credited on this work, in billing order, one entry per part.
    /// Empty when no provider named any.
    #[graphql(complexity = 5)]
    async fn credits(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<Credit>> {
        crate::access::request_user(ctx)?;
        let assignments = complex_storage(ctx)?
            .tag_assignments_for_target(TagTarget::Work(self.id))
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(Credit::from_assignments(assignments))
    }

    /// This work's tags in the given namespaces (`genre`, `parental`, `studio`,
    /// …), ordered by namespace then name.
    #[graphql(complexity = 5)]
    async fn tags(
        &self,
        ctx: &async_graphql::Context<'_>,
        namespaces: Vec<String>,
    ) -> async_graphql::Result<Vec<Tag>> {
        crate::access::request_user(ctx)?;
        let mut tags = complex_storage(ctx)?
            .tags_for_target(TagTarget::Work(self.id))
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        tags.retain(|tag| namespaces.contains(&tag.namespace));
        tags.sort_by(|a, b| (&a.namespace, &a.display).cmp(&(&b.namespace, &b.display)));
        Ok(tags)
    }

    /// The effective image of each kind attached to this work (posters,
    /// backdrops, logos, …) under its library's artwork preference,
    /// ordered by kind.
    #[graphql(complexity = 2)]
    async fn images(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Vec<Image>> {
        crate::access::request_user(ctx)?;
        let preference =
            match ctx.data_opt::<async_graphql::dataloader::DataLoader<LibraryLoader>>() {
                Some(loader) => loader
                    .load_one(self.library_id)
                    .await
                    .map_err(async_graphql::Error::new)?
                    .map(|lib| lib.config.artwork_preference)
                    .unwrap_or_default(),
                None => complex_storage(ctx)?
                    .get_library(self.library_id)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?
                    .map(|lib| lib.config.artwork_preference)
                    .unwrap_or_default(),
            };
        complex_storage(ctx)?
            .list_images(ImageTarget::Work(self.id), preference.source_order())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// The requesting user's play state for this work itself (not its
    /// editions). Null when the request is unauthenticated.
    #[graphql(complexity = 2)]
    async fn user_data(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<UserData>> {
        let Ok(user) = crate::access::request_user(ctx) else {
            return Ok(None);
        };
        complex_storage(ctx)?
            .user_data(user.user_id, noflex_core::PlayTarget::Work(self.id))
            .await
            .map(Some)
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Runtime of the work's first edition (in `editions` order), so
    /// clients can render `userData.positionMs / runtimeMs`. Null when
    /// the work has no edition or the first has no known runtime.
    #[graphql(complexity = 2)]
    async fn runtime_ms(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<i64>> {
        crate::access::request_user(ctx)?;
        Ok(complex_storage(ctx)?
            .editions_for_work(self.id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .first()
            .and_then(|edition| edition.runtime_ms))
    }

    /// Provider IDs of the series this episode belongs to, which identify it
    /// across servers together with its season and episode numbers. Empty
    /// for anything but an episode. Paths and other server-local IDs never
    /// appear.
    #[graphql(complexity = 2)]
    async fn series_external_ids(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<ExternalId>> {
        crate::access::request_user(ctx)?;
        let series = self
            .series_of_episode(complex_storage(ctx)?.as_ref())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(series
            .map(|series| series.provider_ids())
            .unwrap_or_default())
    }

    /// Title of the series this episode belongs to, for showing an episode
    /// away from its series. Null for anything but an episode.
    #[graphql(complexity = 2)]
    async fn series_title(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<String>> {
        crate::access::request_user(ctx)?;
        let series = self
            .series_of_episode(complex_storage(ctx)?.as_ref())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(series.map(|series| series.title))
    }

    /// What the server is still doing to this newly found work; null once
    /// it has done what it can, whether or not it found a match. Read from
    /// memory, so it costs a list of cards nothing.
    #[graphql(complexity = 0)]
    async fn indexing(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> Option<crate::indexing::IndexingStage> {
        ctx.data_opt::<std::sync::Arc<crate::indexing::IndexingProgress>>()?
            .stage(self)
    }
}

impl Work {
    /// Unconfirmed, of a kind providers match automatically, and without
    /// provider ids: what the metadata sweep looks for.
    pub fn awaits_identification(&self) -> bool {
        self.metadata_status == MetadataStatus::Unconfirmed
            && matches!(
                self.kind,
                WorkKind::Movie
                    | WorkKind::Series
                    | WorkKind::Season
                    | WorkKind::Episode
                    | WorkKind::Artist
                    | WorkKind::Album
                    | WorkKind::AudioBook
            )
            && self
                .external_ids
                .iter()
                .all(|id| id.kind == ExternalIdKind::Custom)
    }

    /// External IDs that mean the same title on any server.
    pub fn provider_ids(&self) -> Vec<ExternalId> {
        self.external_ids
            .iter()
            .filter(|id| id.kind != ExternalIdKind::Custom)
            .cloned()
            .collect()
    }

    /// The series above an episode, through its season. `None` for anything else.
    pub async fn series_of_episode(
        &self,
        storage: &dyn crate::Storage,
    ) -> crate::StorageResult<Option<Work>> {
        let (WorkKind::Episode, Some(season)) = (self.kind, self.parent_id) else {
            return Ok(None);
        };
        match storage
            .get_work(season)
            .await?
            .and_then(|season| season.parent_id)
        {
            Some(series) => storage.get_work(series).await,
            None => Ok(None),
        }
    }
}

#[async_graphql::ComplexObject]
impl HistoryRow {
    /// The work this row refers to — for an edition target, that
    /// edition's work. Null when it no longer exists or the viewer
    /// can't access it.
    async fn work(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Option<Work>> {
        let work_id = match self.target_kind {
            PlayTargetKind::Work => WorkId::from_uuid(self.target_id),
            PlayTargetKind::Edition => {
                let loader = ctx
                    .data::<async_graphql::dataloader::DataLoader<EditionWorkLoader>>()
                    .map_err(|_| async_graphql::Error::new("edition loader not configured"))?;
                let Some(work_id) = loader
                    .load_one(EditionId::from_uuid(self.target_id))
                    .await
                    .map_err(async_graphql::Error::new)?
                else {
                    return Ok(None);
                };
                work_id
            }
        };
        load_visible_work(ctx, work_id).await
    }
}

#[async_graphql::ComplexObject]
impl WatchRoom {
    /// The work behind this room's media file. Null when it no longer
    /// exists or the viewer can't access it.
    async fn work(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Option<Work>> {
        let storage = complex_storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        let Some(work) = storage
            .work_for_media_file(self.media_file_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        else {
            return Ok(None);
        };
        crate::access::visible_work(storage.as_ref(), user, work)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Who's in the room, host first and then in the order they joined. Only a
    /// member is told.
    #[graphql(complexity = 2)]
    async fn members(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<WatchRoomMember>> {
        let user = crate::access::request_user(ctx)?;
        let mut members = complex_storage(ctx)?
            .list_watch_room_members(self.id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if !members.iter().any(|member| member.user_id == user.user_id) {
            return Ok(Vec::new());
        }
        members.sort_by_key(|member| (member.user_id != self.host_user_id, member.joined_at));
        Ok(members)
    }
}

#[async_graphql::ComplexObject]
impl WatchRoomMember {
    /// Null if the account is gone.
    async fn name(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<String>> {
        crate::access::request_user(ctx)?;
        Ok(complex_storage(ctx)?
            .get_user(self.user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .map(|user| user.shown_name().to_string()))
    }

    /// Their picture, as on [`User`]. Null without one, or if the account is gone.
    async fn avatar_url(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<String>> {
        crate::access::request_user(ctx)?;
        let user = complex_storage(ctx)?
            .get_user(self.user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        match user {
            Some(user) => user.signed_avatar_url(ctx),
            None => Ok(None),
        }
    }
}

#[async_graphql::ComplexObject]
impl ExportJob {
    /// The work this was exported from. Null when it no longer exists or
    /// the viewer can't access it.
    async fn work(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Option<Work>> {
        let storage = complex_storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        let Some(edition) = storage
            .get_edition(self.edition_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        else {
            return Ok(None);
        };
        let Some(work) = storage
            .get_work(edition.work_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        else {
            return Ok(None);
        };
        crate::access::visible_work(storage.as_ref(), user, work)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}

#[async_graphql::ComplexObject]
impl User {
    /// What to call them: the name they've chosen, or their username.
    async fn name(&self) -> &str {
        self.shown_name()
    }

    /// Their picture, 512 pixels square, relative to this server, and a new URL
    /// when it changes. Carries a token as [`Image`]'s does, so `<img src>` works.
    /// Null without one, for clients to fall back to `name`'s initials.
    async fn avatar_url(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<String>> {
        self.signed_avatar_url(ctx)
    }

    /// Libraries this user has been explicitly granted, by id.
    /// Self-serve via `viewer { libraryAccess }`; admins get the same
    /// grants joined to full rows through `userLibraryAccess`.
    async fn library_access(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<LibraryId>> {
        complex_storage(ctx)?
            .list_user_libraries(self.id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}

#[async_graphql::ComplexObject]
impl Tag {
    /// Works carrying this tag (a person's filmography, a genre's
    /// members, …), newest `addedAt` first like the top-level `works`
    /// query. `first` defaults to 50, max 200.
    #[graphql(complexity = "crate::connection::cost(first, 50, 200, child_complexity)")]
    async fn works(
        &self,
        ctx: &async_graphql::Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Work>> {
        let window = crate::connection::Window::new(first, after, 50, 200)?;
        let storage = complex_storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        let resumed = window.after.is_some();
        let page = storage
            .list_works(&crate::query::ListWorksQuery {
                access_user_id: user.access_user_id(),
                tag: Some(self.id),
                recursive: true,
                limit: window.first,
                cursor: window.after,
                ..Default::default()
            })
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(crate::connection::from_item_page(page, resumed))
    }
}

#[async_graphql::ComplexObject]
impl Collection {
    /// Visible members in membership order. `first` defaults to
    /// `ITEM_PAGE_DEFAULT` (50), max `ITEM_PAGE_MAX` (100).
    #[graphql(
        complexity = "crate::connection::cost(first, crate::query::ITEM_PAGE_DEFAULT, crate::query::ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn items(
        &self,
        ctx: &async_graphql::Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Work>> {
        let window = crate::connection::Window::new(
            first,
            after,
            crate::query::ITEM_PAGE_DEFAULT,
            crate::query::ITEM_PAGE_MAX,
        )?;
        let storage = complex_storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        if !user.is_admin
            && !storage
                .list_user_libraries(user.user_id)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?
                .contains(&self.library_id)
        {
            return Ok(crate::connection::empty());
        }
        let resumed = window.after.is_some();
        let page = storage
            .collection_items(self.id, user, window.first, window.after)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let works = storage
            .get_works(&page.items)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let by_id = works
            .into_iter()
            .map(|work| (work.id, work))
            .collect::<std::collections::HashMap<_, _>>();
        let edges = page
            .item_cursors
            .into_iter()
            .zip(page.items)
            .map(|(cursor, id)| {
                by_id
                    .get(&id)
                    .cloned()
                    .map(|work| (cursor.0, work))
                    .ok_or_else(|| async_graphql::Error::new("collection item disappeared"))
            })
            .collect::<async_graphql::Result<Vec<_>>>()?;
        Ok(crate::connection::from_edges(edges, resumed, page.has_next))
    }
}

#[async_graphql::ComplexObject]
impl Playlist {
    /// Visible entries in playlist order, hydrated. `first` defaults to
    /// `ITEM_PAGE_DEFAULT` (50), max `ITEM_PAGE_MAX` (100).
    #[graphql(
        complexity = "crate::connection::cost(first, crate::query::ITEM_PAGE_DEFAULT, crate::query::ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn items(
        &self,
        ctx: &async_graphql::Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, PlaylistItem>> {
        let window = crate::connection::Window::new(
            first,
            after,
            crate::query::ITEM_PAGE_DEFAULT,
            crate::query::ITEM_PAGE_MAX,
        )?;
        let storage = complex_storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        if !user.is_admin && self.owner_id != user.user_id {
            return Ok(crate::connection::empty());
        }
        let resumed = window.after.is_some();
        let page = storage
            .playlist_items(self.id, user, window.first, window.after)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let work_ids = page
            .items
            .iter()
            .filter_map(|target| match target {
                noflex_core::PlayTarget::Work(id) => Some(*id),
                noflex_core::PlayTarget::Edition(_) => None,
            })
            .collect::<Vec<_>>();
        let edition_ids = page
            .items
            .iter()
            .filter_map(|target| match target {
                noflex_core::PlayTarget::Edition(id) => Some(*id),
                noflex_core::PlayTarget::Work(_) => None,
            })
            .collect::<Vec<_>>();
        let works = storage
            .get_works(&work_ids)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let editions = storage
            .get_editions(&edition_ids)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let works = works
            .into_iter()
            .map(|work| (work.id, work))
            .collect::<std::collections::HashMap<_, _>>();
        let editions = editions
            .into_iter()
            .map(|edition| (edition.id, edition))
            .collect::<std::collections::HashMap<_, _>>();
        let items = page
            .items
            .into_iter()
            .map(|target| match target {
                noflex_core::PlayTarget::Work(id) => works
                    .get(&id)
                    .cloned()
                    .map(|work| PlaylistItem {
                        target_id: id.into_uuid(),
                        kind: PlayTargetKind::Work,
                        work: Some(work),
                        edition: None,
                    })
                    .ok_or_else(|| async_graphql::Error::new("playlist item disappeared")),
                noflex_core::PlayTarget::Edition(id) => editions
                    .get(&id)
                    .cloned()
                    .map(|edition| PlaylistItem {
                        target_id: id.into_uuid(),
                        kind: PlayTargetKind::Edition,
                        work: None,
                        edition: Some(edition),
                    })
                    .ok_or_else(|| async_graphql::Error::new("playlist item disappeared")),
            })
            .collect::<async_graphql::Result<Vec<_>>>()?;
        Ok(crate::connection::from_edges(
            page.item_cursors
                .into_iter()
                .map(|cursor| cursor.0)
                .zip(items),
            resumed,
            page.has_next,
        ))
    }
}

/// An administrator-configured M3U playlist and optional XMLTV guide.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct LiveTvSource {
    pub id: LiveTvSourceId,
    pub name: String,
    pub enabled: bool,
    pub last_refresh_at: Option<OffsetDateTime>,
    pub last_error: Option<String>,
    #[graphql(skip)]
    pub playlist_url: String,
    #[graphql(skip)]
    pub epg_url: Option<String>,
}

/// One admin-set dispatch priority, persisted so it survives restarts.
/// `hook`/`work_kind` are the manifest's kebab-case spellings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginOverrideRow {
    pub plugin_id: String,
    pub hook: String,
    pub work_kind: String,
    pub priority: i32,
}

/// A channel discovered from a Live TV source.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct LiveTvChannel {
    pub id: ChannelId,
    pub source_id: LiveTvSourceId,
    pub external_id: String,
    pub name: String,
    pub number: Option<String>,
    /// The playlist's `tvg-logo`, fetched server-side; clients get the
    /// same-origin proxy URL from `logoUrl` instead.
    #[graphql(skip)]
    pub logo_url: Option<String>,
    pub group: Option<String>,
    pub enabled: bool,
    #[graphql(skip)]
    pub stream_url: String,
}

#[async_graphql::ComplexObject]
impl LiveTvChannel {
    /// Same-origin URL of the channel's logo (proxied and cached by the
    /// server, so the strict image CSP allows it); null without a logo.
    #[graphql(name = "logoUrl")]
    async fn logo_proxy_url(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Option<String>> {
        if self.logo_url.is_none() {
            return Ok(None);
        }
        let path = format!("/live-tv/channels/{}/logo", self.id);
        let Some(signer) = ctx.data_opt::<std::sync::Arc<dyn MediaUrlSigner>>() else {
            return Ok(Some(path));
        };
        let user = crate::access::request_user(ctx)?;
        let token = signer
            .sign_channel_logo_token(user.user_id, crate::access::caller_session(ctx), self.id)
            .map_err(async_graphql::Error::new)?;
        Ok(Some(format!("{path}?token={}", percent_encode(&token))))
    }
}

/// One bounded electronic programme guide entry.
#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct EpgProgram {
    pub id: EpgProgramId,
    pub channel_id: ChannelId,
    pub external_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub start_at: OffsetDateTime,
    pub end_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Enum)]
pub enum RecordingScheduleStatus {
    Scheduled,
    Cancelled,
    Triggered,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct RecordingSchedule {
    pub id: RecordingScheduleId,
    pub owner_id: UserId,
    pub channel_id: ChannelId,
    pub program_id: Option<EpgProgramId>,
    pub start_at: OffsetDateTime,
    pub end_at: OffsetDateTime,
    pub padding_before_secs: u32,
    pub padding_after_secs: u32,
    pub status: RecordingScheduleStatus,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Enum)]
pub enum RecordingStatus {
    Scheduled,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct Recording {
    pub id: RecordingId,
    pub schedule_id: Option<RecordingScheduleId>,
    pub owner_id: UserId,
    pub channel_id: Option<ChannelId>,
    pub status: RecordingStatus,
    #[graphql(skip)]
    pub path: Option<String>,
    pub error: Option<String>,
    pub size_bytes: Option<u64>,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[cfg(test)]
mod variant_tests {
    use super::describe_variant;

    #[test]
    fn variant_keys_read_as_what_was_done() {
        for (key, words) in [
            (
                "copy-h264/aac2",
                "Video kept, audio converted to AAC stereo",
            ),
            ("copy-h265/copy+copy", "Repackaged, nothing re-encoded"),
            (
                "av1-720p-1556k-av1main/aac1",
                "Converted to AV1 720p with AAC mono",
            ),
            (
                "h264-1080p-8000k-high-tonemap/copy",
                "Converted to H.264 1080p SDR, audio kept",
            ),
            (
                "av1-2160p-9000k-av1main/copy+opus6",
                "Converted to AV1 2160p with OPUS 5.1",
            ),
            ("none/aac2", "Video kept, audio converted to AAC stereo"),
        ] {
            assert_eq!(describe_variant(key), words);
        }
    }
}
