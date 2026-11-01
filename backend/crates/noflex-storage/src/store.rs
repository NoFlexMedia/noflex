// SPDX-License-Identifier: AGPL-3.0-only

use std::future::Future;

use async_trait::async_trait;
use noflex_core::{
    ApiKeyId, ChannelId, CollectionId, DevicePinId, EditionId, EmbeddingTarget, EpgProgramId,
    ExportJobId, ExternalIdKind, ImageKind, LibraryId, LiveTvSourceId, MediaFileId, PlaySessionId,
    PlayTarget, PlaylistId, RecordingId, RecordingScheduleId, SessionId, TagId, TagTarget, UserId,
    WatchRoomId, WorkId, WorkKind,
};
use time::OffsetDateTime;
use tokio::io::AsyncWrite;

use crate::{
    domain::{
        ActivityEntry, ActivityKind, ApiKey, Collection, DevicePin, Edition, EpgProgram, ExportJob,
        ExportJobStatus, ExternalIdentity, HistoryPage, Image, ImageSource, ImageTarget,
        IntroMarkers, Library, LiveTvChannel, LiveTvSource, MediaFile, PlaySession, Playlist,
        PluginOverrideRow, PluginTaskPayload, PluginTaskState, Recording, RecordingSchedule,
        RecordingStatus, RefreshTokenLookup, ScanEvent, Session, Tag, TagAssignment, TagSummary,
        TrickplayAsset, User, UserData, UserPluginSecret, UserPreference, UserRestrictions,
        WatchRoom, WatchRoomMember, WatchRoomState, Work,
    },
    error::StorageResult,
    query::{
        HybridQuery, ItemPage, ListTagsQuery, ListWorksQuery, Page, PageCursor, PlaySessionQuery,
        TagQuery,
    },
    search::{HealthReport, SearchHit, VectorHit},
};

#[derive(Debug, Clone)]
pub struct ExportJobLease {
    pub job: ExportJob,
    pub lease_token: String,
    pub attempt_count: u32,
}

#[derive(Debug, Clone)]
pub struct ExportJobCancellation {
    pub job: ExportJob,
    pub changed: bool,
}

