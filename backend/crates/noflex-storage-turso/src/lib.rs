// SPDX-License-Identifier: AGPL-3.0-only
//
// Turso backend — the default. Wraps `turso::Database` behind the
// canonical `Storage` trait. SQL stays inside the `repo` submodules; this
// file is wiring only.

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use noflex_core::{
    ApiKeyId, ChannelId, CollectionId, DevicePinId, EditionId, EmbeddingTarget, EpgProgramId,
    ExportJobId, ExternalIdKind, ImageKind, LibraryId, LiveTvSourceId, MediaFileId, PlayTarget,
    PlaylistId, RecordingId, RecordingScheduleId, SessionId, TagId, TagTarget, UserId, WatchRoomId,
    WorkId, WorkKind,
};
use noflex_storage::{
    domain::{
        ActivityKind, ApiKey, Collection, DevicePin, Edition, EpgProgram, ExportJob,
        ExportJobStatus, ExternalIdentity, Image, ImageSource, ImageTarget, IntroMarkers, Library,
        LiveTvChannel, LiveTvSource, MediaFile, MediaRemoval, Playlist, PluginOverrideRow,
        PluginTaskPayload, PluginTaskState, Recording, RecordingSchedule, RecordingStatus,
        RefreshTokenLookup, ScanEvent, Session, Tag, TagAssignment, TagSummary, TrickplayAsset,
        User, UserData, UserPluginSecret, UserPreference, UserRestrictions, WatchRoom,
        WatchRoomMember, WatchRoomState, Work,
    },
    error::{StorageError, StorageResult},
    query::{
        HybridQuery, ItemPage, ListTagsQuery, ListWorksQuery, Page, PageCursor, SearchMode,
        TagQuery,
    },
    search::{HealthReport, SearchHit, VectorHit},
    store::{ExportJobCancellation, ExportJobLease, Storage, Transaction},
};
use time::OffsetDateTime;
use tokio::{
    fs,
    io::AsyncWrite,
    sync::{OwnedRwLockReadGuard, RwLock},
};
use turso::Value;

mod connection;
mod convert;
mod error;
mod migrate;
mod pool;
mod repo;
mod transaction;
mod writer;

const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

macro_rules! with_write_transaction {
    ($store:expr, $conn:ident, $operation:expr) => {{
        let tx = transaction::begin($store.writer.clone(), $store.schema.clone()).await?;
        let $conn = tx.connection();
        match $operation.await {
            Ok(value) => {
                Box::new(tx).commit().await?;
                Ok(value)
            }
            Err(error) => {
                if let Err(rollback_error) = Box::new(tx).rollback().await {
                    tracing::warn!(%rollback_error, "write transaction rollback failed");
                }
                Err(error)
            }
        }
    }};
}

#[derive(Clone, Debug)]
pub struct TursoOptions {
    pub reader_connections: usize,
    pub busy_timeout: Duration,
}

impl Default for TursoOptions {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism().map_or(4, usize::from);
        Self {
            reader_connections: parallelism.clamp(2, 8),
            busy_timeout: DEFAULT_BUSY_TIMEOUT,
        }
    }
}

/// Turso-backed implementation of [`Storage`].
pub struct TursoStore {
    path: PathBuf,
    writer: writer::SharedWriter,
    readers: pool::ReaderPool,
    schema: Arc<RwLock<()>>,
}

impl TursoStore {
    pub async fn open(path: impl Into<PathBuf>) -> StorageResult<Self> {
        Self::open_with_options(path, TursoOptions::default()).await
    }

    pub async fn open_with_options(
        path: impl Into<PathBuf>,
        options: TursoOptions,
    ) -> StorageResult<Self> {
        let path = path.into();
        let path_str = path
            .to_str()
            .ok_or_else(|| error::backend_msg("db path is not valid UTF-8"))?
            .to_string();
        let db = connection::build_database(&path_str).await?;
        let readers =
            pool::ReaderPool::new(&db, options.reader_connections, options.busy_timeout).await?;
        let writer = writer::WriterSlot::new(db, options.busy_timeout).await?;
        Ok(Self {
            path,
            writer,
            readers,
            schema: Arc::new(RwLock::new(())),
        })
    }

    /// Open an in-memory store. Used by tests.
    pub async fn open_temp() -> StorageResult<Self> {
        Self::open(":memory:").await
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    async fn reader(&self) -> StorageResult<(OwnedRwLockReadGuard<()>, pool::ReaderLease)> {
        let schema = self.schema.clone().read_owned().await;
        let conn = self.readers.acquire().await?;
        Ok((schema, conn))
    }

    async fn writer(&self) -> StorageResult<(OwnedRwLockReadGuard<()>, writer::WriterLease)> {
        let schema = self.schema.clone().read_owned().await;
        let conn = writer::WriterLease::acquire(&self.writer).await?;
        Ok((schema, conn))
    }
}

fn map_turso(err: turso::Error) -> StorageError {
    crate::error::map_turso(err)
}

fn escape_sql_string(s: &str) -> String {
    s.replace('\'', "''")
}

#[async_trait]
impl Storage for TursoStore {
    // ---- Lifecycle ------------------------------------------------------
    async fn migrate(&self) -> StorageResult<()> {
        let _schema = self.schema.write().await;
        let conn = writer::WriterLease::acquire(&self.writer).await?;
        migrate::apply_all(&conn).await?;
        Ok(())
    }

    async fn health(&self) -> StorageResult<HealthReport> {
        let (_schema, conn) = self.reader().await?;
        let schema_version = migrate::schema_version(&conn).await.unwrap_or(0);
        Ok(HealthReport {
            backend: "turso".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            schema_version,
            healthy: true,
            notes: vec![],
        })
    }

