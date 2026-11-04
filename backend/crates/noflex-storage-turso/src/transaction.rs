// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_trait::async_trait;
use noflex_core::{EditionId, TagId, TagTarget};
use noflex_storage::{
    domain::{Edition, Image, MediaFile, MediaRemoval, Session, Tag, User, Work},
    error::StorageResult,
    store::Transaction,
};
use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use turso::Connection;

use crate::{
    error::map_turso,
    writer::{SharedWriter, WriterLease},
};

pub(crate) async fn begin(
    writer: SharedWriter,
    schema: Arc<RwLock<()>>,
) -> StorageResult<TursoTransaction> {
    let schema_guard = schema.read_owned().await;
    let lease = WriterLease::acquire(&writer).await?;
    lease
        .execute("BEGIN IMMEDIATE", ())
        .await
        .map_err(map_turso)?;
    Ok(TursoTransaction {
        lease,
        _schema_guard: schema_guard,
    })
}

/// Holds the writer for the life of one `BEGIN IMMEDIATE` transaction.
/// Dropping it without `commit`/`rollback` (including future
/// cancellation) releases the writer mid-transaction; the next
/// [`WriterLease::acquire`] rolls that back before reuse.
pub(crate) struct TursoTransaction {
    lease: WriterLease,
    _schema_guard: OwnedRwLockReadGuard<()>,
}

impl TursoTransaction {
    pub(crate) fn connection(&self) -> &Connection {
        &self.lease
    }

    fn conn(&self) -> &Connection {
        &self.lease
    }

    async fn finalize(mut self: Box<Self>, commit: bool) -> StorageResult<()> {
        let sql = if commit { "COMMIT" } else { "ROLLBACK" };
        let result = self.lease.execute(sql, ()).await.map_err(map_turso);
        if let Err(error) = self.lease.ensure_idle().await {
            // The next acquirer retries the reset; never block on it here.
            tracing::error!(%error, "writer reset after transaction end failed");
        }
        result.map(|_| ())
    }
}

#[async_trait]
impl Transaction for TursoTransaction {
    async fn commit(self: Box<Self>) -> StorageResult<()> {
        self.finalize(true).await
    }

    async fn rollback(self: Box<Self>) -> StorageResult<()> {
        self.finalize(false).await
    }

    async fn upsert_work(&mut self, work: &Work) -> StorageResult<()> {
        crate::repo::works::upsert(self.conn(), work).await
    }

    async fn upsert_edition(&mut self, edition: &Edition) -> StorageResult<EditionId> {
        crate::repo::editions::upsert(self.conn(), edition).await
    }

    async fn upsert_media_file(&mut self, media: &MediaFile) -> StorageResult<()> {
        crate::repo::media_files::upsert(self.conn(), media).await
    }

    async fn prune_edition(&mut self, edition: EditionId) -> StorageResult<MediaRemoval> {
        crate::repo::media_files::prune_edition(self.conn(), edition).await
    }

    async fn upsert_tag(&mut self, tag: &Tag) -> StorageResult<TagId> {
        crate::repo::tags::upsert(self.conn(), tag).await
    }

    async fn assign_tag(
        &mut self,
        tag: TagId,
        target: TagTarget,
        weight: f32,
        source: &str,
    ) -> StorageResult<()> {
        crate::repo::tags::assign(
            self.conn(),
            tag,
            target,
            weight,
            source,
            &serde_json::json!({}),
        )
        .await
    }

    async fn upsert_user(&mut self, user: &User) -> StorageResult<()> {
        crate::repo::users::upsert(self.conn(), user).await
    }

    async fn create_session(&mut self, session: &Session) -> StorageResult<()> {
        crate::repo::sessions::create(self.conn(), session).await
    }

    async fn upsert_image(&mut self, image: &Image) -> StorageResult<()> {
        crate::repo::images::upsert(self.conn(), image).await
    }
}
