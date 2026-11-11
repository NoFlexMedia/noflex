// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::BTreeMap;

use noflex_storage::{domain::PluginSettingWrite, error::StorageResult};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{col_text, dt_to_text},
    error::map_turso,
};

const LIST: &str = "SELECT key, value FROM plugin_settings WHERE plugin_id = ?1 ORDER BY key";

const SECRET_KEYS: &str =
    "SELECT key FROM plugin_secret_settings WHERE plugin_id = ?1 ORDER BY key";

const SECRET: &str = "SELECT value FROM plugin_secret_settings WHERE plugin_id = ?1 AND key = ?2";

const UPSERT: &str = "INSERT INTO plugin_settings (plugin_id, key, value, updated_at) \
    VALUES (?1, ?2, ?3, ?4) \
    ON CONFLICT(plugin_id, key) DO UPDATE SET \
        value = excluded.value, updated_at = excluded.updated_at";

const UPSERT_SECRET: &str = "INSERT INTO plugin_secret_settings (plugin_id, key, value, updated_at) \
    VALUES (?1, ?2, ?3, ?4) \
    ON CONFLICT(plugin_id, key) DO UPDATE SET \
        value = excluded.value, updated_at = excluded.updated_at";

const DELETE: &str = "DELETE FROM plugin_settings WHERE plugin_id = ?1 AND key = ?2";

const DELETE_SECRET: &str = "DELETE FROM plugin_secret_settings WHERE plugin_id = ?1 AND key = ?2";

pub(crate) async fn list(
    conn: &Connection,
    plugin_id: &str,
) -> StorageResult<BTreeMap<String, String>> {
    let mut rows = conn
        .query(LIST, vec![Value::Text(plugin_id.to_string())])
        .await
        .map_err(map_turso)?;
    let mut out = BTreeMap::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let key = row.get_value(0).map_err(map_turso)?;
        let value = row.get_value(1).map_err(map_turso)?;
        out.insert(col_text(&key)?.to_string(), col_text(&value)?.to_string());
    }
    Ok(out)
}

pub(crate) async fn secret_keys(conn: &Connection, plugin_id: &str) -> StorageResult<Vec<String>> {
    let mut rows = conn
        .query(SECRET_KEYS, vec![Value::Text(plugin_id.to_string())])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let key = row.get_value(0).map_err(map_turso)?;
        out.push(col_text(&key)?.to_string());
    }
    Ok(out)
}

pub(crate) async fn secret(
    conn: &Connection,
    plugin_id: &str,
    key: &str,
) -> StorageResult<Option<String>> {
    let params = vec![
        Value::Text(plugin_id.to_string()),
        Value::Text(key.to_string()),
    ];
    let mut rows = conn.query(SECRET, params).await.map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => {
            let value = row.get_value(0).map_err(map_turso)?;
            Ok(Some(col_text(&value)?.to_string()))
        }
        None => Ok(None),
    }
}

pub(crate) async fn update(
    conn: &Connection,
    plugin_id: &str,
    writes: &[PluginSettingWrite],
) -> StorageResult<()> {
    let now = dt_to_text(OffsetDateTime::now_utc())?;
    for write in writes {
        let key = Value::Text(write.key.clone());
        let plugin = Value::Text(plugin_id.to_string());
        match &write.value {
            Some(value) => {
                let sql = if write.secret { UPSERT_SECRET } else { UPSERT };
                let params = vec![
                    plugin,
                    key,
                    Value::Text(value.clone()),
                    Value::Text(now.clone()),
                ];
                conn.execute(sql, params).await.map_err(map_turso)?;
            }
            None => {
                let sql = if write.secret { DELETE_SECRET } else { DELETE };
                conn.execute(sql, vec![plugin, key])
                    .await
                    .map_err(map_turso)?;
            }
        }
    }
    Ok(())
}
