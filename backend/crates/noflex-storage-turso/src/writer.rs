// SPDX-License-Identifier: AGPL-3.0-only

use std::{ops::Deref, sync::Arc, time::Duration};

use noflex_storage::error::StorageResult;
use tokio::sync::{Mutex, OwnedMutexGuard};
use turso::{Connection, Database};

use crate::connection;

/// The single writer connection plus what is needed to replace it.
pub(crate) struct WriterSlot {
    db: Database,
    busy_timeout: Duration,
    conn: Connection,
}

pub(crate) type SharedWriter = Arc<Mutex<WriterSlot>>;

impl WriterSlot {
    pub(crate) async fn new(db: Database, busy_timeout: Duration) -> StorageResult<SharedWriter> {
        let conn = connection::connect(&db, busy_timeout).await?;
        Ok(Arc::new(Mutex::new(Self {
            db,
            busy_timeout,
            conn,
        })))
    }

    /// Restores autocommit mode. A connection whose ROLLBACK cannot close
    /// the open transaction is discarded — dropping it releases its engine
    /// locks — and replaced, so one failed rollback never wedges writers.
    pub(crate) async fn ensure_idle(&mut self) -> StorageResult<()> {
        if self.conn.is_autocommit().unwrap_or(false) {
            return Ok(());
        }
        if let Err(error) = self.conn.execute("ROLLBACK", ()).await {
            tracing::warn!(%error, "writer rollback failed");
        }
        if self.conn.is_autocommit().unwrap_or(false) {
            return Ok(());
        }
        tracing::error!("replacing writer connection stuck inside a transaction");
        self.conn = connection::connect(&self.db, self.busy_timeout).await?;
        Ok(())
    }
}

/// Exclusive use of the writer connection, guaranteed to start in
/// autocommit mode.
pub(crate) struct WriterLease {
    guard: OwnedMutexGuard<WriterSlot>,
}

impl WriterLease {
    pub(crate) async fn acquire(writer: &SharedWriter) -> StorageResult<Self> {
        let mut guard = writer.clone().lock_owned().await;
        guard.ensure_idle().await?;
        Ok(Self { guard })
    }

    pub(crate) async fn ensure_idle(&mut self) -> StorageResult<()> {
        self.guard.ensure_idle().await
    }
}

impl Deref for WriterLease {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.guard.conn
    }
}
