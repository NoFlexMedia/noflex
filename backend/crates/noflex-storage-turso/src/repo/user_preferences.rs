// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::UserId;
use noflex_storage::{
    domain::UserPreference,
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{blob_to_uuid, col_blob, col_text, dt_to_text, text_to_dt, uuid_to_blob},
    error::map_turso,
};

const LIST: &str = "SELECT user_id, key, value, updated_at \
    FROM user_preferences WHERE user_id = ?1 ORDER BY key";

const UPSERT: &str = "INSERT INTO user_preferences (user_id, key, value, updated_at) \
    VALUES (?1, ?2, ?3, ?4) \
    ON CONFLICT(user_id, key) DO UPDATE SET \
        value = excluded.value, \
        updated_at = excluded.updated_at";

const DELETE: &str = "DELETE FROM user_preferences WHERE user_id = ?1 AND key = ?2";

pub(crate) async fn list(conn: &Connection, user: UserId) -> StorageResult<Vec<UserPreference>> {
    let mut rows = conn
        .query(LIST, vec![Value::Blob(uuid_to_blob(user.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_preference(&row)?);
    }
    Ok(out)
}

pub(crate) async fn set(conn: &Connection, preference: &UserPreference) -> StorageResult<()> {
    let value =
        serde_json::to_string(&preference.value).map_err(|e| StorageError::Backend(Box::new(e)))?;
    let params = vec![
        Value::Blob(uuid_to_blob(preference.user_id.into_uuid())),
        Value::Text(preference.key.clone()),
        Value::Text(value),
        Value::Text(dt_to_text(preference.updated_at)?),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn delete(conn: &Connection, user: UserId, key: &str) -> StorageResult<bool> {
    let changed = conn
        .execute(
            DELETE,
            vec![
                Value::Blob(uuid_to_blob(user.into_uuid())),
                Value::Text(key.to_string()),
            ],
        )
        .await
        .map_err(map_turso)?;
    Ok(changed > 0)
}

fn row_to_preference(row: &turso::Row) -> StorageResult<UserPreference> {
    let user_id = row.get_value(0).map_err(map_turso)?;
    let key = row.get_value(1).map_err(map_turso)?;
    let value = row.get_value(2).map_err(map_turso)?;
    let updated_at = row.get_value(3).map_err(map_turso)?;
    let value =
        serde_json::from_str(col_text(&value)?).map_err(|e| StorageError::Backend(Box::new(e)))?;
    Ok(UserPreference {
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(&user_id)?)?),
        key: col_text(&key)?.to_string(),
        value,
        updated_at: text_to_dt(col_text(&updated_at)?)?,
    })
}
