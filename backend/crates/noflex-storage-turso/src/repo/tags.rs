// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{TagId, TagTarget};
use noflex_storage::{
    domain::{Tag, TagAssignment, TagSummary},
    error::{StorageError, StorageResult},
    query::{ItemPage, ListTagsQuery, Page, PageCursor},
};
use turso::{Connection, Value};
use uuid::Uuid;

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_blob_opt, col_int, col_real, col_text, parse_json, to_json,
        uuid_to_blob,
    },
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO tags (id, namespace, slug, display, parent_id, metadata) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
    ON CONFLICT(id) DO UPDATE SET \
        namespace = excluded.namespace, \
        slug = excluded.slug, \
        display = excluded.display, \
        parent_id = excluded.parent_id, \
        metadata = excluded.metadata";

const GET_BY_ID: &str = "SELECT id, namespace, slug, display, parent_id, metadata \
    FROM tags WHERE id = ?1";

const GET_BY_SLUG: &str = "SELECT id, namespace, slug, display, parent_id, metadata \
    FROM tags WHERE namespace = ?1 AND slug = ?2";

const ASSIGN: &str = "INSERT INTO tag_assignments (tag_id, target_kind, target_id, weight, source) \
    VALUES (?1, ?2, ?3, ?4, ?5) \
    ON CONFLICT(tag_id, target_kind, target_id) DO UPDATE SET \
        weight = excluded.weight, \
        source = excluded.source";

const UNASSIGN: &str = "DELETE FROM tag_assignments \
    WHERE tag_id = ?1 AND target_kind = ?2 AND target_id = ?3";

const TAGS_FOR_TARGET: &str = "SELECT t.id, t.namespace, t.slug, t.display, t.parent_id, t.metadata \
    FROM tags t JOIN tag_assignments a ON a.tag_id = t.id \
    WHERE a.target_kind = ?1 AND a.target_id = ?2";

const ASSIGNMENTS_FOR_TARGET: &str = "SELECT t.id, t.namespace, t.slug, t.display, t.parent_id, t.metadata, \
    a.weight, a.source \
    FROM tags t JOIN tag_assignments a ON a.tag_id = t.id \
    WHERE a.target_kind = ?1 AND a.target_id = ?2";

const SEARCH_BY_TAGS: &str = "SELECT w.id FROM works w \
    WHERE w.id IN ( \
        SELECT target_id FROM tag_assignments \
        WHERE target_kind = 'work' AND tag_id = ?1 \
    ) \
    ORDER BY w.added_at DESC LIMIT ?2";

// The work count is a scalar subquery rather than a LEFT JOIN + GROUP
// BY: same zero-count rows, no grouping of the tag columns. `LOWER()`
// is selected alongside so the page cursor carries the exact
// case-folded key SQLite compared against (Rust's `to_lowercase` folds
// more than SQLite's ASCII-only `LOWER`).
const LIST: &str = "SELECT t.id, t.namespace, t.slug, t.display, t.parent_id, t.metadata, \
    (SELECT COUNT(*) FROM tag_assignments a \
        WHERE a.tag_id = t.id AND a.target_kind = 'work'), \
    LOWER(t.display) \
    FROM tags t";

pub(crate) async fn upsert(conn: &Connection, tag: &Tag) -> StorageResult<TagId> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(tag.id.into_uuid())),
        Value::Text(tag.namespace.clone()),
        Value::Text(tag.slug.clone()),
        Value::Text(tag.display.clone()),
        match tag.parent_id {
            Some(p) => Value::Blob(uuid_to_blob(p.into_uuid())),
            None => Value::Null,
        },
        Value::Text(to_json(&tag.metadata)?),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(tag.id)
}

