// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{DeviceCodeId, UserId};
use noflex_storage::{domain::DeviceCode, error::StorageResult};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_blob_opt, col_text, col_text_opt, dt_to_text, text_to_dt,
        uuid_to_blob,
    },
    error::map_turso,
};

const INSERT: &str = "INSERT INTO device_codes (id, code, user_id, client, device, created_at, expires_at) \
    VALUES (?1,?2,?3,?4,?5,?6,?7)";

const GET_BY_CODE: &str = "SELECT id, code, user_id, client, device, created_at, expires_at FROM device_codes \
    WHERE code = ?1";

const AUTHORIZE: &str = "UPDATE device_codes SET user_id = ?1 WHERE id = ?2";

const DELETE: &str = "DELETE FROM device_codes WHERE id = ?1";

pub(crate) async fn create(conn: &Connection, code: &DeviceCode) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(code.id.into_uuid())),
        Value::Text(code.code.clone()),
        code.user_id
            .map(|u| Value::Blob(uuid_to_blob(u.into_uuid())))
            .unwrap_or(Value::Null),
        code.client.clone().map(Value::Text).unwrap_or(Value::Null),
        code.device.clone().map(Value::Text).unwrap_or(Value::Null),
        Value::Text(dt_to_text(code.created_at)?),
        Value::Text(dt_to_text(code.expires_at)?),
    ];
    conn.execute(INSERT, params).await.map_err(map_turso)?;
    Ok(())
}

/// Expired rows are dead weight: remove them on the way out and report
/// `None`, so "expired" and "never existed" are indistinguishable to
/// callers.
pub(crate) async fn get_by_code(
    conn: &Connection,
    code: &str,
) -> StorageResult<Option<DeviceCode>> {
    let mut rows = conn
        .query(GET_BY_CODE, vec![Value::Text(code.to_string())])
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let found = row_to_device_code(&row)?;
    if found.expires_at <= OffsetDateTime::now_utc() {
        delete(conn, found.id).await?;
        return Ok(None);
    }
    Ok(Some(found))
}

/// The writer lease keeps the eligibility check and authorization update
/// serialized with competing code operations.
pub(crate) async fn authorize(
    conn: &Connection,
    code: &str,
    user_id: UserId,
) -> StorageResult<bool> {
    let Some(found) = get_by_code(conn, code).await? else {
        return Ok(false);
    };
    if found.user_id.is_some() {
        return Ok(false);
    }
    conn.execute(
        AUTHORIZE,
        vec![
            Value::Blob(uuid_to_blob(user_id.into_uuid())),
            Value::Blob(uuid_to_blob(found.id.into_uuid())),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(true)
}

pub(crate) async fn delete(conn: &Connection, id: DeviceCodeId) -> StorageResult<bool> {
    let n = conn
        .execute(DELETE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    Ok(n > 0)
}

fn row_to_device_code(row: &turso::Row) -> StorageResult<DeviceCode> {
    let id = row.get_value(0).map_err(map_turso)?;
    let code = row.get_value(1).map_err(map_turso)?;
    let user_id = row.get_value(2).map_err(map_turso)?;
    let client = row.get_value(3).map_err(map_turso)?;
    let device = row.get_value(4).map_err(map_turso)?;
    let created_at = row.get_value(5).map_err(map_turso)?;
    let expires_at = row.get_value(6).map_err(map_turso)?;
    Ok(DeviceCode {
        id: DeviceCodeId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        code: col_text(&code)?.to_string(),
        user_id: col_blob_opt(&user_id)?
            .map(|b| blob_to_uuid(&b).map(UserId::from_uuid))
            .transpose()?,
        client: col_text_opt(&client)?,
        device: col_text_opt(&device)?,
        created_at: text_to_dt(col_text(&created_at)?)?,
        expires_at: text_to_dt(col_text(&expires_at)?)?,
    })
}
