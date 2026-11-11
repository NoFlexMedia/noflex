// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{CollectionId, LibraryId, WorkId};
use noflex_storage::{
    RequestUser,
    domain::{Collection, PluginCollectionSync},
    error::{StorageError, StorageResult},
    query::{ITEM_PAGE_MAX, ItemPage, PageCursor},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, text_to_dt,
        uuid_to_blob,
    },
    error::map_turso,
};

const INSERT: &str = "INSERT INTO collections (\
    id, library_id, title, sort_title, overview, created_at, updated_at, plugin_id, plugin_key\
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

const UPDATE: &str = "UPDATE collections SET \
    library_id = ?2, title = ?3, sort_title = ?4, overview = ?5, \
    created_at = ?6, updated_at = ?7, plugin_id = ?8, plugin_key = ?9 \
    WHERE id = ?1";

const COLUMNS: &str =
    "id, library_id, title, sort_title, overview, created_at, updated_at, plugin_id, plugin_key";

const SELECT: &str = "SELECT id, library_id, title, sort_title, overview, created_at, updated_at, \
    plugin_id, plugin_key FROM collections WHERE id = ?1";

const LIST_FOR_LIBRARY: &str = "SELECT id, library_id, title, sort_title, overview, created_at, \
    updated_at, plugin_id, plugin_key \
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
        text_or_null(collection.plugin_id.as_deref()),
        text_or_null(collection.plugin_key.as_deref()),
    ])
}

fn text_or_null(value: Option<&str>) -> Value {
    value
        .map(|value| Value::Text(value.to_string()))
        .unwrap_or(Value::Null)
}

fn row_to_collection(row: &turso::Row) -> StorageResult<Collection> {
    let id = row.get_value(0).map_err(map_turso)?;
    let library_id = row.get_value(1).map_err(map_turso)?;
    let title = row.get_value(2).map_err(map_turso)?;
    let sort_title = row.get_value(3).map_err(map_turso)?;
    let overview = row.get_value(4).map_err(map_turso)?;
    let created_at = row.get_value(5).map_err(map_turso)?;
    let updated_at = row.get_value(6).map_err(map_turso)?;
    let plugin_id = row.get_value(7).map_err(map_turso)?;
    let plugin_key = row.get_value(8).map_err(map_turso)?;
    Ok(Collection {
        id: CollectionId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        library_id: LibraryId::from_uuid(blob_to_uuid(col_blob(&library_id)?)?),
        title: col_text(&title)?.to_string(),
        sort_title: col_text(&sort_title)?.to_string(),
        overview: col_text_opt(&overview)?,
        created_at: text_to_dt(col_text(&created_at)?)?,
        updated_at: text_to_dt(col_text(&updated_at)?)?,
        plugin_id: col_text_opt(&plugin_id)?,
        plugin_key: col_text_opt(&plugin_key)?,
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

const WORK_LIBRARY: &str = "SELECT library_id FROM works WHERE id = ?1";

const DETACHED_LIBRARIES: &str = "SELECT library_id FROM detached_plugin_collections \
    WHERE plugin_id = ?1 AND plugin_key = ?2";

const REMEMBER_DETACHED: &str = "INSERT INTO detached_plugin_collections \
    (plugin_id, plugin_key, library_id) VALUES (?1, ?2, ?3) \
    ON CONFLICT(plugin_id, plugin_key, library_id) DO NOTHING";

const DELETE_PLUGIN_COLLECTIONS: &str = "DELETE FROM collections WHERE plugin_id = ?1";

async fn library_ids(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> StorageResult<Vec<LibraryId>> {
    let mut rows = conn.query(sql, params).await.map_err(map_turso)?;
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        ids.push(LibraryId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(0).map_err(map_turso)?,
        )?)?));
    }
    Ok(ids)
}

