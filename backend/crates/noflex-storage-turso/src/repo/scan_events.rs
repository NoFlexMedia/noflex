// SPDX-License-Identifier: AGPL-3.0-only

use noflex_storage::{domain::ScanEvent, error::StorageResult};
use turso::{Connection, Value};

use crate::{
    convert::{dt_to_text, uuid_to_blob},
    error::map_turso,
};

const INSERT: &str = "INSERT INTO scan_events (\
    library_id, started_at, finished_at, added, updated, removed, errored\
) VALUES (?1,?2,?3,?4,?5,?6,?7)";

pub(crate) async fn record(conn: &Connection, event: &ScanEvent) -> StorageResult<()> {
    let finished = match event.finished_at {
        Some(dt) => Value::Text(dt_to_text(dt)?),
        None => Value::Null,
    };
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(event.library_id.into_uuid())),
        Value::Text(dt_to_text(event.started_at)?),
        finished,
        Value::Integer(event.added as i64),
        Value::Integer(event.updated as i64),
        Value::Integer(event.removed as i64),
        Value::Integer(event.errored as i64),
    ];
    conn.execute(INSERT, params).await.map_err(map_turso)?;
    Ok(())
}
