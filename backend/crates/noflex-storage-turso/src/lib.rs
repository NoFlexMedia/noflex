// SPDX-License-Identifier: AGPL-3.0-only
//
// Turso backend — the default. Wraps `turso::Database` behind the
// canonical `Storage` trait. SQL stays inside the `repo` submodules; this
// file is wiring only.

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use noflex_core::{
    ApiKeyId, ChannelId, CollectionId, DeviceCodeId, EditionId, EmbeddingTarget, EpgProgramId,
    ExportJobId, ExternalIdKind, ImageKind, LibraryId, LiveTvSourceId, MediaFileId, PlayTarget,
    PlaylistId, RecordingId, RecordingScheduleId, SessionId, TagId, TagTarget, UserId, WatchRoomId,
    WorkId, WorkKind,
};
use noflex_storage::{
    domain::{
        AccountPurge, ActivityKind, ApiKey, Collection, DeviceCode, Edition, EpgProgram, ExportJob,
        ExportJobStatus, ExternalIdentity, Image, ImageSource, ImageTarget, IntroMarkers, Library,
        LiveTvChannel, LiveTvSource, MediaFile, MediaRemoval, Playlist, PluginOverrideRow,
        PluginTaskPayload, PluginTaskState, Recording, RecordingSchedule, RecordingStatus,
        RefreshTokenLookup, ScanEvent, ServerReset, Session, Tag, TagAssignment, TagSummary,
        TrickplayAsset, User, UserData, UserPluginSecret, UserPreference, UserRestrictions,
        WatchRoom, WatchRoomMember, WatchRoomState, Work,
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
use transaction::Lane;
use turso::Value;

mod connection;
mod convert;
mod error;
mod migrate;
mod pool;
mod repo;
mod transaction;

const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `$operation`, a future over `$conn`, in one write transaction on
/// `$lane`: committed if it returns `Ok`, rolled back if not. Losing a
/// conflict to another writer runs it again from the start, until the busy
/// timeout passes, so `$operation` must touch nothing but the database.
macro_rules! transact {
    ($store:expr, $lane:expr, |$conn:ident| $operation:expr) => {{
        let mut retry = transaction::Retry::new($store.busy_timeout);
        loop {
            let attempt = async {
                let tx = transaction::begin(&$store.writers, &$store.gates, $lane).await?;
                let outcome = {
                    let $conn = tx.connection();
                    $operation.await
                };
                tx.finish(outcome).await
            }
            .await;
            match attempt {
                Err(StorageError::Transient(reason)) if retry.wait().await => {
                    tracing::debug!(%reason, "write transaction conflicted, retrying");
                }
                attempt => break attempt,
            }
        }
    }};
}

#[derive(Clone, Debug)]
pub struct TursoOptions {
    pub reader_connections: usize,
    /// Connections write transactions run on, and so how many can be open at
    /// once.
    pub writer_connections: usize,
    /// How long a write keeps retrying conflicts, and a statement waits on a
    /// lock, before failing with [`StorageError::Transient`].
    pub busy_timeout: Duration,
}

impl Default for TursoOptions {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism().map_or(4, usize::from);
        Self {
            reader_connections: parallelism.clamp(2, 8),
            writer_connections: parallelism.clamp(2, 8),
            busy_timeout: DEFAULT_BUSY_TIMEOUT,
        }
    }
}

/// Turso-backed implementation of [`Storage`].
pub struct TursoStore {
    path: PathBuf,
    writers: pool::Pool,
    readers: pool::Pool,
    gates: transaction::Gates,
    busy_timeout: Duration,
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
            pool::Pool::new(db.clone(), options.reader_connections, options.busy_timeout)?;
        let writers = pool::Pool::new(db, options.writer_connections, options.busy_timeout)?;
        Ok(Self {
            path,
            writers,
            readers,
            gates: transaction::Gates {
                schema: Arc::new(RwLock::new(())),
                gate: Arc::new(RwLock::new(())),
            },
            busy_timeout: options.busy_timeout,
        })
    }

    /// Open an in-memory store. Used by tests.
    pub async fn open_temp() -> StorageResult<Self> {
        Self::open(":memory:").await
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    async fn reader(&self) -> StorageResult<(OwnedRwLockReadGuard<()>, pool::Lease)> {
        let schema = self.gates.schema.clone().read_owned().await;
        let conn = self.readers.acquire().await?;
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
        let _schema = self.gates.schema.write().await;
        let conn = self.writers.acquire().await?;
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
            let _schema = self.gates.schema.write().await;
            let conn = self.writers.acquire().await?;
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
        // The scanner's transactions insert and prune works, editions and media
        // files, which a concurrent writer could be referencing.
        let tx = transaction::begin(&self.writers, &self.gates, Lane::Serial).await?;
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
    async fn work_facets(
        &self,
        q: &noflex_storage::WorkFacetsQuery,
    ) -> StorageResult<Vec<noflex_storage::WorkFacetValues>> {
        let (_schema, conn) = self.reader().await?;
        repo::work_filters::facets(&conn, q).await
    }
    async fn upsert_work(&self, work: &Work) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::works::upsert(
            conn, work
        ))
    }
    async fn delete_work(&self, id: WorkId) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::works::delete(conn, id))
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
        transact!(self, Lane::Concurrent, |conn| repo::editions::upsert(
            conn, ed
        ))
    }
    async fn delete_edition(&self, id: EditionId) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::editions::delete(conn, id))
    }
    async fn set_intro_markers(
        &self,
        edition: EditionId,
        markers: IntroMarkers,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::editions::set_intro_markers(conn, edition, &markers)
        })
    }
    async fn record_intro_scan(
        &self,
        edition: EditionId,
        plugin_id: &str,
        found: bool,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::editions::record_intro_scan(conn, edition, plugin_id, found)
        })
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
        transact!(self, Lane::Concurrent, |conn| repo::media_files::upsert(
            conn, mf
        ))
    }
    async fn delete_media_file(&self, id: MediaFileId) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::media_files::delete(
            conn, id
        ))
    }
    async fn remove_media_file_and_prune(&self, id: MediaFileId) -> StorageResult<MediaRemoval> {
        transact!(self, Lane::Serial, |conn| {
            repo::media_files::remove_and_prune(conn, id)
        })
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
        transact!(self, Lane::Concurrent, |conn| repo::trickplay::upsert(
            conn, asset
        ))
    }
    async fn delete_trickplay(&self, edition: EditionId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::trickplay::delete(
            conn, edition
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::tags::upsert(conn, tag))
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
        transact!(self, Lane::Concurrent, |conn| repo::tags::assign(
            conn, tag, target, weight, source, metadata
        ))
    }
    async fn unassign_tag(&self, tag: TagId, target: TagTarget) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::tags::unassign(
            conn, tag, target
        ))
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
    async fn get_user_by_username(&self, username: &str) -> StorageResult<Option<User>> {
        let (_schema, conn) = self.reader().await?;
        repo::users::get_by_username(&conn, username).await
    }
    async fn upsert_user(&self, user: &User) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::users::upsert(
            conn, user
        ))
    }
    async fn create_initial_admin(&self, user: &User) -> StorageResult<bool> {
        transact!(
            self,
            Lane::Serial,
            |conn| repo::users::create_initial_admin(conn, user)
        )
    }
    async fn set_user_avatar(
        &self,
        user: UserId,
        image: &[u8],
        at: OffsetDateTime,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::users::set_avatar(
            conn, user, image, at
        ))
    }
    async fn set_user_pin(&self, user: UserId, pin_hash: Option<&str>) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::users::set_pin(
            conn, user, pin_hash
        ))
    }
    async fn user_pin_hash(&self, user: UserId) -> StorageResult<Option<String>> {
        let (_schema, conn) = self.reader().await?;
        repo::users::pin_hash(&conn, user).await
    }
    async fn user_avatar(&self, user: UserId) -> StorageResult<Option<Vec<u8>>> {
        let (_schema, conn) = self.reader().await?;
        repo::users::avatar(&conn, user).await
    }
    async fn delete_user_avatar(&self, user: UserId) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::users::delete_avatar(
            conn, user
        ))
    }
    async fn delete_user(&self, id: UserId) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::users::delete(conn, id))
    }
    async fn list_users(&self) -> StorageResult<Vec<User>> {
        let (_schema, conn) = self.reader().await?;
        repo::users::list_all(&conn).await
    }

    async fn delete_account(
        &self,
        user: UserId,
        at: OffsetDateTime,
    ) -> StorageResult<Option<AccountPurge>> {
        transact!(self, Lane::Serial, |conn| repo::erasure::delete_account(
            conn, user, at
        ))
    }
    async fn pending_account_purges(&self) -> StorageResult<Vec<AccountPurge>> {
        let (_schema, conn) = self.reader().await?;
        repo::erasure::pending_purges(&conn).await
    }
    async fn forget_user_activity(&self, user: UserId) -> StorageResult<u64> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::erasure::forget_activity(conn, user)
        })
    }
    async fn finish_account_purge(&self, user: UserId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::erasure::finish_purge(
            conn, user
        ))
    }
    async fn request_server_reset(
        &self,
        admin: UserId,
        reset: &ServerReset,
    ) -> StorageResult<bool> {
        transact!(self, Lane::Serial, |conn| repo::erasure::request_reset(
            conn, admin, reset
        ))
    }
    async fn pending_server_reset(&self) -> StorageResult<Option<ServerReset>> {
        let (_schema, conn) = self.reader().await?;
        repo::erasure::pending_reset(&conn).await
    }
    async fn erase_all_data(&self) -> StorageResult<()> {
        // Held as a migration holds it, so no query sees a table half emptied.
        let _schema = self.gates.schema.write().await;
        let conn = self.writers.acquire().await?;
        repo::erasure::erase_all(&conn).await
    }
    async fn finish_server_reset(&self) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::erasure::finish_reset(conn))
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::external_identities::link(conn, identity)
        })
    }

    async fn unlink_external_identity(&self, user: UserId, plugin_id: &str) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::external_identities::unlink(conn, user, plugin_id)
        })
    }

    async fn list_user_preferences(&self, user: UserId) -> StorageResult<Vec<UserPreference>> {
        let (_schema, conn) = self.reader().await?;
        repo::user_preferences::list(&conn, user).await
    }
    async fn set_user_preference(&self, preference: &UserPreference) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::user_preferences::set(
            conn, preference
        ))
    }
    async fn delete_user_preference(&self, user: UserId, key: &str) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::user_preferences::delete(conn, user, key)
        })
    }

    async fn create_session(&self, session: &Session) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::sessions::create(
            conn, session
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::sessions::touch(
            conn, id, at
        ))
    }
    async fn revoke_session(&self, id: SessionId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::sessions::revoke(
            conn, id
        ))
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::sessions::rotate_refresh_token(
                conn,
                session_id,
                expected_hash,
                new_hash,
                new_expires_at,
                now,
            )
        })
    }

    async fn lock_session(&self, id: SessionId, at: OffsetDateTime) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::sessions::lock(
            conn, id, at
        ))
    }
    async fn unlock_session(&self, id: SessionId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::sessions::unlock(
            conn, id
        ))
    }
    async fn record_pin_failure(&self, id: SessionId) -> StorageResult<u32> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::sessions::record_pin_failure(conn, id)
        })
    }

    async fn grant_library_access(&self, user: UserId, library: LibraryId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::library_access::grant(
            conn, user, library
        ))
    }
    async fn revoke_library_access(&self, user: UserId, library: LibraryId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::library_access::revoke(
            conn, user, library
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::restrictions::set(
            conn,
            restrictions
        ))
    }
    async fn delete_user_restrictions(&self, user: UserId) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::restrictions::delete(
            conn, user
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::watch_rooms::create(
            conn, room
        ))
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
        transact!(self, Lane::Serial, |conn| repo::watch_rooms::join(
            conn, member
        ))
    }
    async fn leave_watch_room(
        &self,
        room: WatchRoomId,
        user: UserId,
    ) -> StorageResult<Option<WatchRoom>> {
        transact!(self, Lane::Serial, |conn| repo::watch_rooms::leave(
            conn, room, user
        ))
    }
    async fn control_watch_room(
        &self,
        room: WatchRoomId,
        host: UserId,
        state: Option<WatchRoomState>,
        position_ms: Option<u64>,
        updated_at: OffsetDateTime,
    ) -> StorageResult<Option<WatchRoom>> {
        transact!(self, Lane::Concurrent, |conn| repo::watch_rooms::control(
            conn,
            room,
            host,
            state,
            position_ms,
            updated_at
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::sessions::revoke_user(
            conn, user
        ))
    }
    async fn prune_sessions_before(&self, before: OffsetDateTime) -> StorageResult<u32> {
        transact!(self, Lane::Concurrent, |conn| repo::sessions::prune_before(
            conn, before
        ))
    }

    async fn create_api_key(
        &self,
        user_id: UserId,
        name: &str,
        key_hash: &str,
    ) -> StorageResult<ApiKey> {
        transact!(self, Lane::Concurrent, |conn| repo::api_keys::create(
            conn, user_id, name, key_hash
        ))
    }
    async fn api_key_by_hash(&self, key_hash: &str) -> StorageResult<Option<ApiKey>> {
        let (_schema, conn) = self.reader().await?;
        repo::api_keys::get_by_hash(&conn, key_hash).await
    }
    async fn get_api_key(&self, id: ApiKeyId) -> StorageResult<Option<ApiKey>> {
        let (_schema, conn) = self.reader().await?;
        repo::api_keys::get(&conn, id).await
    }
    async fn list_api_keys(&self, user_id: UserId) -> StorageResult<Vec<ApiKey>> {
        let (_schema, conn) = self.reader().await?;
        repo::api_keys::list_for_user(&conn, user_id).await
    }
    async fn revoke_api_key(&self, id: ApiKeyId) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::api_keys::revoke(
            conn, id
        ))
    }

    async fn create_device_code(&self, code: &DeviceCode) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::device_codes::create(
            conn, code
        ))
    }
    async fn device_code_by_code(&self, code: &str) -> StorageResult<Option<DeviceCode>> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::device_codes::get_by_code(conn, code)
        })
    }
    async fn authorize_device_code(&self, code: &str, user_id: UserId) -> StorageResult<bool> {
        transact!(
            self,
            Lane::Concurrent,
            |conn| repo::device_codes::authorize(conn, code, user_id)
        )
    }
    async fn delete_device_code(&self, id: DeviceCodeId) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::device_codes::delete(
            conn, id
        ))
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
        transact!(self, Lane::Concurrent, |conn| async {
            let mut data = repo::user_data::get(conn, user, target).await?;
            data.position_ms = Some(position_ms);
            data.last_played = Some(time::OffsetDateTime::now_utc());
            repo::user_data::write(conn, user, target, &data).await
        })
    }
    async fn mark_played(
        &self,
        user: UserId,
        target: PlayTarget,
        played: bool,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| async {
            let mut data = repo::user_data::get(conn, user, target).await?;
            data.played = played;
            // Bumps on the 0 -> 1 transition only; re-marking after an
            // unmark, or repeat calls, leave play_count alone. A
            // watched-then-unwatched-then-watched-again sequence counts
            // as one play.
            if played && data.play_count == 0 {
                data.play_count = 1;
            }
            data.last_played = Some(time::OffsetDateTime::now_utc());
            repo::user_data::write(conn, user, target, &data).await
        })
    }
    async fn set_favorite(
        &self,
        user: UserId,
        target: PlayTarget,
        favorite: bool,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| async {
            let mut data = repo::user_data::get(conn, user, target).await?;
            data.favorite = favorite;
            repo::user_data::write(conn, user, target, &data).await
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::multi_server::create_invite(conn, invite, token_hash)
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::multi_server::consume_invite(conn, token_hash, now)
        })
    }
    async fn release_invite_use(&self, id: noflex_core::InviteId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::multi_server::release_invite_use(conn, id)
        })
    }
    async fn revoke_invite(
        &self,
        id: noflex_core::InviteId,
        now: time::OffsetDateTime,
    ) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::multi_server::revoke_invite(conn, id, now)
        })
    }
    async fn upsert_linked_server(
        &self,
        server: &noflex_storage::domain::LinkedServer,
    ) -> StorageResult<noflex_storage::domain::LinkedServer> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::multi_server::upsert_linked_server(conn, server)
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::multi_server::delete_linked_server(conn, user, id)
        })
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
        transact!(self, Lane::Concurrent, |conn| async {
            let mut data = repo::user_data::get(conn, user, target).await?;
            if played && data.play_count == 0 {
                data.play_count = 1;
            }
            data.played = played;
            data.position_ms = position_ms;
            data.last_played = Some(last_played);
            repo::user_data::write(conn, user, target, &data).await
        })
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
        transact!(self, Lane::Concurrent, |conn| repo::devices::upsert(
            conn, device
        ))
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
    async fn add_smart_transcode_mark(
        &self,
        mark: &noflex_storage::domain::SmartTranscodeMark,
    ) -> StorageResult<noflex_storage::domain::SmartTranscodeMark> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::smart_transcode_marks::add(conn, mark)
        })
    }
    async fn remove_smart_transcode_mark(
        &self,
        user: UserId,
        work: WorkId,
        device: Option<noflex_core::DeviceId>,
    ) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::smart_transcode_marks::remove(conn, user, work, device)
        })
    }
    async fn smart_transcode_marks_for_user(
        &self,
        user: UserId,
    ) -> StorageResult<Vec<noflex_storage::domain::SmartTranscodeMark>> {
        let (_schema, conn) = self.reader().await?;
        repo::smart_transcode_marks::for_user(&conn, user).await
    }
    async fn list_smart_transcode_marks(
        &self,
    ) -> StorageResult<Vec<noflex_storage::domain::SmartTranscodeMark>> {
        let (_schema, conn) = self.reader().await?;
        repo::smart_transcode_marks::list(&conn).await
    }
    async fn record_smart_transcode_plan(
        &self,
        mark: &noflex_storage::domain::SmartTranscodeMark,
        outcome: noflex_storage::domain::SmartTranscodeOutcome,
        media_file_ids: &[noflex_core::MediaFileId],
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::smart_transcode_marks::record_plan(conn, mark, outcome, media_file_ids)
        })
    }
    async fn devices_wanting_versions(&self) -> StorageResult<Vec<noflex_storage::domain::Device>> {
        let (_schema, conn) = self.reader().await?;
        repo::devices::smart_transcode(&conn).await
    }
    async fn delete_device(&self, user: UserId, id: noflex_core::DeviceId) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::devices::delete(
            conn, user, id
        ))
    }
    async fn enqueue_optimized_version(
        &self,
        version: &noflex_storage::domain::OptimizedVersion,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::enqueue(conn, version)
        })
    }
    async fn mark_optimized_version_unwanted(
        &self,
        id: noflex_core::OptimizedVersionId,
        at: OffsetDateTime,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::mark_unwanted(conn, id, at)
        })
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
    async fn optimized_versions_page(
        &self,
        status: Option<noflex_storage::domain::OptimizedVersionStatus>,
        limit: u32,
        after: Option<noflex_storage::PageCursor>,
    ) -> StorageResult<noflex_storage::ItemPage<noflex_storage::domain::OptimizedVersion>> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::page(&conn, status, limit, after).await
    }
    async fn optimized_version_stats(
        &self,
    ) -> StorageResult<noflex_storage::domain::OptimizedVersionStats> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::stats(&conn).await
    }
    async fn claim_optimized_version(
        &self,
    ) -> StorageResult<Option<noflex_storage::domain::OptimizedVersion>> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::claim(conn)
        })
    }
    async fn update_optimized_version(
        &self,
        version: &noflex_storage::domain::OptimizedVersion,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::update(conn, version)
        })
    }
    async fn delete_optimized_version(
        &self,
        id: noflex_core::OptimizedVersionId,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::delete(conn, id)
        })
    }
    async fn requeue_running_optimized_versions(&self) -> StorageResult<u64> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::requeue_running(conn)
        })
    }
    async fn tonemap_curve(&self) -> StorageResult<Option<noflex_core::TonemapCurve>> {
        let (_schema, conn) = self.reader().await?;
        repo::optimized_versions::tonemap_curve(&conn).await
    }
    async fn set_tonemap_curve(
        &self,
        curve: Option<noflex_core::TonemapCurve>,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::set_tonemap_curve(conn, curve)
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::optimized_versions::set_settings(conn, settings)
        })
    }
    async fn downloads_allowed(&self) -> StorageResult<bool> {
        let (_schema, conn) = self.reader().await?;
        repo::server_settings::downloads_allowed(&conn).await
    }
    async fn set_downloads_allowed(&self, allowed: bool) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::server_settings::set_downloads_allowed(conn, allowed)
        })
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
    async fn category_work_ids(
        &self,
        category: noflex_storage::domain::ContentCategory,
        access_user_id: Option<UserId>,
    ) -> StorageResult<Vec<WorkId>> {
        let (_schema, conn) = self.reader().await?;
        repo::list_works::category_work_ids(&conn, category, access_user_id).await
    }
    async fn active_embedding_model(
        &self,
    ) -> StorageResult<Option<noflex_storage::domain::EmbeddingModel>> {
        let (_schema, conn) = self.reader().await?;
        repo::search::active_embedding_model(&conn).await
    }
    async fn activate_embedding_model(
        &self,
        model: &noflex_storage::domain::EmbeddingModel,
    ) -> StorageResult<Option<noflex_storage::domain::EmbeddingModel>> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::search::activate_embedding_model(conn, model)
        })
    }
    async fn prune_embeddings(&self, namespace: &str) -> StorageResult<u64> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::search::prune_embeddings(conn, namespace)
        })
    }
    async fn embedding_endpoint(
        &self,
    ) -> StorageResult<Option<noflex_storage::domain::EmbeddingEndpoint>> {
        let (_schema, conn) = self.reader().await?;
        repo::search::embedding_endpoint(&conn).await
    }
    async fn set_embedding_endpoint(
        &self,
        endpoint: Option<&noflex_storage::domain::EmbeddingEndpoint>,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::search::set_embedding_endpoint(conn, endpoint)
        })
    }
    async fn delete_work_embeddings(&self, work: WorkId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::search::delete_work_embeddings(conn, work)
        })
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
        transact!(self, Lane::Concurrent, |conn| repo::libraries::upsert(
            conn, library
        ))
    }
    async fn delete_library(&self, id: LibraryId) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::libraries::delete(conn, id))
    }
    async fn record_scan_event(&self, event: &ScanEvent) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::scan_events::record(
            conn, event
        ))
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::libraries::set_scan_version(conn, library, version)
        })
    }

    async fn upsert_plugin_override(
        &self,
        plugin_id: &str,
        hook: &str,
        work_kind: &str,
        priority: i32,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::plugin_overrides::upsert(conn, plugin_id, hook, work_kind, priority)
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::plugin_overrides::delete(conn, plugin_id, hook, work_kind)
        })
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
        transact!(
            self,
            Lane::Concurrent,
            |conn| repo::plugin_tasks::set_state(conn, state)
        )
    }
    async fn delete_plugin_task_states(&self, plugin_id: &str) -> StorageResult<u64> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::plugin_tasks::delete_states(conn, plugin_id)
        })
    }
    async fn enqueue_plugin_task(
        &self,
        plugin_id: &str,
        task: &str,
        payload: Option<&str>,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::plugin_tasks::enqueue(
            conn, plugin_id, task, payload
        ))
    }
    async fn dequeue_plugin_task(
        &self,
        plugin_id: &str,
        task: &str,
    ) -> StorageResult<Option<PluginTaskPayload>> {
        transact!(self, Lane::Concurrent, |conn| repo::plugin_tasks::dequeue(
            conn, plugin_id, task
        ))
    }
    async fn list_plugin_task_states(
        &self,
        plugin_id: &str,
    ) -> StorageResult<Vec<PluginTaskState>> {
        let (_schema, conn) = self.reader().await?;
        repo::plugin_tasks::list_states(&conn, plugin_id).await
    }
    async fn clear_plugin_task_queue(&self, plugin_id: &str) -> StorageResult<u64> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::plugin_tasks::clear_queue(conn, plugin_id)
        })
    }

    async fn set_user_plugin_secret(
        &self,
        user: UserId,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::user_plugin_secrets::set(conn, user, plugin_id, key, value)
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::user_plugin_secrets::delete(conn, user, plugin_id)
        })
    }

    async fn append_activity(
        &self,
        kind: ActivityKind,
        actor: Option<&str>,
        subject: Option<&str>,
        detail: Option<&str>,
        occurred_at: time::OffsetDateTime,
    ) -> StorageResult<i64> {
        transact!(self, Lane::Concurrent, |conn| repo::activity::append(
            conn,
            kind.as_str(),
            actor,
            subject,
            detail,
            occurred_at
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::play_sessions::upsert(
            conn, session
        ))
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
                        community_rating, locked_fields_json, metadata_provider \
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
                        w.community_rating, w.locked_fields_json, w.metadata_provider \
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

    async fn upsert_provider_attribution(
        &self,
        attribution: &noflex_storage::domain::ProviderAttribution,
        logo: Option<&[u8]>,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::provider_attributions::upsert(conn, attribution, logo)
        })
    }

    async fn delete_provider_attribution(&self, provider: &str) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::provider_attributions::delete(conn, provider)
        })
    }

    async fn list_provider_attributions(
        &self,
    ) -> StorageResult<Vec<noflex_storage::domain::ProviderAttribution>> {
        let (_schema, conn) = self.reader().await?;
        repo::provider_attributions::list(&conn).await
    }

    async fn provider_attribution_logo(
        &self,
        provider: &str,
    ) -> StorageResult<Option<noflex_storage::domain::AttributionLogo>> {
        let (_schema, conn) = self.reader().await?;
        repo::provider_attributions::logo(&conn, provider).await
    }

    // ---- Images --------------------------------------------------------
    async fn upsert_image(&self, image: &Image) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::images::upsert(
            conn, image
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::images::delete(
            conn, target, kind, source
        ))
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::search::upsert_embedding(conn, target, namespace, model_id, vector)
        })
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

        Ok(noflex_storage::search::fuse_hybrid(
            lexical,
            vector,
            query.access_user_id.is_some(),
            query.limit,
        ))
    }

    // ---- Export jobs ---------------------------------------------------
    async fn create_export_job(&self, job: &ExportJob) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::export_jobs::create(
            conn, job
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::export_jobs::claim(
            conn,
            worker_id,
            lease_token,
            now,
            lease_expires_at,
            max_attempts,
        ))
    }
    async fn update_export_job_progress(
        &self,
        id: ExportJobId,
        lease_token: &str,
        progress: f32,
        lease_expires_at: OffsetDateTime,
    ) -> StorageResult<Option<ExportJobStatus>> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::export_jobs::update_progress(conn, id, lease_token, progress, lease_expires_at)
        })
    }
    async fn complete_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
        output_path: &str,
    ) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::export_jobs::complete(
            conn,
            id,
            lease_token,
            output_path
        ))
    }
    async fn fail_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
        error: &str,
    ) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| repo::export_jobs::fail(
            conn,
            id,
            lease_token,
            error
        ))
    }
    async fn cancel_claimed_export_job(
        &self,
        id: ExportJobId,
        lease_token: &str,
    ) -> StorageResult<bool> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::export_jobs::cancel_claimed(conn, id, lease_token)
        })
    }
    async fn request_export_job_cancellation(
        &self,
        id: ExportJobId,
        owner: UserId,
    ) -> StorageResult<Option<ExportJobCancellation>> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::export_jobs::request_cancellation(conn, id, owner)
        })
    }
    async fn delete_export_job(&self, id: ExportJobId) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::export_jobs::delete(
            conn, id
        ))
    }
    async fn finished_export_job(
        &self,
        edition: noflex_core::EditionId,
        format: noflex_storage::domain::ExportFormat,
    ) -> StorageResult<Option<ExportJob>> {
        let (_schema, conn) = self.reader().await?;
        repo::export_jobs::finished(&conn, edition, format).await
    }
    async fn user_export_job(
        &self,
        user: UserId,
        edition: noflex_core::EditionId,
        format: noflex_storage::domain::ExportFormat,
    ) -> StorageResult<Option<ExportJob>> {
        let (_schema, conn) = self.reader().await?;
        repo::export_jobs::for_user(&conn, user, edition, format).await
    }
    async fn export_output_in_use(&self, output_path: &str) -> StorageResult<bool> {
        let (_schema, conn) = self.reader().await?;
        repo::export_jobs::output_in_use(&conn, output_path).await
    }

    // ---- Collections ---------------------------------------------------
    async fn create_collection(&self, collection: &Collection) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::collections::create(
            conn, collection
        ))
    }
    async fn update_collection(&self, collection: &Collection) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::collections::update(
            conn, collection
        ))
    }
    async fn delete_collection(&self, id: CollectionId) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::collections::delete(
            conn, id
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::collections::set_items(
            conn, id, items
        ))
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
        transact!(self, Lane::Concurrent, |conn| repo::playlists::create(
            conn, playlist
        ))
    }
    async fn update_playlist(&self, playlist: &Playlist) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::playlists::update(
            conn, playlist
        ))
    }
    async fn delete_playlist(&self, id: PlaylistId) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::playlists::delete(conn, id))
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
        transact!(self, Lane::Concurrent, |conn| repo::playlists::set_items(
            conn, id, items
        ))
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
        transact!(
            self,
            Lane::Concurrent,
            |conn| repo::playlists::append_items(conn, id, items)
        )
    }

    async fn upsert_live_tv_source(&self, source: &LiveTvSource) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| repo::live_tv::upsert_source(
            conn, source
        ))
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
        transact!(self, Lane::Serial, |conn| repo::live_tv::delete_source(
            conn, id
        ))
    }
    async fn apply_live_tv_refresh(
        &self,
        source: LiveTvSourceId,
        channels: &[LiveTvChannel],
        programs: &[EpgProgram],
        refreshed_at: OffsetDateTime,
    ) -> StorageResult<()> {
        transact!(self, Lane::Serial, |conn| repo::live_tv::apply_refresh(
            conn,
            source,
            channels,
            programs,
            refreshed_at
        ))
    }
    async fn record_live_tv_refresh_error(
        &self,
        source: LiveTvSourceId,
        error: &str,
        attempted_at: OffsetDateTime,
    ) -> StorageResult<()> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::live_tv::record_refresh_error(conn, source, error, attempted_at)
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::live_tv::create_schedule(conn, s)
        })
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::live_tv::cancel_schedule(conn, id, owner, now)
        })
    }
    async fn claim_due_recording(
        &self,
        now: OffsetDateTime,
    ) -> StorageResult<Option<(RecordingSchedule, Recording)>> {
        transact!(self, Lane::Concurrent, |conn| repo::live_tv::claim_due(
            conn, now
        ))
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
        transact!(self, Lane::Concurrent, |conn| {
            repo::live_tv::update_recording(conn, id, status, path, error, size, at)
        })
    }
    async fn delete_recording(
        &self,
        id: RecordingId,
        owner: UserId,
    ) -> StorageResult<Option<Recording>> {
        transact!(self, Lane::Concurrent, |conn| {
            repo::live_tv::delete_recording(conn, id, owner)
        })
    }
    async fn fail_running_recordings(&self, at: OffsetDateTime, error: &str) -> StorageResult<u64> {
        transact!(self, Lane::Concurrent, |conn| repo::live_tv::fail_running(
            conn, at, error
        ))
    }
}

