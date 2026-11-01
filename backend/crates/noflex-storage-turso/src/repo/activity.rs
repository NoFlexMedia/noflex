// SPDX-License-Identifier: AGPL-3.0-only

use std::str::FromStr;

use noflex_storage::{
    domain::{ActivityEntry, ActivityKind},
    error::{StorageError, StorageResult},
    query::{ItemPage, PageCursor},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{dt_to_text, text_to_dt},
    error::map_turso,
};

const INSERT: &str = "INSERT INTO activity_log (kind, actor, subject, detail, occurred_at) \
    VALUES (?1, ?2, ?3, ?4, ?5)";

const LAST_ID: &str = "SELECT last_insert_rowid()";

pub(crate) async fn append(
    conn: &Connection,
    kind: &str,
    actor: Option<&str>,
    subject: Option<&str>,
    detail: Option<&str>,
    occurred_at: OffsetDateTime,
) -> StorageResult<i64> {
    let params: Vec<Value> = vec![
        Value::Text(kind.to_string()),
        Value::Text(actor.unwrap_or("").to_string()),
        Value::Text(subject.unwrap_or("").to_string()),
        Value::Text(detail.unwrap_or("").to_string()),
        Value::Text(dt_to_text(occurred_at)?),
    ];
    conn.execute(INSERT, params).await.map_err(map_turso)?;
    let mut rows = conn.query(LAST_ID, ()).await.map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Err(StorageError::Backend(
            "last_insert_rowid returned no rows".into(),
        ));
    };
    let id = match row.get_value(0).map_err(map_turso)? {
        Value::Integer(n) => n,
        other => {
            return Err(StorageError::Backend(
                format!("expected INTEGER id, got {other:?}").into(),
            ));
        }
    };
    Ok(id)
}

const LIST_BASE: &str = "SELECT id, kind, actor, subject, detail, occurred_at \
    FROM activity_log";

pub(crate) async fn list(
    conn: &Connection,
    kinds: &[&str],
    since: Option<OffsetDateTime>,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<ActivityEntry>> {
    let limit = limit.max(1);
    let mut sql = String::from(LIST_BASE);
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if !kinds.is_empty() {
        let placeholders: Vec<String> = (0..kinds.len())
            .map(|i| format!("?{}", params.len() + 1 + i))
            .collect();
        where_clauses.push(format!("kind IN ({})", placeholders.join(",")));
        for k in kinds {
            params.push(Value::Text((*k).to_string()));
        }
    }
    if let Some(s) = since {
        where_clauses.push(format!("occurred_at >= ?{}", params.len() + 1));
        params.push(Value::Text(dt_to_text(s)?));
    }
    if let Some(cursor) = &cursor {
        let (at, id) = parse_cursor(cursor)?;
        where_clauses.push(format!(
            "(occurred_at, id) < (?{}, ?{})",
            params.len() + 1,
            params.len() + 2
        ));
        params.push(Value::Text(at));
        params.push(Value::Integer(id));
    }
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY occurred_at DESC, id DESC");
    sql.push_str(&format!(" LIMIT {}", i64::from(limit) + 1));

    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = match row.get_value(0).map_err(map_turso)? {
            Value::Integer(n) => n,
            other => {
                return Err(StorageError::Backend(
                    format!("expected INTEGER id, got {other:?}").into(),
                ));
            }
        };
        let kind: ActivityKind = match row.get_value(1).map_err(map_turso)? {
            Value::Text(s) => ActivityKind::from_str(&s).unwrap_or(ActivityKind::Custom),
            other => {
                return Err(StorageError::Backend(
                    format!("expected TEXT kind, got {other:?}").into(),
                ));
            }
        };
        let actor = optional_text(&row, 2, "actor")?;
        let subject = optional_text(&row, 3, "subject")?;
        let detail = optional_text(&row, 4, "detail")?;
        let at: OffsetDateTime = match row.get_value(5).map_err(map_turso)? {
            Value::Text(s) => text_to_dt(&s)?,
            other => {
                return Err(StorageError::Backend(
                    format!("expected TEXT occurred_at, got {other:?}").into(),
                ));
            }
        };
        out.push(ActivityEntry {
            id,
            kind,
            actor,
            subject,
            detail,
            occurred_at: at,
        });
    }
    ItemPage::try_from_lookahead(out, limit, mint_cursor)
}

/// `<occurred_at>|<id>`, matching the `(occurred_at, id)` order.
fn mint_cursor(entry: &ActivityEntry) -> StorageResult<PageCursor> {
    Ok(PageCursor(format!(
        "{}|{}",
        dt_to_text(entry.occurred_at)?,
        entry.id
    )))
}

fn parse_cursor(cursor: &PageCursor) -> StorageResult<(String, i64)> {
    let malformed =
        || StorageError::InvalidInput(format!("malformed activity cursor: {:?}", cursor.0));
    let (at, id) = cursor.0.rsplit_once('|').ok_or_else(malformed)?;
    let at = dt_to_text(text_to_dt(at).map_err(|_| malformed())?)?;
    Ok((at, id.parse().map_err(|_| malformed())?))
}

fn optional_text(row: &turso::Row, idx: usize, name: &str) -> StorageResult<Option<String>> {
    match row.get_value(idx).map_err(map_turso)? {
        Value::Text(s) if s.is_empty() => Ok(None),
        Value::Text(s) => Ok(Some(s)),
        other => Err(StorageError::Backend(
            format!("expected TEXT {name}, got {other:?}").into(),
        )),
    }
}