/// The single trait every backend implements. Method shapes are operation-
/// level, not query-level: no SQL strings, no `Row`s, no dialect leaks.
///
/// `dyn Storage` is the canonical way the rest of the server consumes this.
#[async_trait]
pub trait Storage: Send + Sync + 'static {
    // ---- Lifecycle ------------------------------------------------------
    async fn migrate(&self) -> StorageResult<()>;
    async fn health(&self) -> StorageResult<HealthReport>;
    async fn backup_to(&self, writer: &mut (dyn AsyncWrite + Send + Unpin)) -> StorageResult<()>;

    // ---- Transactions ---------------------------------------------------
    /// Begin a transaction. Returned object owns its own connection.
    async fn begin(&self) -> StorageResult<Box<dyn Transaction>>;

    // ---- Works ----------------------------------------------------------
    async fn get_work(&self, id: WorkId) -> StorageResult<Option<Work>>;
    /// Fetches works in the same order as `ids`, omitting missing rows.
    async fn get_works(&self, ids: &[WorkId]) -> StorageResult<Vec<Work>> {
        let mut works = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(work) = self.get_work(*id).await? {
                works.push(work);
            }
        }
        Ok(works)
    }
    /// Keyset page of works matching `q`. A `Random` sort ignores
    /// `q.cursor` and never reports `has_next`.
    async fn list_works(&self, q: &ListWorksQuery) -> StorageResult<ItemPage<Work>>;
    async fn upsert_work(&self, work: &Work) -> StorageResult<()>;
    async fn delete_work(&self, id: WorkId) -> StorageResult<()>;
    async fn works_by_external_id(
        &self,
        kind: ExternalIdKind,
        value: &str,
    ) -> StorageResult<Vec<Work>>;

    // ---- Editions / Media files ----------------------------------------
    async fn editions_for_work(&self, work: WorkId) -> StorageResult<Vec<Edition>>;
    async fn get_edition(&self, id: EditionId) -> StorageResult<Option<Edition>>;
    /// Fetches editions in the same order as `ids`, omitting missing rows.
    async fn get_editions(&self, ids: &[EditionId]) -> StorageResult<Vec<Edition>> {
        let mut editions = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(edition) = self.get_edition(*id).await? {
                editions.push(edition);
            }
        }
        Ok(editions)
    }
    /// Writes `edition`, returning the id of the stored row. When a row with
    /// `edition.id` exists it is updated in place — renames and re-parents
    /// keep the id — and `edition.id` is returned. Otherwise the edition is
    /// inserted, or merged into the row already holding its
    /// `(work_id, name)`, whose id is returned. Renaming onto a
    /// `(work_id, name)` owned by a different edition is a
    /// [`StorageError::Constraint`].
    async fn upsert_edition(&self, edition: &Edition) -> StorageResult<EditionId>;
    async fn delete_edition(&self, id: EditionId) -> StorageResult<()>;

    /// Overwrites the intro/credits marker columns of an existing
    /// edition. Only those three columns are touched — chapters,
    /// runtime, and probe metadata are left alone. Returns
    /// [`StorageError::NotFound`] if no edition with `edition` exists.
    async fn set_intro_markers(
        &self,
        edition: EditionId,
        markers: IntroMarkers,
    ) -> StorageResult<()>;

    /// Records that detector `plugin_id` has scanned `edition`,
    /// stamping the row with the current time and whether the plugin
    /// returned markers. Upserts on (edition, plugin): re-recording
    /// refreshes the timestamp and `found` instead of erroring.
    async fn record_intro_scan(
        &self,
        edition: EditionId,
        plugin_id: &str,
        found: bool,
    ) -> StorageResult<()>;

    /// Editions with no scan row for `plugin_id`, keyset-paginated by
    /// edition id. The intro-detection sweep pages through this to
    /// find work for a detector plugin; an edition drops out of the
    /// results after its first [`record_intro_scan`][Storage::record_intro_scan]
    /// for that plugin. Scan rows are per-plugin, so other detectors
    /// still see the edition. Malformed cursors are a
    /// [`StorageError::InvalidInput`].
    async fn editions_missing_intro_scan(
        &self,
        plugin_id: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Edition>>;

    async fn media_files_for_edition(&self, edition: EditionId) -> StorageResult<Vec<MediaFile>>;
    async fn upsert_media_file(&self, media: &MediaFile) -> StorageResult<()>;
    async fn delete_media_file(&self, id: MediaFileId) -> StorageResult<()>;
    /// Removes a media file, then prunes its empty edition and any now-empty
    /// work ancestry in one transaction. Polymorphic dependent rows for each
    /// removed target are deleted with it.
    async fn remove_media_file_and_prune(
        &self,
        id: MediaFileId,
    ) -> StorageResult<crate::domain::MediaRemoval>;
    async fn media_file_by_path(&self, path: &str) -> StorageResult<Option<MediaFile>>;
    async fn media_file_by_id(&self, id: MediaFileId) -> StorageResult<Option<MediaFile>>;

    // ---- Trickplay ------------------------------------------------------
    async fn get_trickplay(&self, edition: EditionId) -> StorageResult<Option<TrickplayAsset>>;
    async fn upsert_trickplay(&self, asset: &TrickplayAsset) -> StorageResult<()>;
    async fn delete_trickplay(&self, edition: EditionId) -> StorageResult<()>;
    /// Editions whose preferred media file has no current trickplay
    /// manifest, keyset-paginated by edition id. Source mtime and size
    /// changes make an existing manifest current only after regeneration.
    async fn editions_missing_trickplay(
        &self,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Edition>>;

    // ---- Tags -----------------------------------------------------------
    async fn upsert_tag(&self, tag: &Tag) -> StorageResult<TagId>;
    async fn get_tag(&self, id: TagId) -> StorageResult<Option<Tag>>;
    /// Tags ordered by case-folded display name, each with its work
    /// count. Keyset-paginated; a malformed cursor is an
    /// [`StorageError::InvalidInput`], never silently ignored.
    async fn list_tags(&self, q: &ListTagsQuery) -> StorageResult<ItemPage<TagSummary>>;
    async fn tag_by_slug(&self, namespace: &str, slug: &str) -> StorageResult<Option<Tag>>;
    async fn assign_tag(
        &self,
        tag: TagId,
        target: TagTarget,
        weight: f32,
        source: &str,
    ) -> StorageResult<()>;
    async fn unassign_tag(&self, tag: TagId, target: TagTarget) -> StorageResult<()>;
    async fn tags_for_target(&self, target: TagTarget) -> StorageResult<Vec<Tag>>;
    /// Assignments for a target with their weight and provenance source.
    async fn tag_assignments_for_target(
        &self,
        target: TagTarget,
    ) -> StorageResult<Vec<TagAssignment>>;
    async fn search_works_by_tags(&self, q: &TagQuery) -> StorageResult<Page<WorkId>>;

    // ---- Users / sessions ----------------------------------------------
    async fn get_user(&self, id: UserId) -> StorageResult<Option<User>>;
    async fn get_user_by_name(&self, name: &str) -> StorageResult<Option<User>>;
    async fn upsert_user(&self, user: &User) -> StorageResult<()>;
    /// Inserts `user` only if no user row exists, atomically with that
    /// check, so concurrent first-run setups admit exactly one
    /// administrator. Returns `false`, writing nothing, when any user
    /// already exists. `user` must have `is_admin` set and the `Admin`
    /// role, else [`StorageError::InvalidInput`]; a duplicate name or id
    /// cannot occur on an empty table.
    async fn create_initial_admin(&self, user: &User) -> StorageResult<bool>;
    async fn delete_user(&self, id: UserId) -> StorageResult<()>;
    async fn list_users(&self) -> StorageResult<Vec<User>>;

    // ---- External identities -------------------------------------------
    async fn external_identity(
        &self,
        plugin_id: &str,
        issuer: &str,
        subject: &str,
    ) -> StorageResult<Option<ExternalIdentity>>;
    async fn list_external_identities(&self, user: UserId) -> StorageResult<Vec<ExternalIdentity>>;
    /// Links a verified provider identity to `identity.user_id`, or
    /// refreshes the profile claims of an existing link. A user holds at
    /// most one identity per plugin, and an `(plugin_id, issuer, subject)`
    /// belongs to at most one user: violating either is a
    /// [`StorageError::Conflict`]. An unknown user is a
    /// [`StorageError::Constraint`].
    async fn link_external_identity(&self, identity: &ExternalIdentity) -> StorageResult<()>;
    async fn unlink_external_identity(&self, user: UserId, plugin_id: &str) -> StorageResult<bool>;

    async fn list_user_preferences(&self, user: UserId) -> StorageResult<Vec<UserPreference>>;
    async fn set_user_preference(&self, preference: &UserPreference) -> StorageResult<()>;
    async fn delete_user_preference(&self, user: UserId, key: &str) -> StorageResult<bool>;

    // ---- Sessions -------------------------------------------------------
    async fn create_session(&self, session: &Session) -> StorageResult<()>;
    async fn get_session(&self, id: SessionId) -> StorageResult<Option<Session>>;
    async fn list_sessions_for_user(&self, user: UserId) -> StorageResult<Vec<Session>>;
    async fn touch_session(&self, id: SessionId, at: OffsetDateTime) -> StorageResult<()>;
    async fn revoke_session(&self, id: SessionId) -> StorageResult<()>;
    /// Classify the SHA-256 hex of a presented refresh token against
    /// current and retired hashes in one consistent read. Expiry is not
    /// checked: the caller inspects `expires_at` on a hit. A session with
    /// a `None` hash matches nothing. Deleting a session (revocation,
    /// pruning, user deletion) deletes its retired hashes, which then
    /// resolve `Unknown`.
    async fn session_by_refresh_token(&self, hash: &str) -> StorageResult<RefreshTokenLookup>;
    /// Compare-and-swap rotation: when `session_id`'s current hash is
    /// `expected_hash`, atomically retire it (stamped `now`, kept until
    /// the session is deleted), install `new_hash`, and set `expires_at`
    /// and `last_seen_at`. Returns `false`, writing nothing, when another
    /// rotation or a revocation won first. A `new_hash` that is already
    /// some session's current hash is a
    /// [`StorageError::Constraint`](crate::StorageError::Constraint).
    async fn rotate_refresh_token(
        &self,
        session_id: SessionId,
        expected_hash: &str,
        new_hash: &str,
        new_expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> StorageResult<bool>;

    async fn grant_library_access(&self, user: UserId, library: LibraryId) -> StorageResult<()>;
    async fn revoke_library_access(&self, user: UserId, library: LibraryId) -> StorageResult<()>;
    async fn list_user_libraries(&self, user: UserId) -> StorageResult<Vec<LibraryId>>;

    // ---- Restrictions / target access ----------------------------------
    async fn get_user_restrictions(&self, user: UserId) -> StorageResult<Option<UserRestrictions>>;
    async fn set_user_restrictions(&self, restrictions: &UserRestrictions) -> StorageResult<()>;
    async fn delete_user_restrictions(&self, user: UserId) -> StorageResult<bool>;
    async fn work_for_edition(&self, edition: EditionId) -> StorageResult<Option<Work>>;
    async fn work_for_media_file(&self, media: MediaFileId) -> StorageResult<Option<Work>>;
    async fn user_can_access_work(&self, user: UserId, work: WorkId) -> StorageResult<bool>;
    /// Returns the accessible, existing ids in input order.
    async fn accessible_work_ids(
        &self,
        user: UserId,
        works: &[WorkId],
    ) -> StorageResult<Vec<WorkId>> {
        let mut accessible = Vec::with_capacity(works.len());
        for work in works {
            if self.user_can_access_work(user, *work).await? {
                accessible.push(*work);
            }
        }
        Ok(accessible)
    }

    // ---- Watch rooms ----------------------------------------------------
    async fn create_watch_room(&self, room: &WatchRoom) -> StorageResult<()>;
    async fn get_watch_room(&self, id: WatchRoomId) -> StorageResult<Option<WatchRoom>>;
    async fn list_watch_rooms(&self, user: UserId) -> StorageResult<Vec<WatchRoom>>;
    async fn list_watch_room_members(
        &self,
        room: WatchRoomId,
    ) -> StorageResult<Vec<WatchRoomMember>>;
    /// Adds `member` to its room (idempotent for an existing member) and
    /// returns the room. `None`, writing nothing, when the room is missing
    /// or ended, its media file no longer resolves to a work, or the user
    /// cannot access that work.
    async fn join_watch_room(&self, member: &WatchRoomMember) -> StorageResult<Option<WatchRoom>>;
    /// Removes `user` from `room`. When the host leaves, the room is ended
    /// and emptied, and the ended room is returned. `None` for every other
    /// outcome: missing room, non-member, a non-host leaving (who is still
    /// removed), or a room that had already ended.
    async fn leave_watch_room(
        &self,
        room: WatchRoomId,
        user: UserId,
    ) -> StorageResult<Option<WatchRoom>>;
    async fn control_watch_room(
        &self,
        room: WatchRoomId,
        host: UserId,
        state: Option<WatchRoomState>,
        position_ms: Option<u64>,
        updated_at: OffsetDateTime,
    ) -> StorageResult<Option<WatchRoom>>;

    async fn list_all_sessions(&self) -> StorageResult<Vec<Session>>;
    async fn count_user_sessions(&self, user: UserId) -> StorageResult<u32>;
    async fn revoke_user_sessions(&self, user: UserId) -> StorageResult<u32>;
    async fn prune_sessions_before(&self, before: time::OffsetDateTime) -> StorageResult<u32>;

    // ---- API keys -------------------------------------------------------
    /// Mint and persist an API key for `user_id`: the implementation
    /// assigns the id and `created_at`. `key_hash` is the SHA-256 hex of
    /// the presented key — the plaintext must never reach this layer.
    /// A duplicate `key_hash` is a `Constraint` error. Returns the
    /// stored row (active, `revoked_at == None`).
    async fn create_api_key(
        &self,
        user_id: UserId,
        name: &str,
        key_hash: &str,
    ) -> StorageResult<ApiKey>;

    /// Resolve a key by its SHA-256 hex hash. Revoked keys ARE returned
    /// — keeping the revocation check in exactly one place (the caller
    /// inspecting `revoked_at`) beats every lookup path reimplementing
    /// it. `None` only for an unknown hash.
    async fn api_key_by_hash(&self, key_hash: &str) -> StorageResult<Option<ApiKey>>;

    /// Every key the user owns, active and revoked alike, newest
    /// `created_at` first. Callers filter on `revoked_at`.
    async fn list_api_keys(&self, user_id: UserId) -> StorageResult<Vec<ApiKey>>;

    /// Stamp `revoked_at` on the key. `true` if this call did the
    /// revoking; `false` when the id is unknown or the key was already
    /// revoked, so callers can tell a replayed revoke from a real one.
    async fn revoke_api_key(&self, id: ApiKeyId) -> StorageResult<bool>;

    // ---- Device PINs ----------------------------------------------------
    /// Persist a freshly-minted PIN request: the caller assigns id, PIN
    /// code, and expiry, and passes `user_id: None`. A duplicate `pin`
    /// or `id` is a `Constraint` error.
    async fn create_device_pin(&self, pin: &DevicePin) -> StorageResult<()>;

    /// Look a request up by its PIN code. Expired rows count as absent
    /// (`None`); the implementation may delete them lazily on the way
    /// out. Authorized-but-unexpired rows are returned with `user_id`
    /// set — that's how the device learns it got linked.
    async fn device_pin_by_pin(&self, pin: &str) -> StorageResult<Option<DevicePin>>;

    /// Attach `user_id` to a pending PIN. `true` if this call did the
    /// authorizing; `false` when the PIN is unknown, expired, or
    /// already authorized — clients can't tell those apart, which is
    /// deliberate.
    async fn authorize_device_pin(&self, pin: &str, user_id: UserId) -> StorageResult<bool>;

    /// Consume a PIN request. Called by the device once it has traded
    /// the authorization for a token, and by cleanup for stale rows.
    /// `false` when the id was already gone.
    async fn delete_device_pin(&self, id: DevicePinId) -> StorageResult<bool>;

    async fn user_data(&self, user: UserId, target: PlayTarget) -> StorageResult<UserData>;
    async fn record_playback_progress(
        &self,
        user: UserId,
        target: PlayTarget,
        position_ms: i64,
    ) -> StorageResult<()>;
    async fn mark_played(
        &self,
        user: UserId,
        target: PlayTarget,
        played: bool,
    ) -> StorageResult<()>;
    async fn set_favorite(
        &self,
        user: UserId,
        target: PlayTarget,
        favorite: bool,
    ) -> StorageResult<()>;
    async fn resume_items(&self, user: UserId, limit: u32) -> StorageResult<Vec<Work>>;
    async fn next_up(&self, user: UserId, limit: u32) -> StorageResult<Vec<Work>>;

    // ---- Libraries / scans ---------------------------------------------
    async fn list_libraries(&self) -> StorageResult<Vec<Library>>;
    async fn upsert_library(&self, library: &Library) -> StorageResult<()>;
    async fn delete_library(&self, id: LibraryId) -> StorageResult<()>;
    async fn get_library(&self, id: LibraryId) -> StorageResult<Option<Library>> {
        Ok(self
            .list_libraries()
            .await?
            .into_iter()
            .find(|lib| lib.id == id))
    }
    async fn record_scan_event(&self, event: &ScanEvent) -> StorageResult<()>;
    /// The scanner layout version last recorded for `library` with
    /// [`set_library_scan_version`][Storage::set_library_scan_version], or
    /// 0 if none was. Opaque to storage. An unknown library is
    /// [`StorageError::NotFound`].
    async fn library_scan_version(&self, library: LibraryId) -> StorageResult<u32>;
    /// [`upsert_library`][Storage::upsert_library] never changes the
    /// recorded version. An unknown library is [`StorageError::NotFound`].
    async fn set_library_scan_version(&self, library: LibraryId, version: u32)
    -> StorageResult<()>;

    // ---- Plugin overrides -----------------------------------------------
    /// Persist a per-(plugin, hook, work_kind) priority override,
    /// replacing any existing priority for that triple.
    async fn upsert_plugin_override(
        &self,
        plugin_id: &str,
        hook: &str,
        work_kind: &str,
        priority: i32,
    ) -> StorageResult<()>;
    /// Every persisted override, ordered by (plugin, hook, work_kind).
    /// The server applies these over manifest and config priorities at
    /// startup.
    async fn list_plugin_overrides(&self) -> StorageResult<Vec<PluginOverrideRow>>;
    /// Remove one override; `false` when there was none.
    async fn delete_plugin_override(
        &self,
        plugin_id: &str,
        hook: &str,
        work_kind: &str,
    ) -> StorageResult<bool>;

    // ---- Plugin background tasks ---------------------------------------
    /// Current cursor + health row for one plugin background task.
    /// `None` when the (plugin, task) pair has never checkpointed.
    /// Host-internal: the plugin-host scheduler is the only caller.
    async fn plugin_task_state(
        &self,
        plugin_id: &str,
        task: &str,
    ) -> StorageResult<Option<PluginTaskState>>;

    /// Upsert the cursor + health row keyed by (plugin, task): an
    /// existing row is overwritten wholesale, including
    /// `consecutive_failures`.
    async fn set_plugin_task_state(&self, state: &PluginTaskState) -> StorageResult<()>;

    /// Delete every state row belonging to `plugin_id`; returns the
    /// number of rows removed. Called when a plugin is uninstalled.
    /// Does NOT touch the plugin's task queue — call
    /// [`Storage::clear_plugin_task_queue`] for that.
    async fn delete_plugin_task_states(&self, plugin_id: &str) -> StorageResult<u64>;

    /// Append `payload` to the (plugin, task) FIFO, stamping
    /// `enqueued_at` host-side. Payloads for different (plugin, task)
    /// pairs never interleave at dequeue time.
    async fn enqueue_plugin_task(
        &self,
        plugin_id: &str,
        task: &str,
        payload: Option<&str>,
    ) -> StorageResult<()>;

    /// Take the oldest pending payload for (plugin, task). Destructive:
    /// the returned row is deleted by the call, so a crash after
    /// dequeue loses the payload. `None` when the queue is empty.
    async fn dequeue_plugin_task(
        &self,
        plugin_id: &str,
        task: &str,
    ) -> StorageResult<Option<PluginTaskPayload>>;

    /// Every state row for `plugin_id`, ordered by task name.
    async fn list_plugin_task_states(&self, plugin_id: &str)
    -> StorageResult<Vec<PluginTaskState>>;

    /// Delete every queued payload belonging to `plugin_id`, across all
    /// tasks; returns the number of rows removed. State rows are
    /// untouched — pair with [`Storage::delete_plugin_task_states`] on
    /// uninstall.
    async fn clear_plugin_task_queue(&self, plugin_id: &str) -> StorageResult<u64>;

    // ---- Per-user plugin secrets ----------------------------------------
    /// Store one credential for a (user, plugin, key) triple,
    /// overwriting any existing value. Timestamps are stamped
    /// host-side: `created_at` survives the overwrite, `updated_at`
    /// moves to the write time. Host-internal — plugins never see
    /// `Storage`; the host reads these on a plugin's behalf after a
    /// link flow. `value` is a secret and must never reach the
    /// GraphQL surface.
    async fn set_user_plugin_secret(
        &self,
        user: UserId,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> StorageResult<()>;

    /// Single-secret lookup by exact (user, plugin, key). `None` when
    /// the user has never linked that plugin key.
    async fn user_plugin_secret(
        &self,
        user: UserId,
        plugin_id: &str,
        key: &str,
    ) -> StorageResult<Option<UserPluginSecret>>;

    /// Every secret the user holds for `plugin_id`, ordered by key.
    async fn list_user_plugin_secrets(
        &self,
        user: UserId,
        plugin_id: &str,
    ) -> StorageResult<Vec<UserPluginSecret>>;

    /// Delete every secret the user holds for `plugin_id`; returns
    /// the number of rows removed. Scoped to exactly one (user,
    /// plugin) pair — other users and other plugins are untouched.
    /// Called when a user unlinks a plugin; rows also cascade away
    /// with the user row itself.
    async fn delete_user_plugin_secrets(&self, user: UserId, plugin_id: &str)
    -> StorageResult<u64>;

    // ---- Activity log --------------------------------------------------
    /// Append a row to the activity log. Returns the assigned id, which
    /// increases monotonically.
    async fn append_activity(
        &self,
        kind: ActivityKind,
        actor: Option<&str>,
        subject: Option<&str>,
        detail: Option<&str>,
        occurred_at: time::OffsetDateTime,
    ) -> StorageResult<i64>;

    /// Keyset page of the activity log newest-first by
    /// `(occurred_at, id)`. `kinds` filters by kind (empty = all);
    /// `since` is an inclusive floor on `occurred_at`; `limit` is raised
    /// to at least one. A malformed cursor is
    /// [`StorageError::InvalidInput`](crate::StorageError::InvalidInput).
    async fn list_activity(
        &self,
        kinds: &[ActivityKind],
        since: Option<time::OffsetDateTime>,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<ActivityEntry>>;

    // ---- Play history ---------------------------------------------------
    /// Inserts the viewing, or replaces every field of the one with the
    /// same id. The user, both works and the library must exist; the row
    /// is deleted with any of them.
    async fn upsert_play_session(&self, session: &PlaySession) -> StorageResult<()>;
    /// `user`'s most recently seen viewing of `work` that hasn't ended
    /// and was last seen at or after `seen_since`.
    async fn open_play_session(
        &self,
        user: UserId,
        work: WorkId,
        seen_since: OffsetDateTime,
    ) -> StorageResult<Option<PlaySession>>;
    async fn get_play_session(&self, id: PlaySessionId) -> StorageResult<Option<PlaySession>>;
    /// Keyset page of viewings matching `query`, newest `started_at`
    /// first, ties by id descending; `limit` is raised to at least one.
    /// A malformed cursor is
    /// [`StorageError::InvalidInput`](crate::StorageError::InvalidInput).
    async fn list_play_sessions(
        &self,
        query: &PlaySessionQuery,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<PlaySession>>;
    /// Every viewing matching `query`, in `list_play_sessions` order.
    /// Unbounded: the input to whole-range rollups.
    async fn play_sessions_matching(
        &self,
        query: &PlaySessionQuery,
    ) -> StorageResult<Vec<PlaySession>>;

    // ---- Aggregations ---------------------------------------------------
    /// Up to `limit` (at least one) top-level works across every library,
    /// newest `added_at` first.
    async fn recently_added_works(&self, limit: u32) -> StorageResult<Vec<Work>>;

    /// Top-N most-recently-played works for the given user, ordered
    /// by `last_played` desc. Empty list for users with no plays.
    async fn recently_played_for_user(&self, user: UserId, limit: u32) -> StorageResult<Vec<Work>>;

    /// Page the user's played targets newest `last_played` first, in a
    /// total order (ties broken by target), so consecutive pages never
    /// skip or repeat a row. `cursor` is a `HistoryPage::next_cursor` or
    /// `HistoryRow::cursor` from an earlier call; paging resumes just
    /// after that row even if it has since changed. `limit` is raised to
    /// at least one. A malformed cursor is [`StorageError::InvalidInput`].
    async fn list_play_history(
        &self,
        user: UserId,
        cursor: Option<PageCursor>,
        limit: u32,
    ) -> StorageResult<HistoryPage>;

    /// All media files belonging to a library (joined via edition → work).
    /// Scanners use this to detect removals on rescan.
    async fn list_media_files_for_library(
        &self,
        library: LibraryId,
    ) -> StorageResult<Vec<MediaFile>>;

    // ---- Images --------------------------------------------------------
    /// Inserts or replaces the row keyed by (target, kind, source); rows
    /// of the same kind from other sources are untouched.
    async fn upsert_image(&self, image: &Image) -> StorageResult<()>;
    /// The `kind` image of `target` from the first source in `order`
    /// holding one. Sources absent from `order` are never returned, so an
    /// empty `order` always yields `None`.
    async fn image(
        &self,
        target: ImageTarget,
        kind: ImageKind,
        order: &[ImageSource],
    ) -> StorageResult<Option<Image>>;
    /// The effective image of every kind attached to `target`, each
    /// resolved as [`Storage::image`] does, ordered by kind.
    async fn list_images(
        &self,
        target: ImageTarget,
        order: &[ImageSource],
    ) -> StorageResult<Vec<Image>>;
    /// Every image row of every source whose target is a work in
    /// `library`, ordered by work id, kind, then source. Edition and tag
    /// images are never included.
    async fn list_work_images_for_library(&self, library: LibraryId) -> StorageResult<Vec<Image>>;
    /// Removes the (`kind`, `source`) image of `target`. Returns `false`
    /// when there was no such row.
    async fn delete_image(
        &self,
        target: ImageTarget,
        kind: ImageKind,
        source: ImageSource,
    ) -> StorageResult<bool>;
    /// Works of `kind` with no `image`-kind row from any of `sources`,
    /// keyset-paginated by work id; a work drops out once such a row is
    /// upserted. Malformed cursors are a [`StorageError::InvalidInput`].
    async fn works_missing_image(
        &self,
        kind: WorkKind,
        image: ImageKind,
        sources: &[ImageSource],
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Work>>;
    /// Works carrying at least one non-[`ExternalIdKind::Custom`]
    /// external id and no image row of any kind from `source`,
    /// keyset-paginated by work id. A page may hold fewer than `limit`
    /// items while `next_cursor` is still set. Malformed cursors are a
    /// [`StorageError::InvalidInput`].
    async fn identified_works_missing_image_source(
        &self,
        source: ImageSource,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Work>>;

    // ---- Search: lexical + vector + hybrid -----------------------------
    async fn search_lexical(
        &self,
        query: &str,
        kinds: &[WorkKind],
        access_user_id: Option<UserId>,
        limit: u32,
    ) -> StorageResult<Vec<SearchHit>>;

    async fn upsert_embedding(
        &self,
        target: EmbeddingTarget,
        namespace: &str,
        model_id: &str,
        vector: &[f32],
    ) -> StorageResult<()>;

    async fn search_vector(
        &self,
        namespace: &str,
        query_vector: &[f32],
        kinds: &[WorkKind],
        access_user_id: Option<UserId>,
        limit: u32,
    ) -> StorageResult<Vec<VectorHit>>;

    /// Works that have no embedding row for `namespace`, keyset-paginated
    /// by work id. Backfill sweeps page through this to find vectors to
    /// compute; a work drops out of the results after its first
    /// `upsert_embedding` for the namespace. Malformed cursors are a
    /// [`StorageError::InvalidInput`].
    async fn works_missing_embedding(
        &self,
        namespace: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Work>>;

    /// Default-impl fuses lexical + vector results with RRF. Backends MAY
    /// override with an engine-native hybrid query (e.g. Turso's planned
    /// native hybrid).
    async fn search_hybrid(&self, query: &HybridQuery) -> StorageResult<Vec<SearchHit>> {
        crate::search::default_hybrid(self, query).await
    }

    // ---- Export jobs ---------------------------------------------------
    async fn create_export_job(&self, job: &ExportJob) -> StorageResult<()>;

    async fn get_export_job(&self, id: ExportJobId) -> StorageResult<Option<ExportJob>>;

    /// Keyset page of a user's export jobs, newest `(created_at, id)`
    /// first. `limit` is clamped to `1..=ITEM_PAGE_MAX`; a malformed
    /// cursor is [`StorageError::InvalidInput`](crate::StorageError::InvalidInput).
    async fn list_export_jobs(
        &self,
        user: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<ExportJob>>;

    /// Atomically claims the oldest queued job or an expired running job.
    /// Expired jobs at `max_attempts` are failed before selecting a candidate.
    async fn claim_export_job(
        &self,
        worker_id: &str,
        lease_token: &str,
        now: OffsetDateTime,
        lease_expires_at: OffsetDateTime,
        max_attempts: u32,
    ) -> StorageResult<Option<ExportJobLease>>;

    /// Renews a lease and writes progress. `None` means the lease was lost;
    /// otherwise the returned state tells the worker whether to stop.
    async fn update_export_job_progress(
        &self,
        id: ExportJobId,
        lease_token: &str,
        progress: f32,
        lease_expires_at: OffsetDateTime,
    ) -> StorageResult<Option<ExportJobStatus>>;

    async fn complete_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
        output_path: &str,
    ) -> StorageResult<bool>;

    async fn fail_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
        error: &str,
    ) -> StorageResult<bool>;

    async fn cancel_claimed_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
    ) -> StorageResult<bool>;

    /// Cancels queued work immediately or requests cancellation from the
    /// worker holding a running lease. Ownership is checked in the same write.
    async fn request_export_job_cancellation(
        &self,
        id: ExportJobId,
        owner: UserId,
    ) -> StorageResult<Option<ExportJobCancellation>>;

    async fn delete_export_job(&self, id: ExportJobId) -> StorageResult<()>;

    // ---- Collections ---------------------------------------------------
    /// Insert a new collection row.
    async fn create_collection(&self, collection: &Collection) -> StorageResult<()>;

    /// Full-row write keyed by id. `NotFound` when the row is absent —
    /// the GQL update path read-modify-writes, so a missing row here
    /// means it was deleted between the read and the write.
    async fn update_collection(&self, collection: &Collection) -> StorageResult<()>;

    /// `NotFound` when the row was absent, so the GQL delete mutation
    /// can report whether anything was actually removed.
    async fn delete_collection(&self, id: CollectionId) -> StorageResult<()>;

    async fn get_collection(&self, id: CollectionId) -> StorageResult<Option<Collection>>;

    /// Collections ordered by `(sort_title, id)`.
    async fn list_collections(
        &self,
        library_id: LibraryId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<Collection>>;

    /// Full ordered replace of a collection's membership: duplicates
    /// are dropped keeping the first occurrence, positions rewrite to
    /// `0..n`. `NotFound` when the collection is absent.
    async fn set_collection_items(&self, id: CollectionId, items: &[WorkId]) -> StorageResult<()>;

    /// A keyset page of the collection's visible works in membership order.
    /// The principal must be authenticated; administrators bypass grants and
    /// restrictions.
    async fn collection_items(
        &self,
        id: CollectionId,
        principal: crate::RequestUser,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<WorkId>>;

    // ---- Playlists ------------------------------------------------------
    async fn create_playlist(&self, playlist: &Playlist) -> StorageResult<()>;

    /// Full-row write keyed by id. `NotFound` when the row is absent.
    async fn update_playlist(&self, playlist: &Playlist) -> StorageResult<()>;

    /// `NotFound` when the row was absent, so the GQL delete mutation
    /// can report whether anything was actually removed.
    async fn delete_playlist(&self, id: PlaylistId) -> StorageResult<()>;

    async fn get_playlist(&self, id: PlaylistId) -> StorageResult<Option<Playlist>>;

    /// The user's playlists ordered by `(title, id)`.
    async fn list_playlists(
        &self,
        owner: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<Playlist>>;

    /// Full ordered replace of a playlist's entries; same semantics as
    /// [`Storage::set_collection_items`]. A playlist item IS a play
    /// target — no separate item type.
    async fn set_playlist_items(&self, id: PlaylistId, items: &[PlayTarget]) -> StorageResult<()>;

    /// A keyset page of visible entries in playlist order. Non-administrators
    /// must own the playlist; anonymous callers are not supported.
    async fn playlist_items(
        &self,
        id: PlaylistId,
        principal: crate::RequestUser,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<PlayTarget>>;

    /// Appends new, distinct targets without reading the existing list.
    async fn append_playlist_items(
        &self,
        id: PlaylistId,
        items: &[PlayTarget],
    ) -> StorageResult<()>;

    // ---- Live TV / DVR --------------------------------------------------
    async fn upsert_live_tv_source(&self, source: &LiveTvSource) -> StorageResult<()>;
    async fn get_live_tv_source(&self, id: LiveTvSourceId) -> StorageResult<Option<LiveTvSource>>;
    async fn list_live_tv_sources(&self) -> StorageResult<Vec<LiveTvSource>>;
    async fn delete_live_tv_source(&self, id: LiveTvSourceId) -> StorageResult<()>;
    /// Atomically replaces `source`'s lineup and guide with one fetched
    /// snapshot. Channels are matched by `(source, external_id)` and keep
    /// their stored id; channels absent from `channels` are disabled, not
    /// deleted. Programs are matched by `(channel, external_id,
    /// start_at)`; every guide row of the source absent from `programs` is
    /// deleted. Each program's `channel_id` must be the id of an entry in
    /// `channels`, else [`StorageError::InvalidInput`]. Sets
    /// `last_refresh_at` and clears `last_error`; an unknown source is
    /// [`StorageError::NotFound`].
    async fn apply_live_tv_refresh(
        &self,
        source: LiveTvSourceId,
        channels: &[LiveTvChannel],
        programs: &[EpgProgram],
        refreshed_at: OffsetDateTime,
    ) -> StorageResult<()>;
    /// Records a failed refresh attempt on `source`. Ignored when the
    /// source is unknown or already records a later attempt.
    async fn record_live_tv_refresh_error(
        &self,
        source: LiveTvSourceId,
        error: &str,
        attempted_at: OffsetDateTime,
    ) -> StorageResult<()>;
    async fn get_live_tv_channel(&self, id: ChannelId) -> StorageResult<Option<LiveTvChannel>>;
    /// Enabled channels, optionally of one source, keyset-paginated by id.
    /// `limit` is clamped to `1..=ITEM_PAGE_MAX`; a malformed cursor is
    /// [`StorageError::InvalidInput`].
    async fn list_live_tv_channels(
        &self,
        source: Option<LiveTvSourceId>,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<LiveTvChannel>>;
    /// Keyset page of programs on any of `channels` overlapping
    /// `[start, end)`, ordered by `(start_at, id)` across all channels.
    /// Duplicate and unknown channel ids are ignored; `limit` is raised
    /// to at least one. A malformed cursor is
    /// [`StorageError::InvalidInput`](crate::StorageError::InvalidInput).
    async fn list_live_tv_guide(
        &self,
        channels: &[ChannelId],
        start: OffsetDateTime,
        end: OffsetDateTime,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<EpgProgram>>;
    async fn get_epg_program(&self, id: EpgProgramId) -> StorageResult<Option<EpgProgram>>;
    async fn create_recording_schedule(&self, schedule: &RecordingSchedule) -> StorageResult<()>;
    async fn get_recording_schedule(
        &self,
        id: RecordingScheduleId,
    ) -> StorageResult<Option<RecordingSchedule>>;
    /// An owner's schedules, newest id first. `limit` is clamped to
    /// `1..=ITEM_PAGE_MAX`; a malformed cursor is
    /// [`StorageError::InvalidInput`].
    async fn list_recording_schedules(
        &self,
        owner: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<RecordingSchedule>>;
    /// Cancels an owner's schedule: a `Scheduled` one directly, a
    /// `Triggered` one by cancelling its `Running` recording at `now`.
    /// Returns the updated schedule, or `None` when the schedule is
    /// missing, owned by someone else, or has nothing left to cancel.
    async fn cancel_recording_schedule(
        &self,
        id: RecordingScheduleId,
        owner: UserId,
        now: OffsetDateTime,
    ) -> StorageResult<Option<RecordingSchedule>>;
    /// Atomically moves the earliest-starting `Scheduled` schedule whose
    /// `start_at - padding_before_secs <= now` to `Triggered` and creates its
    /// `Running` recording with `started_at = now`. Each schedule is
    /// claimed at most once; `None` when nothing is due.
    async fn claim_due_recording(
        &self,
        now: OffsetDateTime,
    ) -> StorageResult<Option<(RecordingSchedule, Recording)>>;
    async fn get_recording(&self, id: RecordingId) -> StorageResult<Option<Recording>>;
    async fn recording_for_schedule(
        &self,
        schedule: RecordingScheduleId,
    ) -> StorageResult<Option<Recording>>;
    /// An owner's recordings, newest id first. `limit` is clamped to
    /// `1..=ITEM_PAGE_MAX`; a malformed cursor is
    /// [`StorageError::InvalidInput`].
    async fn list_recordings(
        &self,
        owner: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<Recording>>;
    /// Moves a `Running` recording to `status`, which must be `Completed`
    /// or `Failed` (else [`StorageError::InvalidInput`]), stamping
    /// `completed_at = at`. `true` iff this call made the transition;
    /// `false` when the recording is missing or already terminal (e.g. a
    /// concurrent cancellation won).
    async fn finish_recording(
        &self,
        id: RecordingId,
        status: RecordingStatus,
        path: Option<&str>,
        error: Option<&str>,
        size_bytes: Option<u64>,
        at: OffsetDateTime,
    ) -> StorageResult<bool>;
    /// Deletes an owner's finished recording row (not its file) and
    /// returns it. `None` when missing or owned by someone else; a
    /// `Scheduled` or `Running` recording is a [`StorageError::Conflict`].
    async fn delete_recording(
        &self,
        id: RecordingId,
        owner: UserId,
    ) -> StorageResult<Option<Recording>>;
    /// Marks every `Running` recording `Failed` with `error` at `at`;
    /// returns how many changed. For startup recovery after a crash.
    async fn fail_running_recordings(&self, at: OffsetDateTime, error: &str) -> StorageResult<u64>;
}

/// A transaction handle. Mirrors a subset of `Storage`'s mutation methods so
/// services that batch writes can do so atomically.
#[async_trait]
pub trait Transaction: Send {
    async fn commit(self: Box<Self>) -> StorageResult<()>;
    async fn rollback(self: Box<Self>) -> StorageResult<()>;

    async fn upsert_work(&mut self, work: &Work) -> StorageResult<()>;
    async fn upsert_edition(&mut self, edition: &Edition) -> StorageResult<EditionId>;
    async fn upsert_media_file(&mut self, media: &MediaFile) -> StorageResult<()>;
    /// Deletes `edition` if no media file references it, then prunes its
    /// now-empty work ancestry as
    /// [`Storage::remove_media_file_and_prune`] does. An edition that still
    /// has media files, or doesn't exist, is left alone and yields an empty
    /// removal.
    async fn prune_edition(
        &mut self,
        edition: EditionId,
    ) -> StorageResult<crate::domain::MediaRemoval>;
    async fn upsert_tag(&mut self, tag: &Tag) -> StorageResult<TagId>;
    async fn assign_tag(
        &mut self,
        tag: TagId,
        target: TagTarget,
        weight: f32,
        source: &str,
    ) -> StorageResult<()>;

    async fn upsert_user(&mut self, user: &User) -> StorageResult<()>;
    async fn create_session(&mut self, session: &Session) -> StorageResult<()>;
    async fn upsert_image(&mut self, image: &crate::domain::Image) -> StorageResult<()>;
}

/// Run `f` inside a transaction. Commits on `Ok`, rolls back on `Err`.
///
/// The closure returns a boxed future whose lifetime is tied to the
/// `&mut dyn Transaction` borrow. The `for<'a>` HRTB + `+ 'a` on the
/// boxed future is what makes the variance work — without it, the
/// `&mut` borrow's invariance would force the returned future to a
/// shorter lifetime than the borrow, which the borrow checker rejects.
/// The `+ Send` keeps the future spawnable on the tokio runtime (used
/// by the scanner's watcher, where the scan runs on a background task).
pub async fn atomic<S, F, T>(storage: &S, f: F) -> StorageResult<T>
where
    S: Storage + ?Sized,
    F: for<'a> FnOnce(
        &'a mut (dyn Transaction + 'a),
    ) -> std::pin::Pin<Box<dyn Future<Output = StorageResult<T>> + Send + 'a>>,
{
    let mut tx: Box<dyn Transaction> = storage.begin().await?;
    let outcome = {
        let mut fut = f(tx.as_mut());
        fut.as_mut().await
    };
    match outcome {
        Ok(value) => {
            tx.commit().await?;
            Ok(value)
        }
        Err(err) => {
            // Don't mask the original error with a rollback failure —
            // operators care about the first one; the rollback failure
            // is at most worth a warning.
            if let Err(rb_err) = tx.rollback().await {
                tracing::warn!(error = %rb_err, "rollback after error failed");
            }
            Err(err)
        }
    }
}
