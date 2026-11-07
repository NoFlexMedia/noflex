// SPDX-License-Identifier: AGPL-3.0-only

use std::time::Duration;

use noflex_storage::error::StorageResult;
use turso::{Builder, Connection, Database, Value};

use crate::error::{backend_msg, map_turso};

/// Open `path` in MVCC mode, which `BEGIN CONCURRENT` needs. Index methods
/// back the Tantivy full-text indexes the migrations create.
pub(crate) async fn build_database(path: &str) -> StorageResult<Database> {
    let db = Builder::new_local(path)
        .experimental_vacuum(true)
        .experimental_index_method(true)
        .build()
        .await
        .map_err(map_turso)?;
    // The mode lives in the database file, so connections opened after this
    // one see it too.
    let conn = db.connect().map_err(map_turso)?;
    let mut rows = conn
        .query("PRAGMA journal_mode = 'mvcc'", ())
        .await
        .map_err(map_turso)?;
    let mode = rows.next().await.map_err(map_turso)?;
    match mode.as_ref().map(|row| row.get_value(0)) {
        Some(Ok(Value::Text(mode))) if mode == "mvcc" => Ok(db),
        other => Err(backend_msg(format!(
            "database did not switch to MVCC: {other:?}"
        ))),
    }
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
