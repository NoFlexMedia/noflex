// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{MediaFileId, OptimizedVersionId, TonemapCurve};
use noflex_storage::{
    domain::{OptimizedVersion, OptimizedVersionStatus, SmartTranscodingSettings},
    error::StorageResult,
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_int_opt, col_real_opt, col_text, col_text_opt,
        dt_to_text, parse_json, text_to_dt, to_json, uuid_to_blob,
    },
    error::{backend_msg, map_turso},
};

const COLUMNS: &str = "id, media_file_id, variant_key, status, priority, plan_json, source_size, \
    source_mtime, progress, attempts, path, size_bytes, index_json, probe_json, error, \
    created_at, finished_at, last_used_at";

fn status_to_text(status: OptimizedVersionStatus) -> &'static str {
    match status {
        OptimizedVersionStatus::Queued => "queued",
        OptimizedVersionStatus::Running => "running",
        OptimizedVersionStatus::Ready => "ready",
        OptimizedVersionStatus::Failed => "failed",
    }
}

fn text_to_status(value: &str) -> StorageResult<OptimizedVersionStatus> {
    match value {
        "queued" => Ok(OptimizedVersionStatus::Queued),
        "running" => Ok(OptimizedVersionStatus::Running),
        "ready" => Ok(OptimizedVersionStatus::Ready),
        "failed" => Ok(OptimizedVersionStatus::Failed),
        other => Err(backend_msg(format!("unknown version status {other}"))),
    }
}

fn optional_json(value: &Option<serde_json::Value>) -> StorageResult<Value> {
    value
        .as_ref()
        .map_or(Ok(Value::Null), |value| Ok(Value::Text(to_json(value)?)))
}

fn optional_time(value: Option<OffsetDateTime>) -> StorageResult<Value> {
    value.map_or(Ok(Value::Null), |value| Ok(Value::Text(dt_to_text(value)?)))
}

fn row_to_version(row: &turso::Row) -> StorageResult<OptimizedVersion> {
    let get = |index| row.get_value(index).map_err(map_turso);
    let json = |value: Option<String>| value.as_deref().map(parse_json).transpose();
    let time = |value: Option<String>| value.as_deref().map(text_to_dt).transpose();
    Ok(OptimizedVersion {
        id: OptimizedVersionId::from_uuid(blob_to_uuid(col_blob(&get(0)?)?)?),
        media_file_id: MediaFileId::from_uuid(blob_to_uuid(col_blob(&get(1)?)?)?),
        variant_key: col_text(&get(2)?)?.to_string(),
        status: text_to_status(col_text(&get(3)?)?)?,
        priority: col_int(&get(4)?)? as i32,
        plan: parse_json(col_text(&get(5)?)?)?,
        source_size: col_int(&get(6)?)? as u64,
        source_mtime: text_to_dt(col_text(&get(7)?)?)?,
        progress: col_real_opt(&get(8)?)?.unwrap_or(0.0),
        attempts: col_int(&get(9)?)? as u32,
        path: col_text_opt(&get(10)?)?,
        size_bytes: col_int_opt(&get(11)?)?.map(|size| size as u64),
        index: json(col_text_opt(&get(12)?)?)?,
        probe: json(col_text_opt(&get(13)?)?)?,
        error: col_text_opt(&get(14)?)?,
        created_at: text_to_dt(col_text(&get(15)?)?)?,
        finished_at: time(col_text_opt(&get(16)?)?)?,
        last_used_at: time(col_text_opt(&get(17)?)?)?,
    })
}

async fn select(
    conn: &Connection,
    filter: &str,
    params: Vec<Value>,
) -> StorageResult<Vec<OptimizedVersion>> {
    let sql = format!("SELECT {COLUMNS} FROM optimized_versions {filter}");
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_version(&row)?);
    }
    Ok(out)
}