pub(crate) async fn get_by_id(conn: &Connection, id: TagId) -> StorageResult<Option<Tag>> {
    let mut rows = conn
        .query(GET_BY_ID, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_tag(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_by_slug(
    conn: &Connection,
    namespace: &str,
    slug: &str,
) -> StorageResult<Option<Tag>> {
    let mut rows = conn
        .query(
            GET_BY_SLUG,
            vec![
                Value::Text(namespace.to_string()),
                Value::Text(slug.to_string()),
            ],
        )
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_tag(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn assign(
    conn: &Connection,
    tag: TagId,
    target: TagTarget,
    weight: f32,
    source: &str,
) -> StorageResult<()> {
    let (kind, id) = tag_target_parts(&target);
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(tag.into_uuid())),
        Value::Text(kind.to_string()),
        Value::Blob(uuid_to_blob(id)),
        Value::Real(weight as f64),
        Value::Text(source.to_string()),
    ];
    conn.execute(ASSIGN, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn unassign(
    conn: &Connection,
    tag: TagId,
    target: TagTarget,
) -> StorageResult<()> {
    let (kind, id) = tag_target_parts(&target);
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(tag.into_uuid())),
        Value::Text(kind.to_string()),
        Value::Blob(uuid_to_blob(id)),
    ];
    conn.execute(UNASSIGN, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn tags_for_target(
    conn: &Connection,
    target: TagTarget,
) -> StorageResult<Vec<Tag>> {
    let (kind, id) = tag_target_parts(&target);
    let mut rows = conn
        .query(
            TAGS_FOR_TARGET,
            vec![Value::Text(kind.to_string()), Value::Blob(uuid_to_blob(id))],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_tag(&row)?);
    }
    Ok(out)
}

pub(crate) async fn assignments_for_target(
    conn: &Connection,
    target: TagTarget,
) -> StorageResult<Vec<TagAssignment>> {
    let (kind, id) = tag_target_parts(&target);
    let mut rows = conn
        .query(
            ASSIGNMENTS_FOR_TARGET,
            vec![Value::Text(kind.to_string()), Value::Blob(uuid_to_blob(id))],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let weight = row.get_value(6).map_err(map_turso)?;
        let source = row.get_value(7).map_err(map_turso)?;
        out.push(TagAssignment {
            tag: row_to_tag(&row)?,
            weight: col_real(&weight)?,
            source: col_text(&source)?.to_string(),
        });
    }
    Ok(out)
}

pub(crate) async fn list(
    conn: &Connection,
    q: &ListTagsQuery,
) -> StorageResult<ItemPage<TagSummary>> {
    let mut sql = String::from(LIST);
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if !q.namespaces.is_empty() {
        let placeholders: Vec<String> = q
            .namespaces
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", params.len() + 1 + i))
            .collect();
        for ns in &q.namespaces {
            params.push(Value::Text(ns.clone()));
        }
        where_clauses.push(format!("t.namespace IN ({})", placeholders.join(",")));
    }
    if let Some(cursor) = &q.cursor {
        let (display, id) = parse_list_cursor(cursor)?;
        where_clauses.push(format!(
            "(LOWER(t.display), t.id) > (?{}, ?{})",
            params.len() + 1,
            params.len() + 2
        ));
        params.push(Value::Text(display));
        params.push(Value::Blob(id));
    }

    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY LOWER(t.display) ASC, t.id ASC");

    let limit = q.limit.max(1) as i64;
    sql.push_str(&format!(" LIMIT {}", limit + 1));

    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut rows_out: Vec<(String, TagSummary)> = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let count = row.get_value(6).map_err(map_turso)?;
        let lowered = row.get_value(7).map_err(map_turso)?;
        let tag = row_to_tag(&row)?;
        rows_out.push((
            col_text(&lowered)?.to_string(),
            TagSummary {
                tag,
                work_count: col_int(&count)? as u32,
            },
        ));
    }
    let page = ItemPage::from_lookahead(rows_out, limit as u32, |(display, summary)| {
        let hex: String = uuid_to_blob(summary.tag.id.into_uuid())
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        PageCursor(format!("{display}|{hex}"))
    });
    Ok(page.map(|(_, summary)| summary))
}

/// Cursors are client-supplied: anything that isn't
/// `<lowered display>|<32 hex chars>` is rejected outright, because
/// silently dropping a bad cursor would re-serve page one.
fn parse_list_cursor(c: &PageCursor) -> StorageResult<(String, Vec<u8>)> {
    let malformed = || StorageError::InvalidInput(format!("malformed tag cursor: {:?}", c.0));
    // `rsplit_once` — a `|` inside the display name must not split the key.
    let (display, hex) = c.0.rsplit_once('|').ok_or_else(malformed)?;
    if hex.len() != 32 || !hex.is_ascii() {
        return Err(malformed());
    }
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| malformed()))
        .collect::<StorageResult<Vec<u8>>>()?;
    Ok((display.to_string(), bytes))
}

pub(crate) async fn search_works(
    conn: &Connection,
    tag: TagId,
    limit: u32,
) -> StorageResult<Page<Uuid>> {
    let mut rows = conn
        .query(
            SEARCH_BY_TAGS,
            vec![
                Value::Blob(uuid_to_blob(tag.into_uuid())),
                Value::Integer(limit as i64),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut items = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = row.get_value(0).map_err(map_turso)?;
        items.push(blob_to_uuid(col_blob(&id)?)?);
    }
    Ok(Page {
        items,
        next_cursor: None,
    })
}

fn tag_target_parts(target: &TagTarget) -> (&'static str, Uuid) {
    match target {
        TagTarget::Work(id) => ("work", id.into_uuid()),
        TagTarget::Edition(id) => ("edition", id.into_uuid()),
        TagTarget::MediaFile(id) => ("media_file", id.into_uuid()),
    }
}

fn row_to_tag(row: &turso::Row) -> StorageResult<Tag> {
    let id = row.get_value(0).map_err(map_turso)?;
    let namespace = row.get_value(1).map_err(map_turso)?;
    let slug = row.get_value(2).map_err(map_turso)?;
    let display = row.get_value(3).map_err(map_turso)?;
    let parent_id = row.get_value(4).map_err(map_turso)?;
    let metadata = row.get_value(5).map_err(map_turso)?;

    let parent = col_blob_opt(&parent_id)?
        .map(|b| blob_to_uuid(&b).map(TagId::from_uuid))
        .transpose()?;

    Ok(Tag {
        id: TagId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        namespace: col_text(&namespace)?.to_string(),
        slug: col_text(&slug)?.to_string(),
        display: col_text(&display)?.to_string(),
        parent_id: parent,
        metadata: parse_json(col_text(&metadata)?)?,
    })
}
