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

pub(crate) struct ReaderPool {
    inner: Arc<ReaderPoolInner>,
}

struct ReaderPoolInner {
    available: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}

impl ReaderPool {
    pub(crate) async fn new(
        db: &Database,
        capacity: usize,
        busy_timeout: Duration,
    ) -> StorageResult<Self> {
        if capacity == 0 {
            return Err(backend_msg(
                "reader connection count must be greater than zero",
            ));
        }

        let mut available = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            available.push(connection::connect(db, busy_timeout).await?);
        }
        Ok(Self {
            inner: Arc::new(ReaderPoolInner {
                available: Mutex::new(available),
                permits: Arc::new(Semaphore::new(capacity)),
            }),
        })
    }

    pub(crate) async fn acquire(&self) -> StorageResult<ReaderLease> {
        let permit = self
            .inner
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| backend_msg("reader pool is closed"))?;
        let conn = self
            .inner
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .ok_or_else(|| backend_msg("reader pool permit had no connection"))?;
        Ok(ReaderLease {
            conn: ManuallyDrop::new(conn),
            inner: self.inner.clone(),
            _permit: permit,
        })
    }
}

pub(crate) struct ReaderLease {
    conn: ManuallyDrop<Connection>,
    inner: Arc<ReaderPoolInner>,
    _permit: OwnedSemaphorePermit,
}

impl Deref for ReaderLease {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

impl Drop for ReaderLease {
    fn drop(&mut self) {
        // SAFETY: `conn` is never touched again after this take; the
        // lease is being dropped and has no other access path.
        let conn = unsafe { ManuallyDrop::take(&mut self.conn) };
        self.inner
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(conn);
    }
}
