// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{ImageKind, LibraryId, WorkKind};
use noflex_storage::{
    domain::{Image, ImageSource, ImageTarget, Work},
    error::StorageResult,
    query::{Page, PageCursor},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int_opt, col_text, col_text_opt, uuid_to_blob,
        work_kind_to_text,
    },
    error::map_turso,
    repo::{search::parse_missing_cursor, works::row_to_work},
};

const COLUMNS: &str = "target_kind, target_id, kind, path, width, height, language, source";

const UPSERT: &str = "INSERT INTO images \
    (target_kind, target_id, kind, path, width, height, language, source) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
    ON CONFLICT(target_kind, target_id, kind, source) DO UPDATE SET \
        path = excluded.path, \
        width = excluded.width, \
        height = excluded.height, \
        language = excluded.language";

const LIST_FOR_LIBRARY_WORKS: &str = "SELECT i.target_kind, i.target_id, i.kind, i.path, \
    i.width, i.height, i.language, i.source \
    FROM images i JOIN works w ON i.target_id = w.id \
    WHERE i.target_kind = 'work' AND w.library_id = ?1 \
    ORDER BY i.target_id, i.kind, i.source";

const DELETE: &str =
    "DELETE FROM images WHERE target_kind = ?1 AND target_id = ?2 AND kind = ?3 AND source = ?4";

const WORK_COLUMNS: &str = "w.id, w.kind, w.parent_id, w.library_id, w.title, \
    w.sort_title, w.year, w.release_date, w.overview, w.external_ids_json, w.index_number, \
    w.index_end, w.parent_index, w.metadata_status, w.added_at, w.updated_at, w.tagline, \
    w.original_title, w.community_rating, w.locked_fields_json";

/// serde writes ids as `{"kind":"Tmdb","value":…}` without spaces and
/// escapes quotes inside values, so a `"kind":"` that survives removing
/// every `"kind":"Custom"` means some id has another kind.
const HAS_PROVIDER_ID: &str =
    r#"REPLACE(w.external_ids_json, '"kind":"Custom"', '') LIKE '%"kind":"%'"#;

/// `?{first}, …` placeholders for `n` values.
fn placeholders(first: usize, n: usize) -> String {
    (first..first + n)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn source_values(sources: &[ImageSource]) -> impl Iterator<Item = Value> + '_ {
    sources.iter().map(|s| Value::Text(s.as_str().to_string()))
}

fn target_values(target: ImageTarget) -> Vec<Value> {
    vec![
        Value::Text(target.kind_str().to_string()),
        Value::Blob(uuid_to_blob(target.id())),
    ]
}

pub(crate) async fn upsert(conn: &Connection, image: &Image) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Text(image.target.kind_str().to_string()),
        Value::Blob(uuid_to_blob(image.target.id())),
        Value::Text(image.kind.as_str().to_string()),
        Value::Text(image.path.clone()),
        image
            .width
            .map(|w| Value::Integer(w as i64))
            .unwrap_or(Value::Null),
        image
            .height
            .map(|h| Value::Integer(h as i64))
            .unwrap_or(Value::Null),
        image
            .language
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Value::Text(image.source.as_str().to_string()),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn get(
    conn: &Connection,
    target: ImageTarget,
    kind: ImageKind,
    order: &[ImageSource],
) -> StorageResult<Option<Image>> {
    if order.is_empty() {
        return Ok(None);
    }
    let sql = format!(
        "SELECT {COLUMNS} FROM images \
         WHERE target_kind = ?1 AND target_id = ?2 AND kind = ?3 AND source IN ({})",
        placeholders(4, order.len())
    );
    let mut params = target_values(target);
    params.push(Value::Text(kind.as_str().to_string()));
    params.extend(source_values(order));
    let rows = collect(conn, &sql, params).await?;
    Ok(Image::resolve_effective(rows, order).into_iter().next())
}

pub(crate) async fn list(
    conn: &Connection,
    target: ImageTarget,
    order: &[ImageSource],
) -> StorageResult<Vec<Image>> {
    let sql = format!("SELECT {COLUMNS} FROM images WHERE target_kind = ?1 AND target_id = ?2");
    let rows = collect(conn, &sql, target_values(target)).await?;
    Ok(Image::resolve_effective(rows, order))
}

pub(crate) async fn list_for_library_works(
    conn: &Connection,
    library: LibraryId,
) -> StorageResult<Vec<Image>> {
    collect(
        conn,
        LIST_FOR_LIBRARY_WORKS,
        vec![Value::Blob(uuid_to_blob(library.into_uuid()))],
    )
    .await
}

pub(crate) async fn delete(
    conn: &Connection,
    target: ImageTarget,
    kind: ImageKind,
    source: ImageSource,
) -> StorageResult<bool> {
    let mut params = target_values(target);
    params.push(Value::Text(kind.as_str().to_string()));
    params.push(Value::Text(source.as_str().to_string()));
    let changed = conn.execute(DELETE, params).await.map_err(map_turso)?;
    Ok(changed > 0)
}