    async fn backup_to(&self, writer: &mut (dyn AsyncWrite + Send + Unpin)) -> StorageResult<()> {
        let tmp = std::env::temp_dir().join(format!(
            "noflex-backup-{}-{}.db",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        {
            let _schema = self.schema.write().await;
            let conn = writer::WriterLease::acquire(&self.writer).await?;
            let sql = format!(
                "VACUUM INTO '{}'",
                escape_sql_string(&tmp.display().to_string())
            );
            conn.execute(&sql, ()).await.map_err(map_turso)?;
        }
        let result = async {
            let bytes = fs::read(&tmp)
                .await
                .map_err(|e| StorageError::Backend(Box::new(e)))?;
            tokio::io::copy(&mut bytes.as_slice(), writer)
                .await
                .map_err(|e| StorageError::Backend(Box::new(e)))?;
            Ok(())
        }
        .await;
        let _ = fs::remove_file(&tmp).await;
        result
    }

    async fn begin(&self) -> StorageResult<Box<dyn Transaction>> {
        let tx = transaction::begin(self.writer.clone(), self.schema.clone()).await?;
        Ok(Box::new(tx))
    }

    // ---- Works ----------------------------------------------------------
    async fn get_work(&self, id: WorkId) -> StorageResult<Option<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::works::get(&conn, id).await
    }
    async fn get_works(&self, ids: &[WorkId]) -> StorageResult<Vec<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::works::get_many(&conn, ids).await
    }
    async fn list_works(&self, q: &ListWorksQuery) -> StorageResult<ItemPage<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::list_works::list(&conn, q).await
    }
    async fn upsert_work(&self, work: &Work) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::works::upsert(&conn, work).await
    }
    async fn delete_work(&self, id: WorkId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::works::delete(&conn, id).await
    }
    async fn works_by_external_id(
        &self,
        kind: ExternalIdKind,
        value: &str,
    ) -> StorageResult<Vec<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::works::by_external_id(&conn, kind, value).await
    }

    // ---- Editions / Media files ----------------------------------------
    async fn editions_for_work(&self, work: WorkId) -> StorageResult<Vec<Edition>> {
        let (_schema, conn) = self.reader().await?;
        repo::editions::for_work(&conn, work).await
    }
    async fn get_edition(&self, id: EditionId) -> StorageResult<Option<Edition>> {
        let (_schema, conn) = self.reader().await?;
        repo::editions::get(&conn, id).await
    }
    async fn get_editions(&self, ids: &[EditionId]) -> StorageResult<Vec<Edition>> {
        let (_schema, conn) = self.reader().await?;
        repo::editions::get_many(&conn, ids).await
    }
    async fn upsert_edition(&self, ed: &Edition) -> StorageResult<EditionId> {
        let (_schema, conn) = self.writer().await?;
        repo::editions::upsert(&conn, ed).await
    }
    async fn delete_edition(&self, id: EditionId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::editions::delete(&conn, id).await
    }
    async fn set_intro_markers(
        &self,
        edition: EditionId,
        markers: IntroMarkers,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::editions::set_intro_markers(&conn, edition, &markers).await
    }
    async fn record_intro_scan(
        &self,
        edition: EditionId,
        plugin_id: &str,
        found: bool,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::editions::record_intro_scan(&conn, edition, plugin_id, found).await
    }
    async fn editions_missing_intro_scan(
        &self,
        plugin_id: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Edition>> {
        let (_schema, conn) = self.reader().await?;
        repo::editions::missing_intro_scan(&conn, plugin_id, limit, cursor.as_ref()).await
    }
    async fn media_files_for_edition(&self, edition: EditionId) -> StorageResult<Vec<MediaFile>> {
        let (_schema, conn) = self.reader().await?;
        repo::media_files::for_edition(&conn, edition).await
    }
    async fn upsert_media_file(&self, mf: &MediaFile) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::media_files::upsert(&conn, mf).await
    }
    async fn delete_media_file(&self, id: MediaFileId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::media_files::delete(&conn, id).await
    }
    async fn remove_media_file_and_prune(&self, id: MediaFileId) -> StorageResult<MediaRemoval> {
        with_write_transaction!(self, conn, repo::media_files::remove_and_prune(conn, id))
    }
    async fn media_file_by_path(&self, path: &str) -> StorageResult<Option<MediaFile>> {
        let (_schema, conn) = self.reader().await?;
        repo::media_files::by_path(&conn, path).await
    }
    async fn media_file_by_id(&self, id: MediaFileId) -> StorageResult<Option<MediaFile>> {
        let (_schema, conn) = self.reader().await?;
        repo::media_files::by_id(&conn, id).await
    }

    // ---- Trickplay ------------------------------------------------------
    async fn get_trickplay(&self, edition: EditionId) -> StorageResult<Option<TrickplayAsset>> {
        let (_schema, conn) = self.reader().await?;
        repo::trickplay::get(&conn, edition).await
    }
    async fn upsert_trickplay(&self, asset: &TrickplayAsset) -> StorageResult<()> {
        with_write_transaction!(self, conn, repo::trickplay::upsert(conn, asset))
    }
    async fn delete_trickplay(&self, edition: EditionId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::trickplay::delete(&conn, edition).await
    }
    async fn editions_missing_trickplay(
        &self,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Edition>> {
        let (_schema, conn) = self.reader().await?;
        repo::trickplay::missing(&conn, limit, cursor.as_ref()).await
    }

    // ---- Tags -----------------------------------------------------------
    async fn upsert_tag(&self, tag: &Tag) -> StorageResult<TagId> {
        let (_schema, conn) = self.writer().await?;
        repo::tags::upsert(&conn, tag).await
    }
    async fn get_tag(&self, id: TagId) -> StorageResult<Option<Tag>> {
        let (_schema, conn) = self.reader().await?;
        repo::tags::get_by_id(&conn, id).await
    }
    async fn list_tags(&self, q: &ListTagsQuery) -> StorageResult<ItemPage<TagSummary>> {
        let (_schema, conn) = self.reader().await?;
        repo::tags::list(&conn, q).await
    }
    async fn tag_by_slug(&self, ns: &str, slug: &str) -> StorageResult<Option<Tag>> {
        let (_schema, conn) = self.reader().await?;
        repo::tags::get_by_slug(&conn, ns, slug).await
    }
    async fn assign_tag_with_metadata(
        &self,
        tag: TagId,
        target: TagTarget,
        weight: f32,
        source: &str,
        metadata: &serde_json::Value,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::tags::assign(&conn, tag, target, weight, source, metadata).await
    }
    async fn unassign_tag(&self, tag: TagId, target: TagTarget) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::tags::unassign(&conn, tag, target).await
    }
    async fn tags_for_target(&self, target: TagTarget) -> StorageResult<Vec<Tag>> {
        let (_schema, conn) = self.reader().await?;
        repo::tags::tags_for_target(&conn, target).await
    }
    async fn tag_assignments_for_target(
        &self,
        target: TagTarget,
    ) -> StorageResult<Vec<TagAssignment>> {
        let (_schema, conn) = self.reader().await?;
        repo::tags::assignments_for_target(&conn, target).await
    }
    async fn search_works_by_tags(&self, q: &TagQuery) -> StorageResult<Page<WorkId>> {
        let (_schema, conn) = self.reader().await?;
        let Some(first) = q.include_tags.first() else {
            return Ok(Page::empty());
        };
        let page = repo::tags::search_works(&conn, *first, q.limit).await?;
        Ok(Page {
            items: page.items.into_iter().map(WorkId::from_uuid).collect(),
            next_cursor: page.next_cursor,
        })
    }

    // ---- Users / sessions ----------------------------------------------
    async fn get_user(&self, id: UserId) -> StorageResult<Option<User>> {
        let (_schema, conn) = self.reader().await?;
        repo::users::get(&conn, id).await
    }
    async fn get_user_by_name(&self, name: &str) -> StorageResult<Option<User>> {
        let (_schema, conn) = self.reader().await?;
        repo::users::get_by_name(&conn, name).await
    }
    async fn upsert_user(&self, user: &User) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::users::upsert(&conn, user).await
    }
    async fn create_initial_admin(&self, user: &User) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::users::create_initial_admin(&conn, user).await
    }
    async fn delete_user(&self, id: UserId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::users::delete(&conn, id).await
    }
    async fn list_users(&self) -> StorageResult<Vec<User>> {
        let (_schema, conn) = self.reader().await?;
        repo::users::list_all(&conn).await
    }

    async fn external_identity(
        &self,
        plugin_id: &str,
        issuer: &str,
        subject: &str,
    ) -> StorageResult<Option<ExternalIdentity>> {
        let (_schema, conn) = self.reader().await?;
        repo::external_identities::get(&conn, plugin_id, issuer, subject).await
    }

    async fn list_external_identities(&self, user: UserId) -> StorageResult<Vec<ExternalIdentity>> {
        let (_schema, conn) = self.reader().await?;
        repo::external_identities::list(&conn, user).await
    }

    async fn link_external_identity(&self, identity: &ExternalIdentity) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::external_identities::link(&conn, identity).await
    }

    async fn unlink_external_identity(&self, user: UserId, plugin_id: &str) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::external_identities::unlink(&conn, user, plugin_id).await
    }

    async fn list_user_preferences(&self, user: UserId) -> StorageResult<Vec<UserPreference>> {
        let (_schema, conn) = self.reader().await?;
        repo::user_preferences::list(&conn, user).await
    }
    async fn set_user_preference(&self, preference: &UserPreference) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::user_preferences::set(&conn, preference).await
    }
    async fn delete_user_preference(&self, user: UserId, key: &str) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::user_preferences::delete(&conn, user, key).await
    }

    async fn create_session(&self, session: &Session) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::sessions::create(&conn, session).await
    }
    async fn get_session(&self, id: SessionId) -> StorageResult<Option<Session>> {
        let (_schema, conn) = self.reader().await?;
        repo::sessions::get(&conn, id).await
    }
    async fn list_sessions_for_user(&self, user: UserId) -> StorageResult<Vec<Session>> {
        let (_schema, conn) = self.reader().await?;
        repo::sessions::list_for_user(&conn, user).await
    }
    async fn touch_session(&self, id: SessionId, at: OffsetDateTime) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::sessions::touch(&conn, id, at).await
    }
    async fn revoke_session(&self, id: SessionId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::sessions::revoke(&conn, id).await
    }
    async fn session_by_refresh_token(&self, hash: &str) -> StorageResult<RefreshTokenLookup> {
        let (_schema, conn) = self.reader().await?;
        repo::sessions::get_by_refresh_token(&conn, hash).await
    }
    async fn rotate_refresh_token(
        &self,
        session_id: SessionId,
        expected_hash: &str,
        new_hash: &str,
        new_expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> StorageResult<bool> {
        with_write_transaction!(
            self,
            conn,
            repo::sessions::rotate_refresh_token(
                conn,
                session_id,
                expected_hash,
                new_hash,
                new_expires_at,
                now,
            )
        )
    }

    async fn grant_library_access(&self, user: UserId, library: LibraryId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::library_access::grant(&conn, user, library).await
    }
    async fn revoke_library_access(&self, user: UserId, library: LibraryId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::library_access::revoke(&conn, user, library).await
    }
    async fn list_user_libraries(&self, user: UserId) -> StorageResult<Vec<LibraryId>> {
        let (_schema, conn) = self.reader().await?;
        repo::library_access::list_for_user(&conn, user).await
    }

    async fn get_user_restrictions(&self, user: UserId) -> StorageResult<Option<UserRestrictions>> {
        let (_schema, conn) = self.reader().await?;
        repo::restrictions::get(&conn, user).await
    }
    async fn set_user_restrictions(&self, restrictions: &UserRestrictions) -> StorageResult<()> {
        with_write_transaction!(self, conn, repo::restrictions::set(conn, restrictions))
    }
    async fn delete_user_restrictions(&self, user: UserId) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::restrictions::delete(&conn, user).await
    }
    async fn work_for_edition(&self, edition: EditionId) -> StorageResult<Option<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::restrictions::work_for_edition(&conn, edition).await
    }
    async fn work_for_media_file(&self, media: MediaFileId) -> StorageResult<Option<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::restrictions::work_for_media(&conn, media).await
    }
    async fn user_can_access_work(&self, user: UserId, work: WorkId) -> StorageResult<bool> {
        let (_schema, conn) = self.reader().await?;
        repo::restrictions::can_access(&conn, user, work).await
    }
    async fn accessible_work_ids(
        &self,
        user: UserId,
        works: &[WorkId],
    ) -> StorageResult<Vec<WorkId>> {
        let (_schema, conn) = self.reader().await?;
        repo::restrictions::accessible_work_ids(&conn, user, works).await
    }

    async fn create_watch_room(&self, room: &WatchRoom) -> StorageResult<()> {
        with_write_transaction!(self, conn, repo::watch_rooms::create(conn, room))
    }
    async fn get_watch_room(&self, id: WatchRoomId) -> StorageResult<Option<WatchRoom>> {
        let (_schema, conn) = self.reader().await?;
        repo::watch_rooms::get(&conn, id).await
    }
    async fn list_watch_rooms(&self, user: UserId) -> StorageResult<Vec<WatchRoom>> {
        let (_schema, conn) = self.reader().await?;
        repo::watch_rooms::list(&conn, user).await
    }
    async fn list_watch_room_members(
        &self,
        room: WatchRoomId,
    ) -> StorageResult<Vec<WatchRoomMember>> {
        let (_schema, conn) = self.reader().await?;
        repo::watch_rooms::members(&conn, room).await
    }
    async fn join_watch_room(&self, member: &WatchRoomMember) -> StorageResult<Option<WatchRoom>> {
        with_write_transaction!(self, conn, repo::watch_rooms::join(conn, member))
    }
    async fn leave_watch_room(
        &self,
        room: WatchRoomId,
        user: UserId,
    ) -> StorageResult<Option<WatchRoom>> {
        with_write_transaction!(self, conn, repo::watch_rooms::leave(conn, room, user))
    }
    async fn control_watch_room(
        &self,
        room: WatchRoomId,
        host: UserId,
        state: Option<WatchRoomState>,
        position_ms: Option<u64>,
        updated_at: OffsetDateTime,
    ) -> StorageResult<Option<WatchRoom>> {
        let (_schema, conn) = self.writer().await?;
        repo::watch_rooms::control(&conn, room, host, state, position_ms, updated_at).await
    }

    async fn list_all_sessions(&self) -> StorageResult<Vec<Session>> {
        let (_schema, conn) = self.reader().await?;
        repo::sessions::list_all(&conn).await
    }
    async fn count_user_sessions(&self, user: UserId) -> StorageResult<u32> {
        let (_schema, conn) = self.reader().await?;
        repo::sessions::count_user(&conn, user).await
    }
    async fn revoke_user_sessions(&self, user: UserId) -> StorageResult<u32> {
        let (_schema, conn) = self.writer().await?;
        repo::sessions::revoke_user(&conn, user).await
    }
    async fn prune_sessions_before(&self, before: OffsetDateTime) -> StorageResult<u32> {
        let (_schema, conn) = self.writer().await?;
        repo::sessions::prune_before(&conn, before).await
    }

    async fn create_api_key(
        &self,
        user_id: UserId,
        name: &str,
        key_hash: &str,
    ) -> StorageResult<ApiKey> {
        let (_schema, conn) = self.writer().await?;
        repo::api_keys::create(&conn, user_id, name, key_hash).await
    }
    async fn api_key_by_hash(&self, key_hash: &str) -> StorageResult<Option<ApiKey>> {
        let (_schema, conn) = self.reader().await?;
        repo::api_keys::get_by_hash(&conn, key_hash).await
    }
    async fn list_api_keys(&self, user_id: UserId) -> StorageResult<Vec<ApiKey>> {
        let (_schema, conn) = self.reader().await?;
        repo::api_keys::list_for_user(&conn, user_id).await
    }
    async fn revoke_api_key(&self, id: ApiKeyId) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::api_keys::revoke(&conn, id).await
    }

    async fn create_device_pin(&self, pin: &DevicePin) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::device_pins::create(&conn, pin).await
    }
    async fn device_pin_by_pin(&self, pin: &str) -> StorageResult<Option<DevicePin>> {
        let (_schema, conn) = self.writer().await?;
        repo::device_pins::get_by_pin(&conn, pin).await
    }
    async fn authorize_device_pin(&self, pin: &str, user_id: UserId) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::device_pins::authorize(&conn, pin, user_id).await
    }
    async fn delete_device_pin(&self, id: DevicePinId) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::device_pins::delete(&conn, id).await
    }

    async fn user_data(&self, user: UserId, target: PlayTarget) -> StorageResult<UserData> {
        let (_schema, conn) = self.reader().await?;
        repo::user_data::get(&conn, user, target).await
    }
    async fn record_playback_progress(
        &self,
        user: UserId,
        target: PlayTarget,
        position_ms: i64,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        let mut data = repo::user_data::get(&conn, user, target).await?;
        data.position_ms = Some(position_ms);
        data.last_played = Some(time::OffsetDateTime::now_utc());
        repo::user_data::write(&conn, user, target, &data).await
    }
    async fn mark_played(
        &self,
        user: UserId,
        target: PlayTarget,
        played: bool,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        let mut data = repo::user_data::get(&conn, user, target).await?;
        data.played = played;
        // Bumps on the 0 -> 1 transition only; re-marking after an
        // unmark, or repeat calls, leave play_count alone. A
        // watched-then-unwatched-then-watched-again sequence counts
        // as one play.
        if played && data.play_count == 0 {
            data.play_count = 1;
        }
        data.last_played = Some(time::OffsetDateTime::now_utc());
        repo::user_data::write(&conn, user, target, &data).await
    }
    async fn set_favorite(
        &self,
        user: UserId,
        target: PlayTarget,
        favorite: bool,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        let mut data = repo::user_data::get(&conn, user, target).await?;
        data.favorite = favorite;
        repo::user_data::write(&conn, user, target, &data).await
    }
    async fn continue_watching(
        &self,
        user: UserId,
        active_since: time::OffsetDateTime,
        limit: u32,
    ) -> StorageResult<Vec<Work>> {
        let (_schema, conn) = self.reader().await?;
        let ids = repo::user_data::continue_watching(&conn, user, active_since, limit).await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(w) = repo::works::get(&conn, id).await? {
                out.push(w);
            }
        }
        Ok(out)
    }
    async fn create_invite(
        &self,
        invite: &noflex_storage::domain::Invite,
        token_hash: &str,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::multi_server::create_invite(&conn, invite, token_hash).await
    }
    async fn list_invites(&self) -> StorageResult<Vec<noflex_storage::domain::Invite>> {
        let (_schema, conn) = self.reader().await?;
        repo::multi_server::list_invites(&conn).await
    }
    async fn consume_invite(
        &self,
        token_hash: &str,
        now: time::OffsetDateTime,
    ) -> StorageResult<Option<noflex_storage::domain::Invite>> {
        let (_schema, conn) = self.writer().await?;
        repo::multi_server::consume_invite(&conn, token_hash, now).await
    }
    async fn release_invite_use(&self, id: noflex_core::InviteId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::multi_server::release_invite_use(&conn, id).await
    }
    async fn revoke_invite(
        &self,
        id: noflex_core::InviteId,
        now: time::OffsetDateTime,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::multi_server::revoke_invite(&conn, id, now).await
    }
    async fn upsert_linked_server(
        &self,
        server: &noflex_storage::domain::LinkedServer,
    ) -> StorageResult<noflex_storage::domain::LinkedServer> {
        let (_schema, conn) = self.writer().await?;
        repo::multi_server::upsert_linked_server(&conn, server).await
    }
    async fn linked_servers_for_user(
        &self,
        user: UserId,
    ) -> StorageResult<Vec<noflex_storage::domain::LinkedServer>> {
        let (_schema, conn) = self.reader().await?;
        repo::multi_server::linked_servers_for_user(&conn, user).await
    }
    async fn delete_linked_server(
        &self,
        user: UserId,
        id: noflex_core::LinkedServerId,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::multi_server::delete_linked_server(&conn, user, id).await
    }
    async fn linked_server_origins(&self) -> StorageResult<Vec<String>> {
        let (_schema, conn) = self.reader().await?;
        repo::multi_server::linked_origins(&conn).await
    }
    async fn apply_watch_state(
        &self,
        user: UserId,
        target: PlayTarget,
        played: bool,
        position_ms: Option<i64>,
        last_played: time::OffsetDateTime,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        let mut data = repo::user_data::get(&conn, user, target).await?;
        if played && data.play_count == 0 {
            data.play_count = 1;
        }
        data.played = played;
        data.position_ms = position_ms;
        data.last_played = Some(last_played);
        repo::user_data::write(&conn, user, target, &data).await
    }
    async fn episodes_after(
        &self,
        series: WorkId,
        season: i32,
        number: i32,
        limit: u32,
    ) -> StorageResult<Vec<Work>> {
        let (_schema, conn) = self.reader().await?;
        let ids = repo::user_data::episodes_after(&conn, series, season, number, limit).await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(work) = repo::works::get(&conn, id).await? {
                out.push(work);
            }
        }
        Ok(out)
    }
    async fn upsert_device(&self, device: &noflex_storage::domain::Device) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::devices::upsert(&conn, device).await
    }
    async fn get_device(
        &self,
        id: noflex_core::DeviceId,
    ) -> StorageResult<Option<noflex_storage::domain::Device>> {
        let (_schema, conn) = self.reader().await?;
        repo::devices::get(&conn, id).await
    }
    async fn devices_for_user(
        &self,
        user: UserId,
    ) -> StorageResult<Vec<noflex_storage::domain::Device>> {
        let (_schema, conn) = self.reader().await?;
        repo::devices::for_user(&conn, user).await
    }
    async fn devices_wanting_versions(&self) -> StorageResult<Vec<noflex_storage::domain::Device>> {
        let (_schema, conn) = self.reader().await?;
        repo::devices::smart_transcode(&conn).await
    }
    async fn delete_device(&self, user: UserId, id: noflex_core::DeviceId) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::devices::delete(&conn, user, id).await
    }
    async fn enqueue_optimized_version(
        &self,
        version: &noflex_storage::domain::OptimizedVersion,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::optimized_versions::enqueue(&conn, version).await
    }
    async fn get_optimized_version(
        &self,
        id: noflex_core::OptimizedVersionId,
    ) -> StorageResult<Option<noflex_storage::domain::OptimizedVersion>> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::get(&conn, id).await
    }
    async fn optimized_versions_for_media(
        &self,
        media: MediaFileId,
    ) -> StorageResult<Vec<noflex_storage::domain::OptimizedVersion>> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::for_media(&conn, media).await
    }
    async fn list_optimized_versions(
        &self,
    ) -> StorageResult<Vec<noflex_storage::domain::OptimizedVersion>> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::list(&conn).await
    }
    async fn claim_optimized_version(
        &self,
    ) -> StorageResult<Option<noflex_storage::domain::OptimizedVersion>> {
        let (_schema, conn) = self.writer().await?;
        repo::optimized_versions::claim(&conn).await
    }
    async fn update_optimized_version(
        &self,
        version: &noflex_storage::domain::OptimizedVersion,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::optimized_versions::update(&conn, version).await
    }
    async fn delete_optimized_version(
        &self,
        id: noflex_core::OptimizedVersionId,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::optimized_versions::delete(&conn, id).await
    }
    async fn requeue_running_optimized_versions(&self) -> StorageResult<u64> {
        let (_schema, conn) = self.writer().await?;
        repo::optimized_versions::requeue_running(&conn).await
    }
    async fn tonemap_curve(&self) -> StorageResult<Option<noflex_core::TonemapCurve>> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::tonemap_curve(&conn).await
    }
    async fn set_tonemap_curve(
        &self,
        curve: Option<noflex_core::TonemapCurve>,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::optimized_versions::set_tonemap_curve(&conn, curve).await
    }
    async fn smart_transcoding_settings(
        &self,
    ) -> StorageResult<noflex_storage::domain::SmartTranscodingSettings> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::settings(&conn).await
    }
    async fn set_smart_transcoding_settings(
        &self,
        settings: &noflex_storage::domain::SmartTranscodingSettings,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::optimized_versions::set_settings(&conn, settings).await
    }
    async fn content_category_counts(
        &self,
        access_user_id: Option<UserId>,
    ) -> StorageResult<Vec<noflex_storage::domain::ContentCategoryCount>> {
        let (_schema, conn) = self.reader().await?;
        repo::list_works::content_category_counts(&conn, access_user_id).await
    }
    async fn watched_top_level_works(
        &self,
        user: UserId,
    ) -> StorageResult<Vec<noflex_storage::domain::WatchedWork>> {
        let (_schema, conn) = self.reader().await?;
        repo::user_data::watched_top_level_works(&conn, user).await
    }
    async fn work_features(
        &self,
        kinds: &[WorkKind],
        tag_namespaces: &[&str],
        access_user_id: Option<UserId>,
    ) -> StorageResult<Vec<noflex_storage::domain::WorkFeatures>> {
        let (_schema, conn) = self.reader().await?;
        repo::list_works::work_features(&conn, kinds, tag_namespaces, access_user_id).await
    }
    async fn delete_work_embeddings(&self, work: WorkId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::search::delete_work_embeddings(&conn, work).await
    }
    async fn work_embeddings(
        &self,
        namespace: &str,
        works: &[WorkId],
    ) -> StorageResult<Vec<(WorkId, Vec<f32>)>> {
        let (_schema, conn) = self.reader().await?;
        repo::search::work_embeddings(&conn, namespace, works).await
    }

    // ---- Libraries / scans ---------------------------------------------
    async fn list_libraries(&self) -> StorageResult<Vec<Library>> {
        let (_schema, conn) = self.reader().await?;
        repo::libraries::list_all(&conn).await
    }
    async fn upsert_library(&self, library: &Library) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::libraries::upsert(&conn, library).await
    }
    async fn delete_library(&self, id: LibraryId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::libraries::delete(&conn, id).await
    }
    async fn record_scan_event(&self, event: &ScanEvent) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::scan_events::record(&conn, event).await
    }
    async fn library_scan_version(&self, library: LibraryId) -> StorageResult<u32> {
        let (_schema, conn) = self.reader().await?;
        repo::libraries::scan_version(&conn, library).await
    }
    async fn set_library_scan_version(
        &self,
        library: LibraryId,
        version: u32,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::libraries::set_scan_version(&conn, library, version).await
    }

    async fn upsert_plugin_override(
        &self,
        plugin_id: &str,
        hook: &str,
        work_kind: &str,
        priority: i32,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::plugin_overrides::upsert(&conn, plugin_id, hook, work_kind, priority).await
    }

    async fn list_plugin_overrides(&self) -> StorageResult<Vec<PluginOverrideRow>> {
        let (_schema, conn) = self.reader().await?;
        repo::plugin_overrides::list(&conn).await
    }

    async fn delete_plugin_override(
        &self,
        plugin_id: &str,
        hook: &str,
        work_kind: &str,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::plugin_overrides::delete(&conn, plugin_id, hook, work_kind).await
    }

    async fn plugin_task_state(
        &self,
        plugin_id: &str,
        task: &str,
    ) -> StorageResult<Option<PluginTaskState>> {
        let (_schema, conn) = self.reader().await?;
        repo::plugin_tasks::get_state(&conn, plugin_id, task).await
    }
    async fn set_plugin_task_state(&self, state: &PluginTaskState) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::plugin_tasks::set_state(&conn, state).await
    }
    async fn delete_plugin_task_states(&self, plugin_id: &str) -> StorageResult<u64> {
        let (_schema, conn) = self.writer().await?;
        repo::plugin_tasks::delete_states(&conn, plugin_id).await
    }
    async fn enqueue_plugin_task(
        &self,
        plugin_id: &str,
        task: &str,
        payload: Option<&str>,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::plugin_tasks::enqueue(&conn, plugin_id, task, payload).await
    }
    async fn dequeue_plugin_task(
        &self,
        plugin_id: &str,
        task: &str,
    ) -> StorageResult<Option<PluginTaskPayload>> {
        let (_schema, conn) = self.writer().await?;
        repo::plugin_tasks::dequeue(&conn, plugin_id, task).await
    }
    async fn list_plugin_task_states(
        &self,
        plugin_id: &str,
    ) -> StorageResult<Vec<PluginTaskState>> {
        let (_schema, conn) = self.reader().await?;
        repo::plugin_tasks::list_states(&conn, plugin_id).await
    }
    async fn clear_plugin_task_queue(&self, plugin_id: &str) -> StorageResult<u64> {
        let (_schema, conn) = self.writer().await?;
        repo::plugin_tasks::clear_queue(&conn, plugin_id).await
    }

    async fn set_user_plugin_secret(
        &self,
        user: UserId,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::user_plugin_secrets::set(&conn, user, plugin_id, key, value).await
    }
    async fn user_plugin_secret(
        &self,
        user: UserId,
        plugin_id: &str,
        key: &str,
    ) -> StorageResult<Option<UserPluginSecret>> {
        let (_schema, conn) = self.reader().await?;
        repo::user_plugin_secrets::get(&conn, user, plugin_id, key).await
    }
    async fn list_user_plugin_secrets(
        &self,
        user: UserId,
        plugin_id: &str,
    ) -> StorageResult<Vec<UserPluginSecret>> {
        let (_schema, conn) = self.reader().await?;
        repo::user_plugin_secrets::list(&conn, user, plugin_id).await
    }
    async fn delete_user_plugin_secrets(
        &self,
        user: UserId,
        plugin_id: &str,
    ) -> StorageResult<u64> {
        let (_schema, conn) = self.writer().await?;
        repo::user_plugin_secrets::delete(&conn, user, plugin_id).await
    }

    async fn append_activity(
        &self,
        kind: ActivityKind,
        actor: Option<&str>,
        subject: Option<&str>,
        detail: Option<&str>,
        occurred_at: time::OffsetDateTime,
    ) -> StorageResult<i64> {
        let (_schema, conn) = self.writer().await?;
        repo::activity::append(&conn, kind.as_str(), actor, subject, detail, occurred_at).await
    }

    async fn list_activity(
        &self,
        kinds: &[ActivityKind],
        since: Option<time::OffsetDateTime>,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<noflex_storage::domain::ActivityEntry>> {
        let (_schema, conn) = self.reader().await?;
        let kind_strs: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
        repo::activity::list(&conn, &kind_strs, since, limit, cursor).await
    }

    async fn upsert_play_session(
        &self,
        session: &noflex_storage::domain::PlaySession,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::play_sessions::upsert(&conn, session).await
    }

    async fn open_play_session(
        &self,
        user: UserId,
        work: WorkId,
        seen_since: OffsetDateTime,
    ) -> StorageResult<Option<noflex_storage::domain::PlaySession>> {
        let (_schema, conn) = self.reader().await?;
        repo::play_sessions::open(&conn, user, work, seen_since).await
    }

    async fn get_play_session(
        &self,
        id: noflex_core::PlaySessionId,
    ) -> StorageResult<Option<noflex_storage::domain::PlaySession>> {
        let (_schema, conn) = self.reader().await?;
        repo::play_sessions::get(&conn, id).await
    }

    async fn list_play_sessions(
        &self,
        query: &noflex_storage::query::PlaySessionQuery,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<noflex_storage::domain::PlaySession>> {
        let (_schema, conn) = self.reader().await?;
        repo::play_sessions::page(&conn, query, limit, cursor).await
    }

    async fn play_sessions_matching(
        &self,
        query: &noflex_storage::query::PlaySessionQuery,
    ) -> StorageResult<Vec<noflex_storage::domain::PlaySession>> {
        let (_schema, conn) = self.reader().await?;
        repo::play_sessions::list(&conn, query, None, None).await
    }

    async fn recently_added_works(
        &self,
        limit: u32,
    ) -> StorageResult<Vec<noflex_storage::domain::Work>> {
        let cap = limit.max(1) as i64;
        let (_schema, conn) = self.reader().await?;
        let mut rows = conn
            .query(
                "SELECT id, kind, parent_id, library_id, title, sort_title, \
                        year, release_date, overview, external_ids_json, \
                        index_number, index_end, parent_index, metadata_status, \
                        added_at, updated_at, tagline, original_title, \
                        community_rating, locked_fields_json \
                 FROM works \
                 WHERE parent_id IS NULL \
                 ORDER BY added_at DESC \
                 LIMIT ?1",
                [Value::Integer(cap)],
            )
            .await
            .map_err(map_turso)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso)? {
            out.push(repo::works::row_to_work(&row)?);
        }
        Ok(out)
    }

    async fn recently_played_for_user(
        &self,
        user: UserId,
        limit: u32,
    ) -> StorageResult<Vec<noflex_storage::domain::Work>> {
        let cap = limit.max(1) as i64;
        let user_blob = crate::convert::uuid_to_blob(user.into_uuid());
        let (_schema, conn) = self.reader().await?;
        let mut rows = conn
            .query(
                "SELECT w.id, w.kind, w.parent_id, w.library_id, w.title, w.sort_title, \
                        w.year, w.release_date, w.overview, w.external_ids_json, \
                        w.index_number, w.index_end, w.parent_index, w.metadata_status, \
                        w.added_at, w.updated_at, w.tagline, w.original_title, \
                        w.community_rating, w.locked_fields_json \
                 FROM user_data ud \
                 JOIN works w ON w.id = ud.target_id \
                 WHERE ud.user_id = ?1 AND ud.target_kind = 'work' AND ud.last_played IS NOT NULL \
                 ORDER BY ud.last_played DESC \
                 LIMIT ?2",
                [Value::Blob(user_blob), Value::Integer(cap)],
            )
            .await
            .map_err(map_turso)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso)? {
            out.push(repo::works::row_to_work(&row)?);
        }
        Ok(out)
    }

    async fn list_play_history(
        &self,
        user: UserId,
        cursor: Option<PageCursor>,
        limit: u32,
    ) -> StorageResult<noflex_storage::domain::HistoryPage> {
        let (_schema, conn) = self.reader().await?;
        repo::user_data::play_history(&conn, user, cursor, limit).await
    }

    async fn list_media_files_for_library(
        &self,
        library: LibraryId,
    ) -> StorageResult<Vec<MediaFile>> {
        let (_schema, conn) = self.reader().await?;
        repo::media_files::for_library(&conn, library).await
    }

    // ---- Images --------------------------------------------------------
    async fn upsert_image(&self, image: &Image) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::images::upsert(&conn, image).await
    }
    async fn image(
        &self,
        target: ImageTarget,
        kind: ImageKind,
        order: &[ImageSource],
    ) -> StorageResult<Option<Image>> {
        let (_schema, conn) = self.reader().await?;
        repo::images::get(&conn, target, kind, order).await
    }
    async fn list_images(
        &self,
        target: ImageTarget,
        order: &[ImageSource],
    ) -> StorageResult<Vec<Image>> {
        let (_schema, conn) = self.reader().await?;
        repo::images::list(&conn, target, order).await
    }
    async fn list_work_images_for_library(&self, library: LibraryId) -> StorageResult<Vec<Image>> {
        let (_schema, conn) = self.reader().await?;
        repo::images::list_for_library_works(&conn, library).await
    }
    async fn delete_image(
        &self,
        target: ImageTarget,
        kind: ImageKind,
        source: ImageSource,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::images::delete(&conn, target, kind, source).await
    }
    async fn works_missing_image(
        &self,
        kind: WorkKind,
        image: ImageKind,
        sources: &[ImageSource],
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::images::works_missing(&conn, kind, image, sources, limit, cursor.as_ref()).await
    }
    async fn identified_works_missing_image_source(
        &self,
        source: ImageSource,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::images::identified_works_missing_source(&conn, source, limit, cursor.as_ref()).await
    }

    // ---- Search --------------------------------------------------------
    async fn search_lexical(
        &self,
        query: &str,
        kinds: &[WorkKind],
        access_user_id: Option<UserId>,
        limit: u32,
    ) -> StorageResult<Vec<SearchHit>> {
        let (_schema, conn) = self.reader().await?;
        repo::search::search_lexical(&conn, query, kinds, access_user_id, limit).await
    }

    async fn upsert_embedding(
        &self,
        target: EmbeddingTarget,
        namespace: &str,
        model_id: &str,
        vector: &[f32],
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::search::upsert_embedding(&conn, target, namespace, model_id, vector).await
    }

    async fn search_vector(
        &self,
        namespace: &str,
        query_vector: &[f32],
        kinds: &[WorkKind],
        access_user_id: Option<UserId>,
        limit: u32,
    ) -> StorageResult<Vec<VectorHit>> {
        let (_schema, conn) = self.reader().await?;
        repo::search::search_vector(&conn, namespace, query_vector, kinds, access_user_id, limit)
            .await
    }

    async fn works_missing_embedding(
        &self,
        namespace: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<Page<Work>> {
        let (_schema, conn) = self.reader().await?;
        repo::search::works_missing_embedding(&conn, namespace, limit, cursor.as_ref()).await
    }

    async fn search_hybrid(&self, query: &HybridQuery) -> StorageResult<Vec<SearchHit>> {
        use std::collections::HashMap;

        const RRF_K: f32 = 60.0;

        let (_schema, conn) = self.reader().await?;
        let lexical = if matches!(query.mode, SearchMode::Lexical | SearchMode::Hybrid) {
            repo::search::search_lexical(
                &conn,
                &query.text,
                &query.kinds,
                query.access_user_id,
                query.limit * 2,
            )
            .await?
        } else {
            Vec::new()
        };
        let vector = if matches!(query.mode, SearchMode::Vector | SearchMode::Hybrid) {
            if let Some(vector) = &query.query_vector {
                repo::search::search_vector(
                    &conn,
                    &query.vector_namespace,
                    vector,
                    &query.kinds,
                    query.access_user_id,
                    query.limit * 2,
                )
                .await?
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let mut fused: HashMap<WorkId, (f32, Vec<String>)> = HashMap::new();
        for (rank, hit) in lexical.into_iter().enumerate() {
            let entry = fused.entry(hit.work).or_default();
            entry.0 += 1.0 / (RRF_K + rank as f32 + 1.0);
            entry.1.extend(hit.matched_fields);
        }
        for (rank, hit) in vector.into_iter().enumerate() {
            let entry = fused.entry(hit.work).or_default();
            entry.0 += 1.0 / (RRF_K + rank as f32 + 1.0);
            entry.1.push("vector".to_string());
        }
        let mut hits = fused
            .into_iter()
            .map(|(work, (score, matched_fields))| SearchHit {
                work,
                access_checked: query.access_user_id.is_some(),
                score,
                matched_fields,
            })
            .collect::<Vec<_>>();
        hits.sort_by(noflex_storage::search::hybrid_order);
        hits.truncate(query.limit as usize);
        Ok(hits)
    }

    // ---- Export jobs ---------------------------------------------------
    async fn create_export_job(&self, job: &ExportJob) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::export_jobs::create(&conn, job).await
    }
    async fn get_export_job(&self, id: ExportJobId) -> StorageResult<Option<ExportJob>> {
        let (_schema, conn) = self.reader().await?;
        repo::export_jobs::get(&conn, id).await
    }
    async fn list_export_jobs(
        &self,
        user: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<ExportJob>> {
        let (_schema, conn) = self.reader().await?;
        repo::export_jobs::list_for_user(&conn, user, limit, cursor).await
    }
    async fn claim_export_job(
        &self,
        worker_id: &str,
        lease_token: &str,
        now: OffsetDateTime,
        lease_expires_at: OffsetDateTime,
        max_attempts: u32,
    ) -> StorageResult<Option<ExportJobLease>> {
        with_write_transaction!(
            self,
            conn,
            repo::export_jobs::claim(
                conn,
                worker_id,
                lease_token,
                now,
                lease_expires_at,
                max_attempts,
            )
        )
    }
    async fn update_export_job_progress(
        &self,
        id: ExportJobId,
        lease_token: &str,
        progress: f32,
        lease_expires_at: OffsetDateTime,
    ) -> StorageResult<Option<ExportJobStatus>> {
        let (_schema, conn) = self.writer().await?;
        repo::export_jobs::update_progress(&conn, id, lease_token, progress, lease_expires_at).await
    }
    async fn complete_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
        output_path: &str,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::export_jobs::complete(&conn, id, lease_token, output_path).await
    }
    async fn fail_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
        error: &str,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::export_jobs::fail(&conn, id, lease_token, error).await
    }
    async fn cancel_claimed_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::export_jobs::cancel_claimed(&conn, id, lease_token).await
    }
    async fn request_export_job_cancellation(
        &self,
        id: ExportJobId,
        owner: UserId,
    ) -> StorageResult<Option<ExportJobCancellation>> {
        with_write_transaction!(
            self,
            conn,
            repo::export_jobs::request_cancellation(conn, id, owner)
        )
    }
    async fn delete_export_job(&self, id: ExportJobId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::export_jobs::delete(&conn, id).await
    }

    // ---- Collections ---------------------------------------------------
    async fn create_collection(&self, collection: &Collection) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::collections::create(&conn, collection).await
    }
    async fn update_collection(&self, collection: &Collection) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::collections::update(&conn, collection).await
    }
    async fn delete_collection(&self, id: CollectionId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::collections::delete(&conn, id).await
    }
    async fn get_collection(&self, id: CollectionId) -> StorageResult<Option<Collection>> {
        let (_schema, conn) = self.reader().await?;
        repo::collections::get(&conn, id).await
    }
    async fn list_collections(
        &self,
        library_id: LibraryId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<Collection>> {
        let (_schema, conn) = self.reader().await?;
        repo::collections::list_for_library(&conn, library_id, limit, cursor).await
    }
    async fn set_collection_items(&self, id: CollectionId, items: &[WorkId]) -> StorageResult<()> {
        with_write_transaction!(self, conn, repo::collections::set_items(conn, id, items))
    }
    async fn collection_items(
        &self,
        id: CollectionId,
        principal: noflex_storage::RequestUser,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<WorkId>> {
        let (_schema, conn) = self.reader().await?;
        repo::collections::items(&conn, id, principal, limit, cursor).await
    }

    // ---- Playlists ------------------------------------------------------
    async fn create_playlist(&self, playlist: &Playlist) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::playlists::create(&conn, playlist).await
    }
    async fn update_playlist(&self, playlist: &Playlist) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::playlists::update(&conn, playlist).await
    }
    async fn delete_playlist(&self, id: PlaylistId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::playlists::delete(&conn, id).await
    }
    async fn get_playlist(&self, id: PlaylistId) -> StorageResult<Option<Playlist>> {
        let (_schema, conn) = self.reader().await?;
        repo::playlists::get(&conn, id).await
    }
    async fn list_playlists(
        &self,
        owner: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<Playlist>> {
        let (_schema, conn) = self.reader().await?;
        repo::playlists::list_for_owner(&conn, owner, limit, cursor).await
    }
    async fn set_playlist_items(&self, id: PlaylistId, items: &[PlayTarget]) -> StorageResult<()> {
        with_write_transaction!(self, conn, repo::playlists::set_items(conn, id, items))
    }
    async fn playlist_items(
        &self,
        id: PlaylistId,
        principal: noflex_storage::RequestUser,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<PlayTarget>> {
        let (_schema, conn) = self.reader().await?;
        repo::playlists::items(&conn, id, principal, limit, cursor).await
    }
    async fn append_playlist_items(
        &self,
        id: PlaylistId,
        items: &[PlayTarget],
    ) -> StorageResult<()> {
        with_write_transaction!(self, conn, repo::playlists::append_items(conn, id, items))
    }

    async fn upsert_live_tv_source(&self, source: &LiveTvSource) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::live_tv::upsert_source(&conn, source).await
    }
    async fn get_live_tv_source(&self, id: LiveTvSourceId) -> StorageResult<Option<LiveTvSource>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::get_source(&conn, id).await
    }
    async fn list_live_tv_sources(&self) -> StorageResult<Vec<LiveTvSource>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::list_sources(&conn).await
    }
    async fn delete_live_tv_source(&self, id: LiveTvSourceId) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::live_tv::delete_source(&conn, id).await
    }
    async fn apply_live_tv_refresh(
        &self,
        source: LiveTvSourceId,
        channels: &[LiveTvChannel],
        programs: &[EpgProgram],
        refreshed_at: OffsetDateTime,
    ) -> StorageResult<()> {
        with_write_transaction!(
            self,
            conn,
            repo::live_tv::apply_refresh(conn, source, channels, programs, refreshed_at)
        )
    }
    async fn record_live_tv_refresh_error(
        &self,
        source: LiveTvSourceId,
        error: &str,
        attempted_at: OffsetDateTime,
    ) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::live_tv::record_refresh_error(&conn, source, error, attempted_at).await
    }
    async fn get_live_tv_channel(&self, id: ChannelId) -> StorageResult<Option<LiveTvChannel>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::get_channel(&conn, id).await
    }
    async fn list_live_tv_channels(
        &self,
        source: Option<LiveTvSourceId>,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<LiveTvChannel>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::list_channels(&conn, source, limit, cursor).await
    }
    async fn list_live_tv_guide(
        &self,
        channels: &[ChannelId],
        start: OffsetDateTime,
        end: OffsetDateTime,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<EpgProgram>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::guide(&conn, channels, start, end, limit, cursor).await
    }
    async fn get_epg_program(&self, id: EpgProgramId) -> StorageResult<Option<EpgProgram>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::get_program(&conn, id).await
    }
    async fn create_recording_schedule(&self, s: &RecordingSchedule) -> StorageResult<()> {
        let (_schema, conn) = self.writer().await?;
        repo::live_tv::create_schedule(&conn, s).await
    }
    async fn get_recording_schedule(
        &self,
        id: RecordingScheduleId,
    ) -> StorageResult<Option<RecordingSchedule>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::get_schedule(&conn, id).await
    }
    async fn list_recording_schedules(
        &self,
        owner: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<RecordingSchedule>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::list_schedules(&conn, owner, limit, cursor).await
    }
    async fn cancel_recording_schedule(
        &self,
        id: RecordingScheduleId,
        owner: UserId,
        now: OffsetDateTime,
    ) -> StorageResult<Option<RecordingSchedule>> {
        with_write_transaction!(
            self,
            conn,
            repo::live_tv::cancel_schedule(conn, id, owner, now)
        )
    }
    async fn claim_due_recording(
        &self,
        now: OffsetDateTime,
    ) -> StorageResult<Option<(RecordingSchedule, Recording)>> {
        with_write_transaction!(self, conn, repo::live_tv::claim_due(conn, now))
    }
    async fn get_recording(&self, id: RecordingId) -> StorageResult<Option<Recording>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::get_recording(&conn, id).await
    }
    async fn recording_for_schedule(
        &self,
        id: RecordingScheduleId,
    ) -> StorageResult<Option<Recording>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::recording_for_schedule(&conn, id).await
    }
    async fn list_recordings(
        &self,
        owner: UserId,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> StorageResult<ItemPage<Recording>> {
        let (_schema, conn) = self.reader().await?;
        repo::live_tv::list_recordings(&conn, owner, limit, cursor).await
    }
    async fn finish_recording(
        &self,
        id: RecordingId,
        status: RecordingStatus,
        path: Option<&str>,
        error: Option<&str>,
        size: Option<u64>,
        at: OffsetDateTime,
    ) -> StorageResult<bool> {
        let (_schema, conn) = self.writer().await?;
        repo::live_tv::update_recording(&conn, id, status, path, error, size, at).await
    }
    async fn delete_recording(
        &self,
        id: RecordingId,
        owner: UserId,
    ) -> StorageResult<Option<Recording>> {
        let (_schema, conn) = self.writer().await?;
        repo::live_tv::delete_recording(&conn, id, owner).await
    }
    async fn fail_running_recordings(&self, at: OffsetDateTime, error: &str) -> StorageResult<u64> {
        let (_schema, conn) = self.writer().await?;
        repo::live_tv::fail_running(&conn, at, error).await
    }
}

