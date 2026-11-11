// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;

use noflex_core::{EditionId, WorkId};
use noflex_storage::{
    domain::{Chapter, Edition},
    error::StorageResult,
};
use turso::{Connection, Value};

use crate::{
    convert::{blob_to_uuid, col_blob, col_int_opt, col_text, parse_json, to_json, uuid_to_blob},
    error::{backend_msg, map_turso},
};

const UPSERT: &str = "INSERT INTO editions (\
    id, work_id, name, runtime_ms, intro_start_ms, intro_end_ms, credits_start_ms, chapters_json\
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
ON CONFLICT(work_id, name) DO UPDATE SET \
    work_id = excluded.work_id, \
    name = excluded.name, \
    runtime_ms = excluded.runtime_ms, \
    intro_start_ms = excluded.intro_start_ms, \
    intro_end_ms = excluded.intro_end_ms, \
    credits_start_ms = excluded.credits_start_ms, \
    chapters_json = excluded.chapters_json \
RETURNING id";

// Same parameter layout as UPSERT. Run first so an existing id keeps its
// identity across a rename or re-parent instead of colliding on the PK.
const UPDATE_BY_ID: &str = "UPDATE editions SET \
    work_id = ?2, \
    name = ?3, \
    runtime_ms = ?4, \
    intro_start_ms = ?5, \
    intro_end_ms = ?6, \
    credits_start_ms = ?7, \
    chapters_json = ?8 \
WHERE id = ?1";

const FOR_WORK: &str = "SELECT id, work_id, name, runtime_ms, intro_start_ms, intro_end_ms, credits_start_ms, chapters_json \
    FROM editions WHERE work_id = ?1 ORDER BY name";

const GET: &str = "SELECT id, work_id, name, runtime_ms, intro_start_ms, intro_end_ms, credits_start_ms, chapters_json \
    FROM editions WHERE id = ?1";

const DELETE: &str = "DELETE FROM editions WHERE id = ?1";

pub(crate) async fn upsert(conn: &Connection, ed: &Edition) -> StorageResult<EditionId> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(ed.id.into_uuid())),
        Value::Blob(uuid_to_blob(ed.work_id.into_uuid())),
        Value::Text(ed.name.clone()),
        ed.runtime_ms.map(Value::Integer).unwrap_or(Value::Null),
        ed.intro_start_ms.map(Value::Integer).unwrap_or(Value::Null),
        ed.intro_end_ms.map(Value::Integer).unwrap_or(Value::Null),
        ed.credits_start_ms
            .map(Value::Integer)
            .unwrap_or(Value::Null),
        Value::Text(to_json(&ed.chapters)?),
    ];
    let updated = conn
        .execute(UPDATE_BY_ID, params.clone())
        .await
        .map_err(map_turso)?;
    if updated != 0 {
        return Ok(ed.id);
    }
    let mut rows = conn.query(UPSERT, params).await.map_err(map_turso)?;
    let row = rows
        .next()
        .await
        .map_err(map_turso)?
        .ok_or_else(|| backend_msg("edition upsert returned no id"))?;
    let id = row.get_value(0).map_err(map_turso)?;
    Ok(EditionId::from_uuid(blob_to_uuid(col_blob(&id)?)?))
}

pub(crate) async fn get(conn: &Connection, id: EditionId) -> StorageResult<Option<Edition>> {
    let mut rows = conn
        .query(GET, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    Ok(Some(row_to_edition(&row)?))
}

pub(crate) async fn get_many(conn: &Connection, ids: &[EditionId]) -> StorageResult<Vec<Edition>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = GET.replace("id = ?1", &format!("id IN ({placeholders})"));
    let params = ids
        .iter()
        .map(|id| Value::Blob(uuid_to_blob(id.into_uuid())))
        .collect::<Vec<_>>();
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut by_id = HashMap::with_capacity(ids.len());
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let edition = row_to_edition(&row)?;
        by_id.insert(edition.id, edition);
    }
    Ok(ids.iter().filter_map(|id| by_id.get(id).cloned()).collect())
}

pub(crate) async fn for_work(conn: &Connection, work: WorkId) -> StorageResult<Vec<Edition>> {
    let mut rows = conn
        .query(FOR_WORK, vec![Value::Blob(uuid_to_blob(work.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_edition(&row)?);
    }
    Ok(out)
}

pub(super) fn row_to_edition(row: &turso::Row) -> StorageResult<Edition> {
    let id = row.get_value(0).map_err(map_turso)?;
    let work_id = row.get_value(1).map_err(map_turso)?;
    let name = row.get_value(2).map_err(map_turso)?;
    let runtime_ms = row.get_value(3).map_err(map_turso)?;
    let intro_start_ms = row.get_value(4).map_err(map_turso)?;
    let intro_end_ms = row.get_value(5).map_err(map_turso)?;
    let credits_start_ms = row.get_value(6).map_err(map_turso)?;
    let chapters_json = row.get_value(7).map_err(map_turso)?;
    let chapters: Vec<Chapter> = parse_json(col_text(&chapters_json)?)?;
    Ok(Edition {
        id: EditionId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        work_id: WorkId::from_uuid(blob_to_uuid(col_blob(&work_id)?)?),
        name: col_text(&name)?.to_string(),
        runtime_ms: col_int_opt(&runtime_ms)?,
        intro_start_ms: col_int_opt(&intro_start_ms)?,
        intro_end_ms: col_int_opt(&intro_end_ms)?,
        credits_start_ms: col_int_opt(&credits_start_ms)?,
        chapters,
    })
}

pub(crate) async fn delete(conn: &Connection, id: EditionId) -> StorageResult<()> {
    conn.execute(DELETE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    Ok(())
}
