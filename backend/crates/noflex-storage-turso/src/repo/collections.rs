// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{CollectionId, LibraryId, WorkId};
use noflex_storage::{
    RequestUser,
    domain::Collection,
    error::{StorageError, StorageResult},
    query::{ITEM_PAGE_MAX, ItemPage, PageCursor},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, text_to_dt,
        uuid_to_blob,
    },
    error::map_turso,
};

const INSERT: &str = "INSERT INTO collections (\
    id, library_id, title, sort_title, overview, created_at, updated_at\
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

const UPDATE: &str = "UPDATE collections SET \
    library_id = ?2, title = ?3, sort_title = ?4, overview = ?5, \
    created_at = ?6, updated_at = ?7 \
    WHERE id = ?1";

const SELECT: &str = "SELECT id, library_id, title, sort_title, overview, created_at, updated_at \
    FROM collections WHERE id = ?1";

const LIST_FOR_LIBRARY: &str = "SELECT id, library_id, title, sort_title, overview, created_at, updated_at \
    FROM collections WHERE library_id = ?1 AND \
        (?2 IS NULL OR sort_title > ?2 OR (sort_title = ?2 AND id > ?3)) \
    ORDER BY sort_title, id LIMIT ?4";

const WORK_IN_LIBRARY_EXISTS: &str = "SELECT 1 FROM works WHERE id = ?1 AND library_id = ?2";

const DELETE: &str = "DELETE FROM collections WHERE id = ?1";

const DELETE_ITEMS: &str = "DELETE FROM collection_items WHERE collection_id = ?1";

const INSERT_ITEM: &str =
    "INSERT INTO collection_items (collection_id, work_id, position) VALUES (?1, ?2, ?3)";

fn item_sql(select: &str, cursor_clause: &str, limit_clause: &str, enforce_access: bool) -> String {
    let access_clause = if enforce_access {
        let access = crate::repo::restrictions::accessible_work_predicate("w", 4);
        format!("AND {access}")
    } else {
        String::new()
    };
    format!(
        "SELECT {select} FROM collection_items ci JOIN works w ON w.id = ci.work_id \
         WHERE ci.collection_id = ?1 {cursor_clause} {access_clause} {limit_clause}"
    )
}

fn encode_cursor(position: i64, id: WorkId) -> PageCursor {
    PageCursor(format!("{position:016x}{}", id.into_uuid().simple()))
}

fn decode_cursor(cursor: &PageCursor) -> StorageResult<(i64, WorkId)> {
    if cursor.0.len() != 48 || !cursor.0.is_ascii() {
        return Err(StorageError::InvalidInput(
            "invalid collection item cursor".to_string(),
        ));
    }
    let position = i64::from_str_radix(&cursor.0[..16], 16)
        .map_err(|_| StorageError::InvalidInput("invalid collection item cursor".to_string()))?;
    if position < 0 {
        return Err(StorageError::InvalidInput(
            "invalid collection item cursor".to_string(),
        ));
    }
    let id = uuid::Uuid::parse_str(&cursor.0[16..])
        .map(WorkId::from_uuid)
        .map_err(|_| StorageError::InvalidInput("invalid collection item cursor".to_string()))?;
    if encode_cursor(position, id) != *cursor {
        return Err(StorageError::InvalidInput(
            "invalid collection item cursor".to_string(),
        ));
    }
    Ok((position, id))
}

fn collection_params(collection: &Collection) -> StorageResult<Vec<Value>> {
    Ok(vec![
        Value::Blob(uuid_to_blob(collection.id.into_uuid())),
        Value::Blob(uuid_to_blob(collection.library_id.into_uuid())),
        Value::Text(collection.title.clone()),
        Value::Text(collection.sort_title.clone()),
        collection
            .overview
            .as_ref()
            .map(|s| Value::Text(s.clone()))
            .unwrap_or(Value::Null),
        Value::Text(dt_to_text(collection.created_at)?),
        Value::Text(dt_to_text(collection.updated_at)?),
    ])
}

fn row_to_collection(row: &turso::Row) -> StorageResult<Collection> {
    let id = row.get_value(0).map_err(map_turso)?;
    let library_id = row.get_value(1).map_err(map_turso)?;
    let title = row.get_value(2).map_err(map_turso)?;
    let sort_title = row.get_value(3).map_err(map_turso)?;
    let overview = row.get_value(4).map_err(map_turso)?;
    let created_at = row.get_value(5).map_err(map_turso)?;
    let updated_at = row.get_value(6).map_err(map_turso)?;
    Ok(Collection {
        id: CollectionId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        library_id: LibraryId::from_uuid(blob_to_uuid(col_blob(&library_id)?)?),
        title: col_text(&title)?.to_string(),
        sort_title: col_text(&sort_title)?.to_string(),
        overview: col_text_opt(&overview)?,
        created_at: text_to_dt(col_text(&created_at)?)?,
        updated_at: text_to_dt(col_text(&updated_at)?)?,
    })
}

