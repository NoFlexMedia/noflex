// SPDX-License-Identifier: AGPL-3.0-only
//
// Apply migration scripts in lexical order. Each script is identified by its
// filename; the filename is recorded in `schema_meta` so that subsequent
// `migrate()` calls are idempotent.

use noflex_storage::error::StorageResult;
use turso::Connection;

use crate::error::map_turso;

/// `(filename, sql)` pairs in lexical order. 0001 is the schema squashed
/// before the first release; add new migrations after it — never edit a
/// migration that has shipped.
pub(crate) const MIGRATIONS: &[(&str, &str)] =
    &[("0001_init.sql", include_str!("../migrations/0001_init.sql"))];

const META_KEY_PREFIX: &str = "migration:";

pub(crate) async fn apply_all(conn: &Connection) -> StorageResult<u32> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_meta (\
            key   TEXT PRIMARY KEY,\
            value TEXT NOT NULL\
        )",
        (),
    )
    .await
    .map_err(map_turso)?;

    let mut applied = 0u32;
    for (name, sql) in MIGRATIONS {
        if is_applied(conn, name).await? {
            continue;
        }
        let tx = turso::transaction::Transaction::new_unchecked(
            conn,
            turso::transaction::TransactionBehavior::Immediate,
        )
        .await
        .map_err(map_turso)?;
        let result = async {
            conn.execute_batch(*sql).await.map_err(map_turso)?;
            record_applied(conn, name).await
        }
        .await;
        match result {
            Ok(()) => tx.commit().await.map_err(map_turso)?,
            Err(error) => {
                let _ = tx.rollback().await;
                return Err(error);
            }
        }
        applied += 1;
    }
    Ok(applied)
}

pub(crate) async fn schema_version(conn: &Connection) -> StorageResult<u32> {
    let mut count = 0u32;
    for (name, _) in MIGRATIONS {
        if is_applied(conn, name).await? {
            count += 1;
        }
    }
    Ok(count)
}

async fn is_applied(conn: &Connection, name: &str) -> StorageResult<bool> {
    let key = format!("{META_KEY_PREFIX}{name}");
    let mut rows = conn
        .query("SELECT 1 FROM schema_meta WHERE key = ?1", vec![key])
        .await
        .map_err(map_turso)?;
    Ok(rows.next().await.map_err(map_turso)?.is_some())
}

async fn record_applied(conn: &Connection, name: &str) -> StorageResult<()> {
    let key = format!("{META_KEY_PREFIX}{name}");
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES (?1, ?2)",
        vec![
            turso::Value::Text(key),
            turso::Value::Text("ok".to_string()),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}
