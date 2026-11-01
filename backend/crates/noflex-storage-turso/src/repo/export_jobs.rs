// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{EditionId, ExportJobId, UserId};
use noflex_storage::{
    domain::{ExportFormat, ExportJob, ExportJobStatus},
    error::{StorageError, StorageResult},
    query::{ITEM_PAGE_MAX, ItemPage, PageCursor},
    store::{ExportJobCancellation, ExportJobLease},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_text, col_text_opt, dt_to_text, text_to_dt, uuid_to_blob,
    },
    error::{backend_msg, map_turso},
};

const COLUMNS: &str = "id, user_id, edition_id, format, status, progress, output_path, error, created_at, started_at, finished_at";

fn format_to_text(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Mp4 => "mp4",
        ExportFormat::Mkv => "mkv",
    }
}

fn text_to_format(value: &str) -> StorageResult<ExportFormat> {
    match value {
        "mp4" => Ok(ExportFormat::Mp4),
        "mkv" => Ok(ExportFormat::Mkv),
        other => Err(backend_msg(format!("unknown export format {other}"))),
    }
}

fn status_to_text(status: ExportJobStatus) -> &'static str {
    match status {
        ExportJobStatus::Queued => "queued",
        ExportJobStatus::Running => "running",
        ExportJobStatus::CancelRequested => "cancel_requested",
        ExportJobStatus::Completed => "completed",
        ExportJobStatus::Failed => "failed",
        ExportJobStatus::Cancelled => "cancelled",
    }
}

fn text_to_status(value: &str) -> StorageResult<ExportJobStatus> {
    match value {
        "queued" => Ok(ExportJobStatus::Queued),
        "running" => Ok(ExportJobStatus::Running),
        "cancel_requested" => Ok(ExportJobStatus::CancelRequested),
        "completed" => Ok(ExportJobStatus::Completed),
        "failed" => Ok(ExportJobStatus::Failed),
        "cancelled" => Ok(ExportJobStatus::Cancelled),
        other => Err(backend_msg(format!("unknown export job status {other}"))),
    }
}

pub(crate) async fn create(conn: &Connection, job: &ExportJob) -> StorageResult<()> {
    let sql = format!(
        "INSERT INTO export_jobs ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
    );
    conn.execute(
        &sql,
        vec![
            Value::Blob(uuid_to_blob(job.id.into_uuid())),
            Value::Blob(uuid_to_blob(job.user_id.into_uuid())),
            Value::Blob(uuid_to_blob(job.edition_id.into_uuid())),
            Value::Text(format_to_text(job.format).to_string()),
            Value::Text(status_to_text(job.status).to_string()),
            Value::Real(job.progress as f64),
            text_value(job.output_path.as_deref()),
            text_value(job.error.as_deref()),
            Value::Text(dt_to_text(job.created_at)?),
            optional_dt(job.started_at)?,
            optional_dt(job.finished_at)?,
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

fn text_value(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |value| Value::Text(value.to_string()))
}

fn optional_dt(value: Option<OffsetDateTime>) -> StorageResult<Value> {
    Ok(value
        .map(dt_to_text)
        .transpose()?
        .map_or(Value::Null, Value::Text))
}

fn row_to_job(row: &turso::Row) -> StorageResult<ExportJob> {
    let progress = match row.get_value(5).map_err(map_turso)? {
        Value::Real(value) => value as f32,
        Value::Integer(value) => value as f32,
        Value::Null => 0.0,
        other => {
            return Err(backend_msg(format!(
                "expected REAL progress, got {other:?}"
            )));
        }
    };
    Ok(ExportJob {
        id: ExportJobId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(0).map_err(map_turso)?,
        )?)?),
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(1).map_err(map_turso)?,
        )?)?),
        edition_id: EditionId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(2).map_err(map_turso)?,
        )?)?),
        format: text_to_format(col_text(&row.get_value(3).map_err(map_turso)?)?)?,
        status: text_to_status(col_text(&row.get_value(4).map_err(map_turso)?)?)?,
        progress,
        output_path: col_text_opt(&row.get_value(6).map_err(map_turso)?)?,
        error: col_text_opt(&row.get_value(7).map_err(map_turso)?)?,
        created_at: required_dt(row.get_value(8).map_err(map_turso)?, "created_at")?,
        started_at: nullable_dt(row.get_value(9).map_err(map_turso)?, "started_at")?,
        finished_at: nullable_dt(row.get_value(10).map_err(map_turso)?, "finished_at")?,
    })
}

fn required_dt(value: Value, column: &str) -> StorageResult<OffsetDateTime> {
    match value {
        Value::Text(value) => text_to_dt(&value),
        other => Err(backend_msg(format!(
            "expected TEXT {column}, got {other:?}"
        ))),
    }
}