pub(crate) async fn create(conn: &Connection, collection: &Collection) -> StorageResult<()> {
    conn.execute(INSERT, collection_params(collection)?)
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn update(conn: &Connection, collection: &Collection) -> StorageResult<()> {
    let changed = conn
        .execute(UPDATE, collection_params(collection)?)
        .await
        .map_err(map_turso)?;
    if changed == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub(crate) async fn get(conn: &Connection, id: CollectionId) -> StorageResult<Option<Collection>> {
    let mut rows = conn
        .query(SELECT, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    Ok(Some(row_to_collection(&row)?))
}

pub(crate) async fn list_for_library(
    conn: &Connection,
    library: LibraryId,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<Collection>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let decoded = cursor.as_ref().map(decode_text_cursor).transpose()?;
    let (sort_title, id) = decoded.unzip();
    let mut rows = conn
        .query(
            LIST_FOR_LIBRARY,
            vec![
                Value::Blob(uuid_to_blob(library.into_uuid())),
                sort_title.map(Value::Text).unwrap_or(Value::Null),
                id.map(|id| Value::Blob(uuid_to_blob(id)))
                    .unwrap_or(Value::Null),
                Value::Integer(i64::from(limit) + 1),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut items = Vec::with_capacity(limit as usize + 1);
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        items.push(row_to_collection(&row)?);
    }
    Ok(ItemPage::from_lookahead(items, limit, |row| {
        encode_text_cursor(&row.sort_title, row.id.into_uuid())
    }))
}

fn encode_text_cursor(value: &str, id: uuid::Uuid) -> PageCursor {
    let value = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    PageCursor(format!("{value}:{}", id.simple()))
}

fn decode_text_cursor(cursor: &PageCursor) -> StorageResult<(String, uuid::Uuid)> {
    let (encoded, id) = cursor
        .0
        .rsplit_once(':')
        .ok_or_else(|| StorageError::InvalidInput("invalid collection cursor".to_string()))?;
    if !encoded.is_ascii() || encoded.len() % 2 != 0 || id.len() != 32 {
        return Err(StorageError::InvalidInput(
            "invalid collection cursor".to_string(),
        ));
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| StorageError::InvalidInput("invalid collection cursor".to_string()))?;
    let value = String::from_utf8(bytes)
        .map_err(|_| StorageError::InvalidInput("invalid collection cursor".to_string()))?;
    let id = uuid::Uuid::parse_str(id)
        .map_err(|_| StorageError::InvalidInput("invalid collection cursor".to_string()))?;
    Ok((value, id))
}

async fn row_exists(conn: &Connection, sql: &str, params: Vec<Value>) -> StorageResult<bool> {
    let mut rows = conn.query(sql, params).await.map_err(map_turso)?;
    Ok(rows.next().await.map_err(map_turso)?.is_some())
}

pub(crate) async fn delete(conn: &Connection, id: CollectionId) -> StorageResult<()> {
    let changed = conn
        .execute(DELETE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    if changed == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub(crate) async fn set_items(
    conn: &Connection,
    id: CollectionId,
    items: &[WorkId],
) -> StorageResult<()> {
    let collection = get(conn, id).await?.ok_or(StorageError::NotFound)?;
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    let unique = items
        .iter()
        .copied()
        .filter(|item| seen.insert(*item))
        .collect::<Vec<_>>();
    for item in &unique {
        if !row_exists(
            conn,
            WORK_IN_LIBRARY_EXISTS,
            vec![
                Value::Blob(uuid_to_blob(item.into_uuid())),
                Value::Blob(uuid_to_blob(collection.library_id.into_uuid())),
            ],
        )
        .await?
        {
            return Err(StorageError::InvalidInput(
                "collection items must belong to the collection library".to_string(),
            ));
        }
    }
    let blob = Value::Blob(uuid_to_blob(id.into_uuid()));
    conn.execute(DELETE_ITEMS, vec![blob.clone()])
        .await
        .map_err(map_turso)?;
    // Dedupe preserving first occurrence; positions rewrite to 0..n.
    for (position, item) in unique.into_iter().enumerate() {
        conn.execute(
            INSERT_ITEM,
            vec![
                blob.clone(),
                Value::Blob(uuid_to_blob(item.into_uuid())),
                Value::Integer(position as i64),
            ],
        )
        .await
        .map_err(map_turso)?;
    }
    Ok(())
}

pub(crate) async fn items(
    conn: &Connection,
    id: CollectionId,
    principal: RequestUser,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<WorkId>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let decoded = cursor.as_ref().map(decode_cursor).transpose()?;
    let enforce_access = !principal.is_admin;
    let principal_param = || Value::Blob(uuid_to_blob(principal.user_id.into_uuid()));
    let (position, work_id) = decoded.unzip();
    let limit_param = if enforce_access { 5 } else { 4 };
    let sql = item_sql(
        "ci.work_id, ci.position",
        "AND (?2 IS NULL OR ci.position > ?2 OR (ci.position = ?2 AND ci.work_id > ?3))",
        &format!("ORDER BY ci.position, ci.work_id LIMIT ?{limit_param}"),
        enforce_access,
    );
    let mut params = vec![
        Value::Blob(uuid_to_blob(id.into_uuid())),
        position.map(Value::Integer).unwrap_or(Value::Null),
        work_id
            .map(|id| Value::Blob(uuid_to_blob(id.into_uuid())))
            .unwrap_or(Value::Null),
    ];
    if enforce_access {
        params.push(principal_param());
    }
    params.push(Value::Integer(i64::from(limit) + 1));
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::with_capacity(limit as usize + 1);
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let work_id = row.get_value(0).map_err(map_turso)?;
        let position = row.get_value(1).map_err(map_turso)?;
        out.push((
            WorkId::from_uuid(blob_to_uuid(col_blob(&work_id)?)?),
            col_int(&position)?,
        ));
    }
    let has_next = out.len() > limit as usize;
    out.truncate(limit as usize);
    let item_cursors = out
        .iter()
        .map(|(id, position)| encode_cursor(*position, *id))
        .collect::<Vec<_>>();
    let end_cursor = item_cursors.last().cloned();
    Ok(ItemPage {
        items: out.into_iter().map(|(id, _)| id).collect(),
        item_cursors,
        end_cursor,
        has_next,
    })
}