pub(crate) async fn enqueue(conn: &Connection, version: &OptimizedVersion) -> StorageResult<()> {
    // Re-planning only moves a waiting job up or down the queue.
    conn.execute(
        "INSERT INTO optimized_versions (id, media_file_id, variant_key, status, priority, \
            plan_json, source_size, source_mtime, created_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
         ON CONFLICT(media_file_id, variant_key) DO UPDATE SET priority = excluded.priority \
         WHERE optimized_versions.status = 'queued'",
        vec![
            Value::Blob(uuid_to_blob(version.id.into_uuid())),
            Value::Blob(uuid_to_blob(version.media_file_id.into_uuid())),
            Value::Text(version.variant_key.clone()),
            Value::Text(status_to_text(version.status).to_string()),
            Value::Integer(version.priority.into()),
            Value::Text(to_json(&version.plan)?),
            Value::Integer(version.source_size as i64),
            Value::Text(dt_to_text(version.source_mtime)?),
            Value::Text(dt_to_text(version.created_at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn get(
    conn: &Connection,
    id: OptimizedVersionId,
) -> StorageResult<Option<OptimizedVersion>> {
    Ok(select(
        conn,
        "WHERE id = ?1",
        vec![Value::Blob(uuid_to_blob(id.into_uuid()))],
    )
    .await?
    .into_iter()
    .next())
}

pub(crate) async fn for_media(
    conn: &Connection,
    media: MediaFileId,
) -> StorageResult<Vec<OptimizedVersion>> {
    select(
        conn,
        "WHERE media_file_id = ?1 ORDER BY created_at, id",
        vec![Value::Blob(uuid_to_blob(media.into_uuid()))],
    )
    .await
}

pub(crate) async fn list(conn: &Connection) -> StorageResult<Vec<OptimizedVersion>> {
    select(conn, "ORDER BY status, priority, created_at, id", vec![]).await
}

pub(crate) async fn claim(conn: &Connection) -> StorageResult<Option<OptimizedVersion>> {
    let sql = format!(
        "UPDATE optimized_versions SET status = 'running', attempts = attempts + 1, progress = 0, \
            error = NULL \
         WHERE id = (SELECT id FROM optimized_versions WHERE status = 'queued' \
            ORDER BY priority, created_at, id LIMIT 1) \
         RETURNING {COLUMNS}"
    );
    let mut rows = conn.query(&sql, ()).await.map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_version(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn update(conn: &Connection, version: &OptimizedVersion) -> StorageResult<()> {
    conn.execute(
        "UPDATE optimized_versions SET status = ?2, priority = ?3, progress = ?4, attempts = ?5, \
            path = ?6, size_bytes = ?7, index_json = ?8, probe_json = ?9, error = ?10, \
            finished_at = ?11, last_used_at = ?12 \
         WHERE id = ?1",
        vec![
            Value::Blob(uuid_to_blob(version.id.into_uuid())),
            Value::Text(status_to_text(version.status).to_string()),
            Value::Integer(version.priority.into()),
            Value::Real(f64::from(version.progress)),
            Value::Integer(version.attempts.into()),
            version.path.clone().map_or(Value::Null, Value::Text),
            version
                .size_bytes
                .map_or(Value::Null, |size| Value::Integer(size as i64)),
            optional_json(&version.index)?,
            optional_json(&version.probe)?,
            version.error.clone().map_or(Value::Null, Value::Text),
            optional_time(version.finished_at)?,
            optional_time(version.last_used_at)?,
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn delete(conn: &Connection, id: OptimizedVersionId) -> StorageResult<()> {
    conn.execute(
        "DELETE FROM optimized_versions WHERE id = ?1",
        vec![Value::Blob(uuid_to_blob(id.into_uuid()))],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn requeue_running(conn: &Connection) -> StorageResult<u64> {
    conn.execute(
        "UPDATE optimized_versions SET status = 'queued', progress = 0 WHERE status = 'running'",
        (),
    )
    .await
    .map_err(map_turso)
}

pub(crate) async fn settings(conn: &Connection) -> StorageResult<SmartTranscodingSettings> {
    let mut rows = conn
        .query(
            "SELECT dir, max_storage_gb, max_version_gb FROM smart_transcoding_settings",
            (),
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(SmartTranscodingSettings::default());
    };
    Ok(SmartTranscodingSettings {
        dir: col_text_opt(&row.get_value(0).map_err(map_turso)?)?,
        max_storage_gb: col_int_opt(&row.get_value(1).map_err(map_turso)?)?.map(|gb| gb as u64),
        max_version_gb: col_int_opt(&row.get_value(2).map_err(map_turso)?)?.map(|gb| gb as u64),
    })
}

pub(crate) async fn set_settings(
    conn: &Connection,
    settings: &SmartTranscodingSettings,
) -> StorageResult<()> {
    let gb = |value: Option<u64>| value.map_or(Value::Null, |gb| Value::Integer(gb as i64));
    conn.execute(
        "INSERT INTO smart_transcoding_settings (id, dir, max_storage_gb, max_version_gb) \
         VALUES (1, ?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET dir = excluded.dir, \
             max_storage_gb = excluded.max_storage_gb, max_version_gb = excluded.max_version_gb",
        vec![
            settings.dir.clone().map_or(Value::Null, Value::Text),
            gb(settings.max_storage_gb),
            gb(settings.max_version_gb),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn tonemap_curve(conn: &Connection) -> StorageResult<Option<TonemapCurve>> {
    let mut rows = conn
        .query("SELECT tonemap_curve FROM transcoding_settings", ())
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    col_text_opt(&row.get_value(0).map_err(map_turso)?)?
        .map(|curve| parse_json(&curve))
        .transpose()
}

pub(crate) async fn set_tonemap_curve(
    conn: &Connection,
    curve: Option<TonemapCurve>,
) -> StorageResult<()> {
    let curve = curve
        .map(|curve| to_json(&curve))
        .transpose()?
        .map_or(Value::Null, Value::Text);
    conn.execute(
        "INSERT INTO transcoding_settings (id, tonemap_curve) VALUES (1, ?1) \
         ON CONFLICT(id) DO UPDATE SET tonemap_curve = excluded.tonemap_curve",
        vec![curve],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}
