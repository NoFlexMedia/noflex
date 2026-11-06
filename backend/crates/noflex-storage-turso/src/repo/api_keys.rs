// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{ApiKeyId, UserId};
use noflex_storage::{domain::ApiKey, error::StorageResult};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_text, col_text_opt, dt_to_text, text_to_dt, uuid_to_blob,
    },
    error::map_turso,
};

const INSERT: &str = "INSERT INTO api_keys (id, user_id, name, key_hash, created_at, revoked_at) \
    VALUES (?1,?2,?3,?4,?5,NULL)";

const GET_BY_HASH: &str =
    "SELECT id, user_id, name, created_at, revoked_at FROM api_keys WHERE key_hash = ?1";

const GET: &str = "SELECT id, user_id, name, created_at, revoked_at FROM api_keys WHERE id = ?1";

const LIST_FOR_USER: &str = "SELECT id, user_id, name, created_at, revoked_at FROM api_keys WHERE user_id = ?1 \
    ORDER BY created_at DESC";

const REVOKE: &str = "UPDATE api_keys SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL";

pub(crate) async fn create(
    conn: &Connection,
    user_id: UserId,
    name: &str,
    key_hash: &str,
) -> StorageResult<ApiKey> {
    let key = ApiKey {
        id: ApiKeyId::new(),
        user_id,
        name: name.to_string(),
        created_at: OffsetDateTime::now_utc(),
        revoked_at: None,
    };
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(key.id.into_uuid())),
        Value::Blob(uuid_to_blob(user_id.into_uuid())),
        Value::Text(name.to_string()),
        Value::Text(key_hash.to_string()),
        Value::Text(dt_to_text(key.created_at)?),
    ];
    conn.execute(INSERT, params).await.map_err(map_turso)?;
    Ok(key)
}

pub(crate) async fn get(conn: &Connection, id: ApiKeyId) -> StorageResult<Option<ApiKey>> {
    let mut rows = conn
        .query(GET, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_api_key(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_by_hash(
    conn: &Connection,
    key_hash: &str,
) -> StorageResult<Option<ApiKey>> {
    let mut rows = conn
        .query(GET_BY_HASH, vec![Value::Text(key_hash.to_string())])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_api_key(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_for_user(
    conn: &Connection,
    user_id: UserId,
) -> StorageResult<Vec<ApiKey>> {
    let mut rows = conn
        .query(
            LIST_FOR_USER,
            vec![Value::Blob(uuid_to_blob(user_id.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_api_key(&row)?);
    }
    Ok(out)
}

pub(crate) async fn revoke(conn: &Connection, id: ApiKeyId) -> StorageResult<bool> {
    let n = conn
        .execute(
            REVOKE,
            vec![
                Value::Text(dt_to_text(OffsetDateTime::now_utc())?),
                Value::Blob(uuid_to_blob(id.into_uuid())),
            ],
        )
        .await
        .map_err(map_turso)?;
    Ok(n > 0)
}

fn row_to_api_key(row: &turso::Row) -> StorageResult<ApiKey> {
    let id = row.get_value(0).map_err(map_turso)?;
    let user_id = row.get_value(1).map_err(map_turso)?;
    let name = row.get_value(2).map_err(map_turso)?;
    let created_at = row.get_value(3).map_err(map_turso)?;
    let revoked_at = row.get_value(4).map_err(map_turso)?;
    Ok(ApiKey {
        id: ApiKeyId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(&user_id)?)?),
        name: col_text(&name)?.to_string(),
        created_at: text_to_dt(col_text(&created_at)?)?,
        revoked_at: col_text_opt(&revoked_at)?
            .map(|s| text_to_dt(&s))
            .transpose()?,
    })
}
