// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{DeviceId, UserId};
use noflex_storage::{
    domain::Device,
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_int_opt, col_text, col_text_opt, dt_to_text,
        parse_json, text_to_dt, to_json, uuid_to_blob,
    },
    error::map_turso,
};

const COLUMNS: &str = "id, user_id, name, capabilities_json, smart_transcode, max_bitrate_bps, \
    allowed_video_codecs_json, smart_downloads, download_budget_bytes, downloads_wifi_only, \
    created_at, last_seen_at, client";

// The WHERE keeps one user from taking over another's device id: the row
// stays as it was, and the caller sees no rows returned.
const UPSERT: &str = "INSERT INTO devices (id, user_id, name, capabilities_json, smart_transcode, \
        max_bitrate_bps, allowed_video_codecs_json, smart_downloads, download_budget_bytes, \
        downloads_wifi_only, created_at, last_seen_at, client) \
    VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) \
    ON CONFLICT(id) DO UPDATE SET \
        name = excluded.name, \
        client = excluded.client, \
        capabilities_json = excluded.capabilities_json, \
        smart_transcode = excluded.smart_transcode, \
        max_bitrate_bps = excluded.max_bitrate_bps, \
        allowed_video_codecs_json = excluded.allowed_video_codecs_json, \
        smart_downloads = excluded.smart_downloads, \
        download_budget_bytes = excluded.download_budget_bytes, \
        downloads_wifi_only = excluded.downloads_wifi_only, \
        last_seen_at = excluded.last_seen_at \
    WHERE devices.user_id = excluded.user_id \
    RETURNING id";

fn optional_u64(value: Option<u64>) -> StorageResult<Value> {
    value.map_or(Ok(Value::Null), |value| {
        i64::try_from(value)
            .map(Value::Integer)
            .map_err(|_| StorageError::InvalidInput("value out of range".into()))
    })
}

fn row_to_device(row: &turso::Row) -> StorageResult<Device> {
    let get = |index| row.get_value(index).map_err(map_turso);
    Ok(Device {
        id: DeviceId::from_uuid(blob_to_uuid(col_blob(&get(0)?)?)?),
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(&get(1)?)?)?),
        name: col_text(&get(2)?)?.to_string(),
        capabilities: parse_json(col_text(&get(3)?)?)?,
        smart_transcode: col_int(&get(4)?)? != 0,
        max_bitrate_bps: col_int_opt(&get(5)?)?.map(|value| value as u64),
        allowed_video_codecs: parse_json(col_text(&get(6)?)?)?,
        smart_downloads: col_int(&get(7)?)? != 0,
        download_budget_bytes: col_int_opt(&get(8)?)?.map(|value| value as u64),
        downloads_wifi_only: col_int(&get(9)?)? != 0,
        created_at: text_to_dt(col_text(&get(10)?)?)?,
        last_seen_at: text_to_dt(col_text(&get(11)?)?)?,
        client: col_text_opt(&get(12)?)?,
    })
}

pub(crate) async fn upsert(conn: &Connection, device: &Device) -> StorageResult<()> {
    let params = vec![
        Value::Blob(uuid_to_blob(device.id.into_uuid())),
        Value::Blob(uuid_to_blob(device.user_id.into_uuid())),
        Value::Text(device.name.clone()),
        Value::Text(to_json(&device.capabilities.0)?),
        Value::Integer(device.smart_transcode.into()),
        optional_u64(device.max_bitrate_bps)?,
        Value::Text(to_json(&device.allowed_video_codecs)?),
        Value::Integer(device.smart_downloads.into()),
        optional_u64(device.download_budget_bytes)?,
        Value::Integer(device.downloads_wifi_only.into()),
        Value::Text(dt_to_text(device.created_at)?),
        Value::Text(dt_to_text(device.last_seen_at)?),
        device.client.clone().map_or(Value::Null, Value::Text),
    ];
    let mut rows = conn.query(UPSERT, params).await.map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(_) => Ok(()),
        None => Err(StorageError::NotFound),
    }
}

async fn select(conn: &Connection, filter: &str, params: Vec<Value>) -> StorageResult<Vec<Device>> {
    let sql = format!("SELECT {COLUMNS} FROM devices {filter} ORDER BY created_at, id");
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_device(&row)?);
    }
    Ok(out)
}

pub(crate) async fn get(conn: &Connection, id: DeviceId) -> StorageResult<Option<Device>> {
    let found = select(
        conn,
        "WHERE id = ?1",
        vec![Value::Blob(uuid_to_blob(id.into_uuid()))],
    )
    .await?;
    Ok(found.into_iter().next())
}

pub(crate) async fn for_user(conn: &Connection, user: UserId) -> StorageResult<Vec<Device>> {
    select(
        conn,
        "WHERE user_id = ?1",
        vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
    )
    .await
}

pub(crate) async fn smart_transcode(conn: &Connection) -> StorageResult<Vec<Device>> {
    select(
        conn,
        "WHERE smart_transcode = 1 OR smart_downloads = 1",
        vec![],
    )
    .await
}

pub(crate) async fn delete(conn: &Connection, user: UserId, id: DeviceId) -> StorageResult<bool> {
    let removed = conn
        .execute(
            "DELETE FROM devices WHERE id = ?1 AND user_id = ?2",
            vec![
                Value::Blob(uuid_to_blob(id.into_uuid())),
                Value::Blob(uuid_to_blob(user.into_uuid())),
            ],
        )
        .await
        .map_err(map_turso)?;
    Ok(removed > 0)
}