#[cfg(test)]
mod concurrency_tests {
    use std::{sync::Arc, time::Duration};

    use noflex_storage::Storage;
    use tokio::time::timeout;
    use turso::Value;

    use super::{TursoOptions, TursoStore};

    async fn store() -> TursoStore {
        TursoStore::open_with_options(
            ":memory:",
            TursoOptions {
                reader_connections: 2,
                busy_timeout: Duration::from_secs(1),
            },
        )
        .await
        .unwrap()
    }

    async fn scalar(conn: &turso::Connection, sql: &str) -> i64 {
        let mut rows = conn.query(sql, ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        match row.get_value(0).unwrap() {
            Value::Integer(value) => value,
            value => panic!("expected integer, got {value:?}"),
        }
    }

    #[tokio::test]
    async fn readers_are_distinct_connections_and_share_memory_database() {
        let store = store().await;
        {
            let (_schema, writer) = store.writer().await.unwrap();
            writer
                .execute("CREATE TABLE shared(value INTEGER)", ())
                .await
                .unwrap();
            writer
                .execute("INSERT INTO shared VALUES (7)", ())
                .await
                .unwrap();
        }
        let (_schema, first) = store.reader().await.unwrap();
        first
            .execute("CREATE TEMP TABLE connection_marker(value INTEGER)", ())
            .await
            .unwrap();

        let (_schema, second) = store.reader().await.unwrap();
        assert_eq!(
            scalar(
                &second,
                "SELECT count(*) FROM sqlite_temp_master WHERE name = 'connection_marker'"
            )
            .await,
            0
        );
        assert_eq!(scalar(&second, "SELECT value FROM shared").await, 7);
    }

    #[tokio::test]
    async fn readers_never_observe_an_uncommitted_writer_update() {
        let store = Arc::new(store().await);
        {
            let (_schema, writer) = store.writer().await.unwrap();
            writer
                .execute("CREATE TABLE values_under_test(value INTEGER)", ())
                .await
                .unwrap();
            writer
                .execute("INSERT INTO values_under_test VALUES (0)", ())
                .await
                .unwrap();
        }

        let (_schema, writer) = store.writer().await.unwrap();
        writer.execute("BEGIN IMMEDIATE", ()).await.unwrap();
        writer
            .execute("UPDATE values_under_test SET value = 1", ())
            .await
            .unwrap();
        let (_schema, reader) = store.reader().await.unwrap();
        assert_eq!(
            scalar(&reader, "SELECT value FROM values_under_test").await,
            0
        );
        drop(reader);
        writer.execute("COMMIT", ()).await.unwrap();
        drop(writer);

        let (_schema, reader) = store.reader().await.unwrap();
        assert_eq!(
            scalar(&reader, "SELECT value FROM values_under_test").await,
            1
        );
    }

    #[tokio::test]
    async fn dropped_transaction_rolls_back_before_releasing_writer() {
        let store = store().await;
        let transaction = store.begin().await.unwrap();
        drop(transaction);

        let recovered = timeout(Duration::from_secs(1), store.begin())
            .await
            .expect("writer lease was not recovered")
            .unwrap();
        recovered.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn writer_left_mid_transaction_is_rolled_back_before_reuse() {
        let store = store().await;
        {
            let (_schema, writer) = store.writer().await.unwrap();
            writer
                .execute("CREATE TABLE leaked(value INTEGER)", ())
                .await
                .unwrap();
            writer.execute("BEGIN IMMEDIATE", ()).await.unwrap();
            writer
                .execute("INSERT INTO leaked VALUES (1)", ())
                .await
                .unwrap();
        }
        let (_schema, writer) = timeout(Duration::from_secs(1), store.writer())
            .await
            .expect("writer was not recovered")
            .unwrap();
        assert!(writer.is_autocommit().unwrap());
        assert_eq!(scalar(&writer, "SELECT count(*) FROM leaked").await, 0);
        drop(writer);
        let transaction = timeout(Duration::from_secs(1), store.begin())
            .await
            .expect("transaction after recovery")
            .unwrap();
        transaction.commit().await.unwrap();
    }

    #[tokio::test]
    async fn writer_transactions_serialize() {
        let store = store().await;
        let first = store.begin().await.unwrap();
        assert!(
            timeout(Duration::from_millis(25), store.begin())
                .await
                .is_err()
        );
        first.rollback().await.unwrap();
        let second = timeout(Duration::from_secs(1), store.begin())
            .await
            .unwrap()
            .unwrap();
        second.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_transaction_releases_the_writer() {
        let store = Arc::new(store().await);
        let (started, wait) = tokio::sync::oneshot::channel();
        let task = tokio::spawn({
            let store = store.clone();
            async move {
                let transaction = store.begin().await.unwrap();
                let _ = started.send(());
                std::future::pending::<()>().await;
                drop(transaction);
            }
        });
        wait.await.unwrap();
        task.abort();
        let _ = task.await;

        let transaction = timeout(Duration::from_secs(1), store.begin())
            .await
            .expect("cancelled transaction did not release writer")
            .unwrap();
        transaction.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_dequeues_are_unique_and_migrations_are_idempotent() {
        let store = Arc::new(store().await);
        store.migrate().await.unwrap();
        store.migrate().await.unwrap();
        store
            .enqueue_plugin_task("plugin", "task", Some("first"))
            .await
            .unwrap();
        store
            .enqueue_plugin_task("plugin", "task", Some("second"))
            .await
            .unwrap();

        let (first, second) = tokio::join!(
            store.dequeue_plugin_task("plugin", "task"),
            store.dequeue_plugin_task("plugin", "task")
        );
        let first = first.unwrap().unwrap();
        let second = second.unwrap().unwrap();
        assert_ne!(first.id, second.id);
        assert!(
            store
                .dequeue_plugin_task("plugin", "task")
                .await
                .unwrap()
                .is_none()
        );
    }
}