#[cfg(test)]
mod concurrency_tests {
    use std::{sync::Arc, time::Duration};

    use noflex_core::{InviteId, LibraryId, UserId, UserRole};
    use noflex_storage::{
        Storage,
        domain::{Invite, User},
    };
    use time::OffsetDateTime;
    use tokio::time::timeout;
    use turso::Value;

    use super::{TursoOptions, TursoStore, transaction::Lane};

    async fn store() -> TursoStore {
        TursoStore::open_with_options(
            ":memory:",
            TursoOptions {
                reader_connections: 2,
                writer_connections: 4,
                busy_timeout: Duration::from_secs(5),
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

    async fn execute(store: &TursoStore, sql: &str) {
        let conn = store.writers.acquire().await.unwrap();
        conn.execute(sql, ()).await.unwrap();
    }

    #[tokio::test]
    async fn readers_are_distinct_connections_and_share_memory_database() {
        let store = store().await;
        execute(&store, "CREATE TABLE shared(value INTEGER)").await;
        execute(&store, "INSERT INTO shared VALUES (7)").await;
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
    async fn readers_never_observe_an_uncommitted_write() {
        let store = store().await;
        execute(&store, "CREATE TABLE values_under_test(value INTEGER)").await;
        execute(&store, "INSERT INTO values_under_test VALUES (0)").await;

        let tx = super::transaction::begin(&store.writers, &store.gates, Lane::Concurrent)
            .await
            .unwrap();
        tx.connection()
            .execute("UPDATE values_under_test SET value = 1", ())
            .await
            .unwrap();
        let (_schema, reader) = store.reader().await.unwrap();
        assert_eq!(
            scalar(&reader, "SELECT value FROM values_under_test").await,
            0
        );
        drop(reader);
        tx.finish(Ok(())).await.unwrap();

        let (_schema, reader) = store.reader().await.unwrap();
        assert_eq!(
            scalar(&reader, "SELECT value FROM values_under_test").await,
            1
        );
    }

    #[tokio::test]
    async fn concurrent_transactions_are_open_and_commit_together() {
        let store = store().await;
        execute(&store, "CREATE TABLE lanes(id INTEGER PRIMARY KEY)").await;
        let begin = || super::transaction::begin(&store.writers, &store.gates, Lane::Concurrent);
        let first = timeout(Duration::from_secs(1), begin())
            .await
            .unwrap()
            .unwrap();
        let second = timeout(Duration::from_secs(1), begin())
            .await
            .expect("a second concurrent writer waited on the first")
            .unwrap();
        for (tx, id) in [(&first, 1), (&second, 2)] {
            tx.connection()
                .execute("INSERT INTO lanes VALUES (?1)", [id])
                .await
                .unwrap();
        }
        first.finish(Ok(())).await.unwrap();
        second.finish(Ok(())).await.unwrap();
        let (_schema, reader) = store.reader().await.unwrap();
        assert_eq!(scalar(&reader, "SELECT count(*) FROM lanes").await, 2);
    }

    /// Every use counted means no conflicting increment was lost: each one
    /// that lost a conflict ran again on the winner's count.
    #[tokio::test]
    async fn conflicting_writes_are_retried_until_every_one_lands() {
        let store = Arc::new(store().await);
        store.migrate().await.unwrap();
        let now = OffsetDateTime::now_utc();
        let admin = User {
            id: UserId::new(),
            username: "admin".to_string(),
            display_name: None,
            password_hash: "argon2id$placeholder".to_string(),
            is_admin: true,
            role: UserRole::Admin,
            created_at: now,
            avatar_updated_at: None,
            has_pin: false,
        };
        store.upsert_user(&admin).await.unwrap();
        let invite = Invite {
            id: InviteId::new(),
            created_by: admin.id,
            library_ids: vec![LibraryId::new()],
            max_uses: 100,
            uses: 0,
            expires_at: now + time::Duration::days(1),
            revoked_at: None,
            created_at: now,
        };
        store.create_invite(&invite, "contended").await.unwrap();

        let spends = (0..32).map(|_| {
            let store = store.clone();
            tokio::spawn(async move { store.consume_invite("contended", now).await })
        });
        for spend in spends {
            spend.await.unwrap().unwrap().expect("invite has uses left");
        }
        let invites = store.list_invites().await.unwrap();
        assert_eq!(invites[0].uses, 32);
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

    /// An abandoned exclusive transaction would leave every later one busy.
    #[tokio::test]
    async fn connection_left_mid_transaction_is_rolled_back_before_reuse() {
        let store = store().await;
        execute(&store, "CREATE TABLE leaked(value INTEGER)").await;
        {
            let conn = store.writers.acquire().await.unwrap();
            conn.execute("BEGIN IMMEDIATE", ()).await.unwrap();
            conn.execute("INSERT INTO leaked VALUES (1)", ())
                .await
                .unwrap();
        }
        let transaction = timeout(
            Duration::from_secs(1),
            super::transaction::begin(&store.writers, &store.gates, Lane::Serial),
        )
        .await
        .expect("transaction after recovery")
        .unwrap();
        assert_eq!(
            scalar(transaction.connection(), "SELECT count(*) FROM leaked").await,
            0
        );
        transaction.finish(Ok(())).await.unwrap();
    }

    #[tokio::test]
    async fn serial_transactions_exclude_every_other_writer() {
        let store = store().await;
        let first = store.begin().await.unwrap();
        assert!(
            timeout(Duration::from_millis(25), store.begin())
                .await
                .is_err()
        );
        assert!(
            timeout(
                Duration::from_millis(25),
                super::transaction::begin(&store.writers, &store.gates, Lane::Concurrent)
            )
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