pub(crate) async fn works_missing(
    conn: &Connection,
    kind: WorkKind,
    image: ImageKind,
    sources: &[ImageSource],
    limit: u32,
    cursor: Option<&PageCursor>,
) -> StorageResult<Page<Work>> {
    let mut params = vec![Value::Text(work_kind_to_text(kind).to_string())];
    let mut sql = format!("SELECT {WORK_COLUMNS} FROM works w WHERE w.kind = ?1");
    // With no sources no row can count, so every work of `kind` is
    // missing (and `IN ()` would be a syntax error).
    if !sources.is_empty() {
        params.push(Value::Text(image.as_str().to_string()));
        params.extend(source_values(sources));
        sql.push_str(&format!(
            " AND NOT EXISTS (SELECT 1 FROM images i \
             WHERE i.target_kind = 'work' AND i.target_id = w.id AND i.kind = ?2 \
             AND i.source IN ({}))",
            placeholders(3, sources.len())
        ));
    }
    if let Some(cursor) = cursor {
        params.push(Value::Blob(parse_missing_cursor(cursor, "image")?));
        sql.push_str(&format!(" AND w.id > ?{}", params.len()));
    }
    page_of_works(conn, sql, params, limit).await
}

pub(crate) async fn identified_works_missing_source(
    conn: &Connection,
    source: ImageSource,
    limit: u32,
    cursor: Option<&PageCursor>,
) -> StorageResult<Page<Work>> {
    let mut params = vec![Value::Text(source.as_str().to_string())];
    let mut sql = format!(
        "SELECT {WORK_COLUMNS} FROM works w \
         WHERE {HAS_PROVIDER_ID} AND NOT EXISTS (SELECT 1 FROM images i \
             WHERE i.target_kind = 'work' AND i.target_id = w.id AND i.source = ?1)"
    );
    if let Some(cursor) = cursor {
        params.push(Value::Blob(parse_missing_cursor(cursor, "image")?));
        sql.push_str(" AND w.id > ?2");
    }
    page_of_works(conn, sql, params, limit).await
}

async fn page_of_works(
    conn: &Connection,
    mut sql: String,
    params: Vec<Value>,
    limit: u32,
) -> StorageResult<Page<Work>> {
    sql.push_str(" ORDER BY w.id ASC");
    let limit = limit.max(1) as usize;
    sql.push_str(&format!(" LIMIT {}", limit + 1));

    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut items = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        items.push(row_to_work(&row)?);
    }
    let next_cursor = if items.len() > limit {
        items.pop();
        items
            .last()
            .map(|w| PageCursor(w.id.into_uuid().simple().to_string()))
    } else {
        None
    };
    Ok(Page { items, next_cursor })
}

async fn collect(conn: &Connection, sql: &str, params: Vec<Value>) -> StorageResult<Vec<Image>> {
    let mut rows = conn.query(sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_image(&row)?);
    }
    Ok(out)
}

fn row_to_image(row: &turso::Row) -> StorageResult<Image> {
    let target_kind = row.get_value(0).map_err(map_turso)?;
    let target_id = row.get_value(1).map_err(map_turso)?;
    let kind = row.get_value(2).map_err(map_turso)?;
    let path = row.get_value(3).map_err(map_turso)?;
    let width = row.get_value(4).map_err(map_turso)?;
    let height = row.get_value(5).map_err(map_turso)?;
    let language = row.get_value(6).map_err(map_turso)?;
    let source = row.get_value(7).map_err(map_turso)?;

    let kind_text = col_text(&kind)?;
    let source_text = col_text(&source)?;
    let target = match col_text(&target_kind)? {
        "work" => ImageTarget::Work(noflex_core::WorkId::from_uuid(blob_to_uuid(col_blob(
            &target_id,
        )?)?)),
        "edition" => ImageTarget::Edition(noflex_core::EditionId::from_uuid(blob_to_uuid(
            col_blob(&target_id)?,
        )?)),
        "tag" => ImageTarget::Tag(noflex_core::TagId::from_uuid(blob_to_uuid(col_blob(
            &target_id,
        )?)?)),
        other => {
            return Err(crate::error::backend_msg(format!(
                "unknown image target_kind {other}"
            )));
        }
    };

    Ok(Image {
        target,
        kind: ImageKind::from_str(kind_text)
            .ok_or_else(|| crate::error::backend_msg(format!("unknown image kind {kind_text}")))?,
        source: ImageSource::from_str(source_text).ok_or_else(|| {
            crate::error::backend_msg(format!("unknown image source {source_text}"))
        })?,
        path: col_text(&path)?.to_string(),
        width: col_int_opt(&width)?.map(|w| w as u32),
        height: col_int_opt(&height)?.map(|h| h as u32),
        language: col_text_opt(&language)?,
    })
}
