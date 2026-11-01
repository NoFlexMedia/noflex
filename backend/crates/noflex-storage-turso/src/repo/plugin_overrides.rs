// SPDX-License-Identifier: AGPL-3.0-only

use noflex_storage::{domain::PluginOverrideRow, error::StorageResult};
use turso::{Connection, Value};

use crate::{
    convert::{col_int, col_text},
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO plugin_overrides (plugin_id, hook, work_kind, priority) \
    VALUES (?1, ?2, ?3, ?4) \
    ON CONFLICT(plugin_id, hook, work_kind) DO UPDATE SET \
        priority = excluded.priority";

const LIST: &str = "SELECT plugin_id, hook, work_kind, priority FROM plugin_overrides \
    ORDER BY plugin_id, hook, work_kind";

const DELETE: &str =
    "DELETE FROM plugin_overrides WHERE plugin_id = ?1 AND hook = ?2 AND work_kind = ?3";

pub(crate) async fn upsert(
    conn: &Connection,
    plugin_id: &str,
    hook: &str,
    work_kind: &str,
    priority: i32,
) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Text(plugin_id.to_string()),
        Value::Text(hook.to_string()),
        Value::Text(work_kind.to_string()),
        Value::Integer(priority as i64),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn list(conn: &Connection) -> StorageResult<Vec<PluginOverrideRow>> {
    let mut rows = conn.query(LIST, ()).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let plugin_id = row.get_value(0).map_err(map_turso)?;
        let hook = row.get_value(1).map_err(map_turso)?;
        let work_kind = row.get_value(2).map_err(map_turso)?;
        let priority = row.get_value(3).map_err(map_turso)?;
        out.push(PluginOverrideRow {
            plugin_id: col_text(&plugin_id)?.to_string(),
            hook: col_text(&hook)?.to_string(),
            work_kind: col_text(&work_kind)?.to_string(),
            priority: i32::try_from(col_int(&priority)?).unwrap_or(i32::MAX),
        });
    }
    Ok(out)
}

pub(crate) async fn delete(
    conn: &Connection,
    plugin_id: &str,
    hook: &str,
    work_kind: &str,
) -> StorageResult<bool> {
    let params: Vec<Value> = vec![
        Value::Text(plugin_id.to_string()),
        Value::Text(hook.to_string()),
        Value::Text(work_kind.to_string()),
    ];
    let changed = conn.execute(DELETE, params).await.map_err(map_turso)?;
    Ok(changed > 0)
}
