// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{DeviceId, MediaFileId, UserId, WorkId};
use noflex_storage::{
    domain::{SmartTranscodeMark, SmartTranscodeOutcome},
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_text, col_text_opt, dt_to_text, parse_json, text_to_dt,
        to_json, uuid_to_blob,
    },
    error::{backend_msg, map_turso},
};

const COLUMNS: &str = "user_id, work_id, device_id, created_at, outcome, media_file_ids_json";

fn outcome_to_text(outcome: SmartTranscodeOutcome) -> &'static str {
    match outcome {
        SmartTranscodeOutcome::Planned => "planned",
        SmartTranscodeOutcome::NotNeeded => "not_needed",
        SmartTranscodeOutcome::TooLarge => "too_large",
        SmartTranscodeOutcome::NoDevices => "no_devices",
    }
}

fn text_to_outcome(text: &str) -> StorageResult<SmartTranscodeOutcome> {
    match text {
        "planned" => Ok(SmartTranscodeOutcome::Planned),
        "not_needed" => Ok(SmartTranscodeOutcome::NotNeeded),
        "too_large" => Ok(SmartTranscodeOutcome::TooLarge),
        "no_devices" => Ok(SmartTranscodeOutcome::NoDevices),
        other => Err(backend_msg(format!(
            "unknown smart transcode outcome {other}"
        ))),
    }
}

/// The column's value for a mark's scope: see the migration.
fn device_blob(device: Option<DeviceId>) -> Value {
    Value::Blob(device.map_or_else(Vec::new, |id| uuid_to_blob(id.into_uuid())))
}

fn row_to_mark(row: &turso::Row) -> StorageResult<SmartTranscodeMark> {
    let get = |index| row.get_value(index).map_err(map_turso);
    let device = get(2)?;
    let device = col_blob(&device)?;
    Ok(SmartTranscodeMark {
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(&get(0)?)?)?),
        work_id: WorkId::from_uuid(blob_to_uuid(col_blob(&get(1)?)?)?),
        device_id: if device.is_empty() {
            None
        } else {
            Some(DeviceId::from_uuid(blob_to_uuid(device)?))
        },
        created_at: text_to_dt(col_text(&get(3)?)?)?,
        outcome: col_text_opt(&get(4)?)?
            .as_deref()
            .map(text_to_outcome)
            .transpose()?,
        media_file_ids: parse_json(col_text(&get(5)?)?)?,
    })
}

async fn collect(mut rows: turso::Rows) -> StorageResult<Vec<SmartTranscodeMark>> {
    let mut marks = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        marks.push(row_to_mark(&row)?);
    }
    Ok(marks)
}

pub(crate) async fn add(
    conn: &Connection,
    mark: &SmartTranscodeMark,
) -> StorageResult<SmartTranscodeMark> {
    // Marking twice keeps the first place in the queue.
    conn.execute(
        "INSERT INTO smart_transcode_marks (user_id, work_id, device_id, created_at) \
         VALUES (?1, ?2, ?3, ?4) ON CONFLICT(user_id, work_id, device_id) DO NOTHING",
        vec![
            Value::Blob(uuid_to_blob(mark.user_id.into_uuid())),
            Value::Blob(uuid_to_blob(mark.work_id.into_uuid())),
            device_blob(mark.device_id),
            Value::Text(dt_to_text(mark.created_at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    let mut rows = conn
        .query(
            &format!(
                "SELECT {COLUMNS} FROM smart_transcode_marks \
                 WHERE user_id = ?1 AND work_id = ?2 AND device_id = ?3"
            ),
            vec![
                Value::Blob(uuid_to_blob(mark.user_id.into_uuid())),
                Value::Blob(uuid_to_blob(mark.work_id.into_uuid())),
                device_blob(mark.device_id),
            ],
        )
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => row_to_mark(&row),
        None => Err(StorageError::NotFound),
    }
}

pub(crate) async fn remove(
    conn: &Connection,
    user: UserId,
    work: WorkId,
    device: Option<DeviceId>,
) -> StorageResult<bool> {
    let removed = conn
        .execute(
            "DELETE FROM smart_transcode_marks \
             WHERE user_id = ?1 AND work_id = ?2 AND device_id = ?3",
            vec![
                Value::Blob(uuid_to_blob(user.into_uuid())),
                Value::Blob(uuid_to_blob(work.into_uuid())),
                device_blob(device),
            ],
        )
        .await
        .map_err(map_turso)?;
    Ok(removed > 0)
}

pub(crate) async fn for_user(
    conn: &Connection,
    user: UserId,
) -> StorageResult<Vec<SmartTranscodeMark>> {
    collect(
        conn.query(
            &format!(
                "SELECT {COLUMNS} FROM smart_transcode_marks WHERE user_id = ?1 \
                 ORDER BY created_at, work_id, device_id"
            ),
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?,
    )
    .await
}

pub(crate) async fn list(conn: &Connection) -> StorageResult<Vec<SmartTranscodeMark>> {
    collect(
        conn.query(
            &format!(
                "SELECT {COLUMNS} FROM smart_transcode_marks \
                 ORDER BY created_at, work_id, device_id"
            ),
            (),
        )
        .await
        .map_err(map_turso)?,
    )
    .await
}

pub(crate) async fn record_plan(
    conn: &Connection,
    mark: &SmartTranscodeMark,
    outcome: SmartTranscodeOutcome,
    media_file_ids: &[MediaFileId],
) -> StorageResult<()> {
    conn.execute(
        "UPDATE smart_transcode_marks SET outcome = ?4, media_file_ids_json = ?5 \
         WHERE user_id = ?1 AND work_id = ?2 AND device_id = ?3",
        vec![
            Value::Blob(uuid_to_blob(mark.user_id.into_uuid())),
            Value::Blob(uuid_to_blob(mark.work_id.into_uuid())),
            device_blob(mark.device_id),
            Value::Text(outcome_to_text(outcome).to_string()),
            Value::Text(to_json(&media_file_ids)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}