async fn owned_by_key(
    conn: &Connection,
    plugin_id: &str,
    key: &str,
) -> StorageResult<Vec<Collection>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM collections WHERE plugin_id = ?1 AND plugin_key = ?2 ORDER BY id"
    );
    let mut rows = conn
        .query(
            &sql,
            vec![
                Value::Text(plugin_id.to_string()),
                Value::Text(key.to_string()),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_collection(&row)?);
    }
    Ok(out)
}

pub(crate) async fn sync_plugin(
    conn: &Connection,
    sync: &PluginCollectionSync,
    now: OffsetDateTime,
) -> StorageResult<Vec<Collection>> {
    let plugin = Value::Text(sync.plugin_id.clone());
    let key = Value::Text(sync.key.clone());
    let detached = library_ids(conn, DETACHED_LIBRARIES, vec![plugin, key]).await?;
    let mut seen = std::collections::HashSet::new();
    let mut by_library: Vec<(LibraryId, Vec<WorkId>)> = Vec::new();
    for work in &sync.works {
        if !seen.insert(*work) {
            continue;
        }
        let library = library_ids(
            conn,
            WORK_LIBRARY,
            vec![Value::Blob(uuid_to_blob(work.into_uuid()))],
        )
        .await?;
        let Some(library) = library.first().copied() else {
            continue;
        };
        if detached.contains(&library) {
            continue;
        }
        match by_library.iter_mut().find(|(id, _)| *id == library) {
            Some((_, works)) => works.push(*work),
            None => by_library.push((library, vec![*work])),
        }
    }
    let existing = owned_by_key(conn, &sync.plugin_id, &sync.key).await?;
    for collection in &existing {
        if !by_library
            .iter()
            .any(|(library, _)| *library == collection.library_id)
        {
            delete(conn, collection.id).await?;
        }
    }
    let mut synced = Vec::with_capacity(by_library.len());
    for (library, works) in by_library {
        let collection = match existing.iter().find(|c| c.library_id == library) {
            Some(current) => {
                let mut collection = current.clone();
                collection.title = sync.title.clone();
                collection.sort_title = sync.title.clone();
                collection.overview = sync.overview.clone();
                if collection != *current {
                    collection.updated_at = now;
                    update(conn, &collection).await?;
                }
                collection
            }
            None => {
                let collection = Collection {
                    id: CollectionId::new(),
                    library_id: library,
                    title: sync.title.clone(),
                    sort_title: sync.title.clone(),
                    overview: sync.overview.clone(),
                    created_at: now,
                    updated_at: now,
                    plugin_id: Some(sync.plugin_id.clone()),
                    plugin_key: Some(sync.key.clone()),
                };
                create(conn, &collection).await?;
                collection
            }
        };
        set_items(conn, collection.id, &works).await?;
        synced.push(collection);
    }
    Ok(synced)
}

pub(crate) async fn detach(
    conn: &Connection,
    id: CollectionId,
    now: OffsetDateTime,
) -> StorageResult<Collection> {
    let mut collection = get(conn, id).await?.ok_or(StorageError::NotFound)?;
    let (Some(plugin), Some(key)) = (collection.plugin_id.take(), collection.plugin_key.take())
    else {
        return Err(StorageError::InvalidInput(
            "collection isn't synced by a plugin".to_string(),
        ));
    };
    conn.execute(
        REMEMBER_DETACHED,
        vec![
            Value::Text(plugin),
            Value::Text(key),
            Value::Blob(uuid_to_blob(collection.library_id.into_uuid())),
        ],
    )
    .await
    .map_err(map_turso)?;
    collection.updated_at = now;
    update(conn, &collection).await?;
    Ok(collection)
}

/// Delete every collection `plugin_id` owns; returns how many.
pub(crate) async fn remove_plugin(conn: &Connection, plugin_id: &str) -> StorageResult<u64> {
    conn.execute(
        DELETE_PLUGIN_COLLECTIONS,
        vec![Value::Text(plugin_id.to_string())],
    )
    .await
    .map_err(map_turso)
}
