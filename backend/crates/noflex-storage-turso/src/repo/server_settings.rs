// SPDX-License-Identifier: AGPL-3.0-only

use noflex_storage::error::StorageResult;
use turso::{Connection, Value};

use crate::{convert::col_int, error::map_turso};

pub(crate) async fn downloads_allowed(conn: &Connection) -> StorageResult<bool> {
    let mut rows = conn
        .query("SELECT downloads_allowed FROM server_settings", ())
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(true);
    };
    Ok(col_int(&row.get_value(0).map_err(map_turso)?)? != 0)
}

pub(crate) async fn set_downloads_allowed(conn: &Connection, allowed: bool) -> StorageResult<()> {
    conn.execute(
        "INSERT INTO server_settings (id, downloads_allowed) VALUES (1, ?1) \
         ON CONFLICT(id) DO UPDATE SET downloads_allowed = excluded.downloads_allowed",
        vec![Value::Integer(i64::from(allowed))],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}
