// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    mem::ManuallyDrop,
    ops::Deref,
    sync::{Arc, Mutex},
    time::Duration,
};

use noflex_storage::error::StorageResult;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use turso::{Connection, Database};

use crate::{connection, error::backend_msg};

/// At most `capacity` connections to one database, opened on first use.
/// Every lease starts in autocommit mode.
pub(crate) struct Pool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    db: Database,
    busy_timeout: Duration,
    idle: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}

impl Pool {
    /// # Errors
    ///
    /// A `capacity` of zero, since every acquire would wait forever.
    pub(crate) fn new(
        db: Database,
        capacity: usize,
        busy_timeout: Duration,
    ) -> StorageResult<Self> {
        if capacity == 0 {
            return Err(backend_msg("connection count must be greater than zero"));
        }
        Ok(Self {
            inner: Arc::new(PoolInner {
                db,
                busy_timeout,
                idle: Mutex::new(Vec::with_capacity(capacity)),
                permits: Arc::new(Semaphore::new(capacity)),
            }),
        })
    }

    /// Waits for a free slot, then hands out an idle connection or opens one.
    pub(crate) async fn acquire(&self) -> StorageResult<Lease> {
        let permit = self
            .inner
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| backend_msg("connection pool is closed"))?;
        let idle = self.inner.take_idle();
        let conn = match idle {
            Some(conn) => self.inner.reset(conn).await?,
            None => self.inner.open().await?,
        };
        Ok(Lease {
            conn: ManuallyDrop::new(conn),
            inner: self.inner.clone(),
            permit: Some(permit),
        })
    }
}

impl PoolInner {
    fn take_idle(&self) -> Option<Connection> {
        self.idle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
    }

    fn put_idle(&self, conn: Connection) {
        self.idle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(conn);
    }

    async fn open(&self) -> StorageResult<Connection> {
        connection::connect(&self.db, self.busy_timeout).await
    }

    /// Returns `conn` to autocommit mode. One whose ROLLBACK can't close
    /// the open transaction is dropped, which releases what it holds, and
    /// replaced.
    async fn reset(&self, conn: Connection) -> StorageResult<Connection> {
        if conn.is_autocommit().unwrap_or(false) {
            return Ok(conn);
        }
        if let Err(error) = conn.execute("ROLLBACK", ()).await {
            tracing::warn!(%error, "pooled connection rollback failed");
        }
        if conn.is_autocommit().unwrap_or(false) {
            return Ok(conn);
        }
        tracing::error!("replacing connection stuck inside a transaction");
        drop(conn);
        self.open().await
    }
}

/// One pooled connection, returned on drop.
pub(crate) struct Lease {
    conn: ManuallyDrop<Connection>,
    inner: Arc<PoolInner>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Deref for Lease {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        // SAFETY: `conn` is never touched again after this take; the
        // lease is being dropped and has no other access path.
        let conn = unsafe { ManuallyDrop::take(&mut self.conn) };
        let permit = self.permit.take();
        if conn.is_autocommit().unwrap_or(false) {
            self.inner.put_idle(conn);
            return;
        }
        // A cancelled transaction. Under MVCC an abandoned exclusive one fails
        // every other commit until it ends, so end it now rather than whenever
        // this connection is next picked; the slot stays taken until then.
        let inner = self.inner.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    match inner.reset(conn).await {
                        Ok(conn) => inner.put_idle(conn),
                        Err(error) => tracing::error!(%error, "pooled connection reset failed"),
                    }
                    drop(permit);
                });
            }
            Err(_) => self.inner.put_idle(conn),
        }
    }
}
