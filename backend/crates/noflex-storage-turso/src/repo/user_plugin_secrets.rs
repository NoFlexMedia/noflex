// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::UserId;
use noflex_storage::{domain::UserPluginSecret, error::StorageResult};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{blob_to_uuid, col_blob, col_text, dt_to_text, text_to_dt, uuid_to_blob},
    error::map_turso,
};

const GET: &str = "SELECT user_id, plugin_id, key, value, created_at, updated_at \
    FROM user_plugin_secrets WHERE user_id = ?1 AND plugin_id = ?2 AND key = ?3";

const LIST: &str = "SELECT user_id, plugin_id, key, value, created_at, updated_at \
    FROM user_plugin_secrets WHERE user_id = ?1 AND plugin_id = ?2 ORDER BY key";

// created_at is left out of the conflict update so an upsert keeps the
// original link time while bumping updated_at.
const UPSERT: &str = "INSERT INTO user_plugin_secrets \
    (user_id, plugin_id, key, value, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
    ON CONFLICT(user_id, plugin_id, key) DO UPDATE SET \
        value = excluded.value, \
        updated_at = excluded.updated_at";

const DELETE: &str = "DELETE FROM user_plugin_secrets WHERE user_id = ?1 AND plugin_id = ?2";

pub(crate) async fn get(
    conn: &Connection,
    user: UserId,
    plugin_id: &str,
    key: &str,
) -> StorageResult<Option<UserPluginSecret>> {
    let params = vec![
        Value::Blob(uuid_to_blob(user.into_uuid())),
        Value::Text(plugin_id.to_string()),
        Value::Text(key.to_string()),
    ];
    let mut rows = conn.query(GET, params).await.map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_secret(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list(
    conn: &Connection,
    user: UserId,
    plugin_id: &str,
) -> StorageResult<Vec<UserPluginSecret>> {
    let params = vec![
        Value::Blob(uuid_to_blob(user.into_uuid())),
        Value::Text(plugin_id.to_string()),
    ];
    let mut rows = conn.query(LIST, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_secret(&row)?);
    }
    Ok(out)
}

pub(crate) async fn set(
    conn: &Connection,
    user: UserId,
    plugin_id: &str,
    key: &str,
    value: &str,
) -> StorageResult<()> {
    let now = dt_to_text(OffsetDateTime::now_utc())?;
    let params = vec![
        Value::Blob(uuid_to_blob(user.into_uuid())),
        Value::Text(plugin_id.to_string()),
        Value::Text(key.to_string()),
        Value::Text(value.to_string()),
        Value::Text(now.clone()),
        Value::Text(now),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn delete(conn: &Connection, user: UserId, plugin_id: &str) -> StorageResult<u64> {
    let params = vec![
        Value::Blob(uuid_to_blob(user.into_uuid())),
        Value::Text(plugin_id.to_string()),
    ];
    conn.execute(DELETE, params).await.map_err(map_turso)
}

fn row_to_secret(row: &turso::Row) -> StorageResult<UserPluginSecret> {
    let user_id = row.get_value(0).map_err(map_turso)?;
    let plugin_id = row.get_value(1).map_err(map_turso)?;
    let key = row.get_value(2).map_err(map_turso)?;
    let value = row.get_value(3).map_err(map_turso)?;
    let created_at = row.get_value(4).map_err(map_turso)?;
    let updated_at = row.get_value(5).map_err(map_turso)?;
    Ok(UserPluginSecret {
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(&user_id)?)?),
        plugin_id: col_text(&plugin_id)?.to_string(),
        key: col_text(&key)?.to_string(),
        value: col_text(&value)?.to_string(),
        created_at: text_to_dt(col_text(&created_at)?)?,
        updated_at: text_to_dt(col_text(&updated_at)?)?,
    })
}
