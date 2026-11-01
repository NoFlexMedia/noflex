// SPDX-License-Identifier: AGPL-3.0-only

use noflex_storage::{
    domain::{PluginTaskPayload, PluginTaskState},
    error::StorageResult,
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{col_int, col_text, col_text_opt, dt_to_text, text_to_dt},
    error::map_turso,
};

const GET_STATE: &str = "SELECT plugin_id, task, cursor, last_run_at, consecutive_failures \
    FROM plugin_task_state WHERE plugin_id = ?1 AND task = ?2";

const UPSERT_STATE: &str = "INSERT INTO plugin_task_state \
    (plugin_id, task, cursor, last_run_at, consecutive_failures) \
    VALUES (?1, ?2, ?3, ?4, ?5) \
    ON CONFLICT(plugin_id, task) DO UPDATE SET \
        cursor = excluded.cursor, \
        last_run_at = excluded.last_run_at, \
        consecutive_failures = excluded.consecutive_failures";

const DELETE_STATES: &str = "DELETE FROM plugin_task_state WHERE plugin_id = ?1";

const LIST_STATES: &str = "SELECT plugin_id, task, cursor, last_run_at, consecutive_failures \
    FROM plugin_task_state WHERE plugin_id = ?1 ORDER BY task";

const ENQUEUE: &str = "INSERT INTO plugin_task_queue (plugin_id, task, payload, enqueued_at) \
    VALUES (?1, ?2, ?3, ?4)";

const OLDEST_QUEUED: &str = "SELECT id, plugin_id, task, payload, enqueued_at \
    FROM plugin_task_queue WHERE plugin_id = ?1 AND task = ?2 ORDER BY id LIMIT 1";

const DELETE_QUEUED: &str = "DELETE FROM plugin_task_queue WHERE id = ?1";

const CLEAR_QUEUE: &str = "DELETE FROM plugin_task_queue WHERE plugin_id = ?1";

pub(crate) async fn get_state(
    conn: &Connection,
    plugin_id: &str,
    task: &str,
) -> StorageResult<Option<PluginTaskState>> {
    let params = vec![
        Value::Text(plugin_id.to_string()),
        Value::Text(task.to_string()),
    ];
    let mut rows = conn.query(GET_STATE, params).await.map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_state(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn set_state(conn: &Connection, state: &PluginTaskState) -> StorageResult<()> {
    let last_run_at = state.last_run_at.map(dt_to_text).transpose()?;
    let params = vec![
        Value::Text(state.plugin_id.clone()),
        Value::Text(state.task.clone()),
        match &state.cursor {
            Some(c) => Value::Text(c.clone()),
            None => Value::Null,
        },
        match last_run_at {
            Some(t) => Value::Text(t),
            None => Value::Null,
        },
        Value::Integer(i64::from(state.consecutive_failures)),
    ];
    conn.execute(UPSERT_STATE, params)
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn delete_states(conn: &Connection, plugin_id: &str) -> StorageResult<u64> {
    conn.execute(DELETE_STATES, vec![Value::Text(plugin_id.to_string())])
        .await
        .map_err(map_turso)
}

pub(crate) async fn list_states(
    conn: &Connection,
    plugin_id: &str,
) -> StorageResult<Vec<PluginTaskState>> {
    let mut rows = conn
        .query(LIST_STATES, vec![Value::Text(plugin_id.to_string())])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_state(&row)?);
    }
    Ok(out)
}

pub(crate) async fn enqueue(
    conn: &Connection,
    plugin_id: &str,
    task: &str,
    payload: Option<&str>,
) -> StorageResult<()> {
    let params = vec![
        Value::Text(plugin_id.to_string()),
        Value::Text(task.to_string()),
        match payload {
            Some(p) => Value::Text(p.to_string()),
            None => Value::Null,
        },
        Value::Text(dt_to_text(OffsetDateTime::now_utc())?),
    ];
    conn.execute(ENQUEUE, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn dequeue(
    conn: &Connection,
    plugin_id: &str,
    task: &str,
) -> StorageResult<Option<PluginTaskPayload>> {
    let params = vec![
        Value::Text(plugin_id.to_string()),
        Value::Text(task.to_string()),
    ];
    let mut rows = conn.query(OLDEST_QUEUED, params).await.map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let payload = row_to_payload(&row)?;
    conn.execute(DELETE_QUEUED, vec![Value::Integer(payload.id)])
        .await
        .map_err(map_turso)?;
    Ok(Some(payload))
}

pub(crate) async fn clear_queue(conn: &Connection, plugin_id: &str) -> StorageResult<u64> {
    conn.execute(CLEAR_QUEUE, vec![Value::Text(plugin_id.to_string())])
        .await
        .map_err(map_turso)
}

fn row_to_state(row: &turso::Row) -> StorageResult<PluginTaskState> {
    let plugin_id = row.get_value(0).map_err(map_turso)?;
    let task = row.get_value(1).map_err(map_turso)?;
    let cursor = row.get_value(2).map_err(map_turso)?;
    let last_run_at = row.get_value(3).map_err(map_turso)?;
    let consecutive_failures = row.get_value(4).map_err(map_turso)?;
    Ok(PluginTaskState {
        plugin_id: col_text(&plugin_id)?.to_string(),
        task: col_text(&task)?.to_string(),
        cursor: col_text_opt(&cursor)?,
        last_run_at: col_text_opt(&last_run_at)?
            .map(|t| text_to_dt(&t))
            .transpose()?,
        consecutive_failures: col_int(&consecutive_failures)? as u32,
    })
}

fn row_to_payload(row: &turso::Row) -> StorageResult<PluginTaskPayload> {
    let id = row.get_value(0).map_err(map_turso)?;
    let plugin_id = row.get_value(1).map_err(map_turso)?;
    let task = row.get_value(2).map_err(map_turso)?;
    let payload = row.get_value(3).map_err(map_turso)?;
    let enqueued_at = row.get_value(4).map_err(map_turso)?;
    Ok(PluginTaskPayload {
        id: col_int(&id)?,
        plugin_id: col_text(&plugin_id)?.to_string(),
        task: col_text(&task)?.to_string(),
        payload: col_text_opt(&payload)?,
        enqueued_at: text_to_dt(col_text(&enqueued_at)?)?,
    })
}
