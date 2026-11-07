// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use noflex_core::{EditionId, TagId, TagTarget};
use noflex_storage::{
    domain::{Edition, Image, MediaFile, MediaRemoval, Session, Tag, User, Work},
    error::StorageResult,
    store::Transaction,
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use turso::Connection;

use crate::{
    error::map_turso,
    pool::{Lease, Pool},
};

/// First pause before retrying a write that lost a conflict; it doubles up to
/// [`MAX_RETRY_DELAY`].
const FIRST_RETRY_DELAY: Duration = Duration::from_micros(500);
const MAX_RETRY_DELAY: Duration = Duration::from_millis(50);

/// How a write transaction shares the database with other writers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    /// `BEGIN CONCURRENT` beside every other concurrent writer. When two
    /// write the same row the later commit fails and is retried, so this
    /// suits writes keyed to the rows they change.
    Concurrent,
    /// Alone: no other write transaction is open while it runs. Snapshot
    /// isolation stops neither a concurrent insert referencing a row this
    /// deletes nor two writers each acting on a read the other invalidates,
    /// so deletes of referenced rows and checks spanning rows go here.
    Serial,
}

/// Locks every write transaction holds: `schema` shared, as all queries do,
/// and `gate` shared or exclusive by [`Lane`].
pub(crate) struct Gates {
    pub(crate) schema: Arc<RwLock<()>>,
    pub(crate) gate: Arc<RwLock<()>>,
}

pub(crate) async fn begin(
    writers: &Pool,
    gates: &Gates,
    lane: Lane,
) -> StorageResult<TursoTransaction> {
    let schema = gates.schema.clone().read_owned().await;
    let (shared, exclusive, sql) = match lane {
        Lane::Concurrent => (
            Some(gates.gate.clone().read_owned().await),
            None,
            "BEGIN CONCURRENT",
        ),
        Lane::Serial => (
            None,
            Some(gates.gate.clone().write_owned().await),
            "BEGIN IMMEDIATE",
        ),
    };
    let lease = writers.acquire().await?;
    lease.execute(sql, ()).await.map_err(map_turso)?;
    Ok(TursoTransaction {
        lease,
        _shared: shared,
        _exclusive: exclusive,
        _schema: schema,
    })
}

/// One open write transaction on a pooled connection. Dropping it without
/// [`finish`](Self::finish), `commit` or `rollback`, including by
/// cancellation, rolls it back as the connection returns to the pool.
pub(crate) struct TursoTransaction {
    lease: Lease,
    _shared: Option<OwnedRwLockReadGuard<()>>,
    _exclusive: Option<OwnedRwLockWriteGuard<()>>,
    _schema: OwnedRwLockReadGuard<()>,
}

impl TursoTransaction {
    pub(crate) fn connection(&self) -> &Connection {
        &self.lease
    }

    /// Commits on `Ok` and rolls back on `Err`, returning the first error.
    pub(crate) async fn finish<T>(self, outcome: StorageResult<T>) -> StorageResult<T> {
        match outcome {
            Ok(value) => match self.lease.execute("COMMIT", ()).await {
                Ok(_) => Ok(value),
                Err(error) => {
                    self.rollback_if_open().await;
                    Err(map_turso(error))
                }
            },
            Err(error) => {
                self.rollback_if_open().await;
                Err(error)
            }
        }
    }

    /// A conflict ends the transaction inside the engine, so only roll back
    /// one that's still open.
    async fn rollback_if_open(&self) {
        if self.lease.is_autocommit().unwrap_or(false) {
            return;
        }
        if let Err(error) = self.lease.execute("ROLLBACK", ()).await {
            // The pool retries the reset before anyone reuses the connection.
            tracing::warn!(%error, "write transaction rollback failed");
        }
    }
}

/// Paces retries of a write that lost a conflict, giving up after the busy
/// timeout as a lock wait would.
pub(crate) struct Retry {
    deadline: Instant,
    delay: Duration,
}

impl Retry {
    pub(crate) fn new(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            delay: FIRST_RETRY_DELAY,
        }
    }

    /// Sleeps before the next attempt; `false`, at once, past the deadline.
    pub(crate) async fn wait(&mut self) -> bool {
        let Some(left) = self.deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        tokio::time::sleep(self.delay.min(left)).await;
        self.delay = (self.delay * 2).min(MAX_RETRY_DELAY);
        true
    }
}

#[async_trait]
impl Transaction for TursoTransaction {
    async fn commit(self: Box<Self>) -> StorageResult<()> {
        self.finish(Ok(())).await
    }

    async fn rollback(self: Box<Self>) -> StorageResult<()> {
        if self.lease.is_autocommit().unwrap_or(false) {
            return Ok(());
        }
        self.lease
            .execute("ROLLBACK", ())
            .await
            .map_err(map_turso)?;
        Ok(())
    }

    async fn upsert_work(&mut self, work: &Work) -> StorageResult<()> {
        crate::repo::works::upsert(self.connection(), work).await
    }

    async fn upsert_edition(&mut self, edition: &Edition) -> StorageResult<EditionId> {
        crate::repo::editions::upsert(self.connection(), edition).await
    }

    async fn upsert_media_file(&mut self, media: &MediaFile) -> StorageResult<()> {
        crate::repo::media_files::upsert(self.connection(), media).await
    }

    async fn prune_edition(&mut self, edition: EditionId) -> StorageResult<MediaRemoval> {
        crate::repo::media_files::prune_edition(self.connection(), edition).await
    }

    async fn upsert_tag(&mut self, tag: &Tag) -> StorageResult<TagId> {
        crate::repo::tags::upsert(self.connection(), tag).await
    }

    async fn assign_tag(
        &mut self,
        tag: TagId,
        target: TagTarget,
        weight: f32,
        source: &str,
    ) -> StorageResult<()> {
        crate::repo::tags::assign(
            self.connection(),
            tag,
            target,
            weight,
            source,
            &serde_json::json!({}),
        )
        .await
    }

    async fn upsert_user(&mut self, user: &User) -> StorageResult<()> {
        crate::repo::users::upsert(self.connection(), user).await
    }

    async fn create_session(&mut self, session: &Session) -> StorageResult<()> {
        crate::repo::sessions::create(self.connection(), session).await
    }

    async fn upsert_image(&mut self, image: &Image) -> StorageResult<()> {
        crate::repo::images::upsert(self.connection(), image).await
    }
}
