// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{LibraryId, PlaySessionId, UserId, WorkId};
use noflex_storage::{
    domain::PlaySession,
    error::{StorageError, StorageResult},
    query::{ItemPage, PageCursor, PlaySessionQuery},
};
use time::OffsetDateTime;
use turso::{Connection, Row, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, text_to_dt,
        uuid_to_blob,
    },
    error::map_turso,
};

const COLUMNS: &str = "id, user_id, work_id, root_work_id, library_id, started_at, \
    last_seen_at, watched_ms, position_ms, playing, ended, device";

const UPSERT: &str = "INSERT INTO play_sessions (id, user_id, work_id, root_work_id, \
    library_id, started_at, last_seen_at, watched_ms, position_ms, playing, ended, device) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
    ON CONFLICT(id) DO UPDATE SET \
        user_id = excluded.user_id, \
        work_id = excluded.work_id, \
        root_work_id = excluded.root_work_id, \
        library_id = excluded.library_id, \
        started_at = excluded.started_at, \
        last_seen_at = excluded.last_seen_at, \
        watched_ms = excluded.watched_ms, \
        position_ms = excluded.position_ms, \
        playing = excluded.playing, \
        ended = excluded.ended, \
        device = excluded.device";

fn blob<T>(id: T, into: impl Fn(T) -> uuid::Uuid) -> Value {
    Value::Blob(uuid_to_blob(into(id)))
}

fn int(n: u64, what: &str) -> StorageResult<Value> {
    i64::try_from(n)
        .map(Value::Integer)
        .map_err(|_| StorageError::InvalidInput(format!("{what} exceeds storage range")))
}

pub(crate) async fn upsert(conn: &Connection, s: &PlaySession) -> StorageResult<()> {
    let params = vec![
        blob(s.id, PlaySessionId::into_uuid),
        blob(s.user_id, UserId::into_uuid),
        blob(s.work_id, WorkId::into_uuid),
        blob(s.root_work_id, WorkId::into_uuid),
        blob(s.library_id, LibraryId::into_uuid),
        Value::Text(dt_to_text(s.started_at)?),
        Value::Text(dt_to_text(s.last_seen_at)?),
        int(s.watched_ms, "watched_ms")?,
        int(s.position_ms, "position_ms")?,
        Value::Integer(i64::from(s.playing)),
        Value::Integer(i64::from(s.ended)),
        s.device.clone().map_or(Value::Null, Value::Text),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn open(
    conn: &Connection,
    user: UserId,
    work: WorkId,
    seen_since: OffsetDateTime,
) -> StorageResult<Option<PlaySession>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM play_sessions \
         WHERE user_id = ?1 AND work_id = ?2 AND ended = 0 AND last_seen_at >= ?3 \
         ORDER BY last_seen_at DESC, id DESC LIMIT 1"
    );
    let params = vec![
        blob(user, UserId::into_uuid),
        blob(work, WorkId::into_uuid),
        Value::Text(dt_to_text(seen_since)?),
    ];
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_session(&row))
        .transpose()
}

pub(crate) async fn get(
    conn: &Connection,
    id: PlaySessionId,
) -> StorageResult<Option<PlaySession>> {
    let sql = format!("SELECT {COLUMNS} FROM play_sessions WHERE id = ?1");
    let mut rows = conn
        .query(&sql, vec![blob(id, PlaySessionId::into_uuid)])
        .await
        .map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_session(&row))
        .transpose()
}

pub(crate) async fn list(
    conn: &Connection,
    query: &PlaySessionQuery,
    limit: Option<u32>,
    cursor: Option<PageCursor>,
) -> StorageResult<Vec<PlaySession>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    let mut push = |clause: &str, value: Value, params: &mut Vec<Value>| {
        params.push(value);
        clauses.push(clause.replace('?', &format!("?{}", params.len())));
    };
    if let Some(user) = query.user {
        push("user_id = ?", blob(user, UserId::into_uuid), &mut params);
    }
    if let Some(library) = query.library {
        push(
            "library_id = ?",
            blob(library, LibraryId::into_uuid),
            &mut params,
        );
    }
    if let Some(root) = query.root_work {
        push(
            "root_work_id = ?",
            blob(root, WorkId::into_uuid),
            &mut params,
        );
    }
    if let Some(since) = query.since {
        push(
            "started_at >= ?",
            Value::Text(dt_to_text(since)?),
            &mut params,
        );
    }
    if let Some(until) = query.until {
        push(
            "started_at < ?",
            Value::Text(dt_to_text(until)?),
            &mut params,
        );
    }
    if let Some(cursor) = &cursor {
        let (at, id) = parse_cursor(cursor)?;
        params.push(Value::Text(at));
        params.push(blob(id, PlaySessionId::into_uuid));
        clauses.push(format!(
            "(started_at, id) < (?{}, ?{})",
            params.len() - 1,
            params.len()
        ));
    }
    let mut sql = format!("SELECT {COLUMNS} FROM play_sessions");
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY started_at DESC, id DESC");
    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {}", i64::from(limit) + 1));
    }
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_session(&row)?);
    }
    Ok(out)
}

pub(crate) async fn page(
    conn: &Connection,
    query: &PlaySessionQuery,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<PlaySession>> {
    let limit = limit.max(1);
    let rows = list(conn, query, Some(limit), cursor).await?;
    ItemPage::try_from_lookahead(rows, limit, mint_cursor)
}

/// `<started_at>|<id>`, matching the `(started_at, id)` order.
fn mint_cursor(s: &PlaySession) -> StorageResult<PageCursor> {
    Ok(PageCursor(format!(
        "{}|{}",
        dt_to_text(s.started_at)?,
        s.id
    )))
}

fn parse_cursor(cursor: &PageCursor) -> StorageResult<(String, PlaySessionId)> {
    let malformed =
        || StorageError::InvalidInput(format!("malformed play session cursor: {:?}", cursor.0));
    let (at, id) = cursor.0.rsplit_once('|').ok_or_else(malformed)?;
    let at = dt_to_text(text_to_dt(at).map_err(|_| malformed())?)?;
    let id = id.parse::<uuid::Uuid>().map_err(|_| malformed())?;
    Ok((at, PlaySessionId::from_uuid(id)))
}

fn row_to_session(row: &Row) -> StorageResult<PlaySession> {
    let v = |i: usize| row.get_value(i).map_err(map_turso);
    let uuid = |i: usize| -> StorageResult<uuid::Uuid> { blob_to_uuid(col_blob(&v(i)?)?) };
    let unsigned = |i: usize, what: &str| -> StorageResult<u64> {
        u64::try_from(col_int(&v(i)?)?)
            .map_err(|_| StorageError::Backend(format!("negative {what}").into()))
    };
    Ok(PlaySession {
        id: PlaySessionId::from_uuid(uuid(0)?),
        user_id: UserId::from_uuid(uuid(1)?),
        work_id: WorkId::from_uuid(uuid(2)?),
        root_work_id: WorkId::from_uuid(uuid(3)?),
        library_id: LibraryId::from_uuid(uuid(4)?),
        started_at: text_to_dt(col_text(&v(5)?)?)?,
        last_seen_at: text_to_dt(col_text(&v(6)?)?)?,
        watched_ms: unsigned(7, "watched_ms")?,
        position_ms: unsigned(8, "position_ms")?,
        playing: col_int(&v(9)?)? != 0,
        ended: col_int(&v(10)?)? != 0,
        device: col_text_opt(&v(11)?)?,
    })
}
