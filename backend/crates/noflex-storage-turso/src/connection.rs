// SPDX-License-Identifier: AGPL-3.0-only

use std::time::Duration;

use noflex_storage::error::StorageResult;
use turso::{Builder, Connection, Database};

use crate::error::map_turso;

pub(crate) async fn build_database(path: &str) -> StorageResult<Database> {
    Builder::new_local(path)
        .experimental_vacuum(true)
        .build()
        .await
        .map_err(map_turso)
}

/// Open a fresh connection and apply the per-connection pragmas the rest of
/// the backend assumes.
pub(crate) async fn connect(db: &Database, busy_timeout: Duration) -> StorageResult<Connection> {
    let conn = db.connect().map_err(map_turso)?;
    conn.execute("PRAGMA foreign_keys = ON", ())
        .await
        .map_err(map_turso)?;
    conn.busy_timeout(busy_timeout).map_err(map_turso)?;
    Ok(conn)
}