fn nullable_dt(value: Value, column: &str) -> StorageResult<Option<OffsetDateTime>> {
    match value {
        Value::Null => Ok(None),
        Value::Text(value) => Ok(Some(text_to_dt(&value)?)),
        other => Err(backend_msg(format!(
            "expected TEXT {column}, got {other:?}"
        ))),
    }
}

pub(crate) async fn get(conn: &Connection, id: ExportJobId) -> StorageResult<Option<ExportJob>> {
    let sql = format!("SELECT {COLUMNS} FROM export_jobs WHERE id = ?1");
    query_one(conn, &sql, vec![Value::Blob(uuid_to_blob(id.into_uuid()))]).await
}

pub(crate) async fn list_for_user(
    conn: &Connection,
    user: UserId,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<ExportJob>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let (before_at, before_id) = match cursor.as_ref().map(parse_cursor).transpose()? {
        Some((at, id)) => (Value::Text(at), Value::Blob(id.to_vec())),
        None => (Value::Null, Value::Null),
    };
    let sql = format!(
        "SELECT {COLUMNS} FROM export_jobs WHERE user_id = ?1 \
         AND (?2 IS NULL OR (created_at, id) < (?2, ?3)) \
         ORDER BY created_at DESC, id DESC LIMIT ?4"
    );
    let mut rows = conn
        .query(
            &sql,
            vec![
                Value::Blob(uuid_to_blob(user.into_uuid())),
                before_at,
                before_id,
                Value::Integer(i64::from(limit) + 1),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut jobs = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        jobs.push(row_to_job(&row)?);
    }
    ItemPage::try_from_lookahead(jobs, limit, |job| {
        Ok(PageCursor(format!(
            "{}|{}",
            dt_to_text(job.created_at)?,
            job.id.into_uuid()
        )))
    })
}

/// `<created_at>|<job uuid>`, matching the `(created_at, id)` order.
fn parse_cursor(cursor: &PageCursor) -> StorageResult<(String, [u8; 16])> {
    let malformed =
        || StorageError::InvalidInput(format!("malformed export job cursor: {:?}", cursor.0));
    let (at, id) = cursor.0.rsplit_once('|').ok_or_else(malformed)?;
    let at = dt_to_text(text_to_dt(at).map_err(|_| malformed())?)?;
    let id = uuid::Uuid::parse_str(id).map_err(|_| malformed())?;
    Ok((at, *id.as_bytes()))
}

async fn query_one(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> StorageResult<Option<ExportJob>> {
    let mut rows = conn.query(sql, params).await.map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_job(&row))
        .transpose()
}

pub(crate) async fn claim(
    conn: &Connection,
    worker_id: &str,
    lease_token: &str,
    now: OffsetDateTime,
    lease_expires_at: OffsetDateTime,
    max_attempts: u32,
) -> StorageResult<Option<ExportJobLease>> {
    let now = dt_to_text(now)?;
    conn.execute(
        "UPDATE export_jobs SET status = 'cancelled', finished_at = ?1, worker_id = NULL, lease_token = NULL, lease_expires_at = NULL WHERE status = 'cancel_requested' AND lease_expires_at <= ?1",
        vec![Value::Text(now.clone())],
    ).await.map_err(map_turso)?;
    conn.execute(
        "UPDATE export_jobs SET status = 'failed', error = 'export retry limit exceeded', finished_at = ?1, worker_id = NULL, lease_token = NULL, lease_expires_at = NULL WHERE status = 'running' AND lease_expires_at <= ?1 AND attempt_count >= ?2",
        vec![Value::Text(now.clone()), Value::Integer(max_attempts.into())],
    ).await.map_err(map_turso)?;

    let sql = format!(
        "UPDATE export_jobs SET status = 'running', worker_id = ?1, lease_token = ?2, lease_expires_at = ?3, attempt_count = attempt_count + 1, started_at = COALESCE(started_at, ?4), error = NULL WHERE id = (SELECT id FROM export_jobs WHERE status = 'queued' OR (status = 'running' AND lease_expires_at <= ?4 AND attempt_count < ?5) ORDER BY created_at, id LIMIT 1) RETURNING {COLUMNS}, attempt_count"
    );
    let mut rows = conn
        .query(
            &sql,
            vec![
                Value::Text(worker_id.to_string()),
                Value::Text(lease_token.to_string()),
                Value::Text(dt_to_text(lease_expires_at)?),
                Value::Text(now),
                Value::Integer(max_attempts.into()),
            ],
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let attempt_count = match row.get_value(11).map_err(map_turso)? {
        Value::Integer(value) => {
            u32::try_from(value).map_err(|_| backend_msg("invalid export attempt count"))?
        }
        other => {
            return Err(backend_msg(format!(
                "expected INTEGER attempt_count, got {other:?}"
            )));
        }
    };
    Ok(Some(ExportJobLease {
        job: row_to_job(&row)?,
        lease_token: lease_token.to_string(),
        attempt_count,
    }))
}

pub(crate) async fn update_progress(
    conn: &Connection,
    id: ExportJobId,
    lease_token: &str,
    progress: f32,
    lease_expires_at: OffsetDateTime,
) -> StorageResult<Option<ExportJobStatus>> {
    let mut rows = conn.query(
        "UPDATE export_jobs SET progress = ?3, lease_expires_at = ?4 WHERE id = ?1 AND lease_token = ?2 AND status IN ('running', 'cancel_requested') RETURNING status",
        vec![
            Value::Blob(uuid_to_blob(id.into_uuid())),
            Value::Text(lease_token.to_string()),
            Value::Real(f64::from(progress.clamp(0.0, 1.0))),
            Value::Text(dt_to_text(lease_expires_at)?),
        ],
    ).await.map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    Ok(Some(text_to_status(col_text(
        &row.get_value(0).map_err(map_turso)?,
    )?)?))
}

pub(crate) async fn complete(
    conn: &Connection,
    id: ExportJobId,
    token: &str,
    output_path: &str,
) -> StorageResult<bool> {
    terminal(
        conn,
        id,
        token,
        "completed",
        Some(output_path),
        None,
        "running",
    )
    .await
}

pub(crate) async fn fail(
    conn: &Connection,
    id: ExportJobId,
    token: &str,
    error: &str,
) -> StorageResult<bool> {
    terminal(conn, id, token, "failed", None, Some(error), "running").await
}

pub(crate) async fn cancel_claimed(
    conn: &Connection,
    id: ExportJobId,
    token: &str,
) -> StorageResult<bool> {
    terminal(conn, id, token, "cancelled", None, None, "cancel_requested").await
}

async fn terminal(
    conn: &Connection,
    id: ExportJobId,
    token: &str,
    status: &str,
    output: Option<&str>,
    error: Option<&str>,
    from_status: &str,
) -> StorageResult<bool> {
    let changed = conn.execute(
        "UPDATE export_jobs SET status = ?3, progress = CASE WHEN ?3 = 'completed' THEN 1.0 ELSE progress END, output_path = ?4, error = ?5, finished_at = ?6, worker_id = NULL, lease_token = NULL, lease_expires_at = NULL WHERE id = ?1 AND lease_token = ?2 AND status = ?7",
        vec![
            Value::Blob(uuid_to_blob(id.into_uuid())), Value::Text(token.to_string()),
            Value::Text(status.to_string()), text_value(output), text_value(error),
            Value::Text(dt_to_text(OffsetDateTime::now_utc())?), Value::Text(from_status.to_string()),
        ],
    ).await.map_err(map_turso)?;
    Ok(changed == 1)
}

pub(crate) async fn request_cancellation(
    conn: &Connection,
    id: ExportJobId,
    owner: UserId,
) -> StorageResult<Option<ExportJobCancellation>> {
    let now = dt_to_text(OffsetDateTime::now_utc())?;
    let changed = conn.execute(
        "UPDATE export_jobs SET status = CASE status WHEN 'queued' THEN 'cancelled' ELSE 'cancel_requested' END, finished_at = CASE status WHEN 'queued' THEN ?3 ELSE finished_at END WHERE id = ?1 AND user_id = ?2 AND status IN ('queued', 'running')",
        vec![Value::Blob(uuid_to_blob(id.into_uuid())), Value::Blob(uuid_to_blob(owner.into_uuid())), Value::Text(now)],
    ).await.map_err(map_turso)?;
    let sql = format!("SELECT {COLUMNS} FROM export_jobs WHERE id = ?1 AND user_id = ?2");
    Ok(query_one(
        conn,
        &sql,
        vec![
            Value::Blob(uuid_to_blob(id.into_uuid())),
            Value::Blob(uuid_to_blob(owner.into_uuid())),
        ],
    )
    .await?
    .map(|job| ExportJobCancellation {
        job,
        changed: changed == 1,
    }))
}

pub(crate) async fn delete(conn: &Connection, id: ExportJobId) -> StorageResult<()> {
    conn.execute(
        "DELETE FROM export_jobs WHERE id = ?1",
        vec![Value::Blob(uuid_to_blob(id.into_uuid()))],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_text_round_trip() {
        for status in [
            ExportJobStatus::Queued,
            ExportJobStatus::Running,
            ExportJobStatus::CancelRequested,
            ExportJobStatus::Completed,
            ExportJobStatus::Failed,
            ExportJobStatus::Cancelled,
        ] {
            assert_eq!(text_to_status(status_to_text(status)).unwrap(), status);
        }
    }
}
