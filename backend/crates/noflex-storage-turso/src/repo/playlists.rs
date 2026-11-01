// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{EditionId, PlayTarget, PlaylistId, UserId, WorkId};
use noflex_storage::{
    RequestUser,
    domain::Playlist,
    error::{StorageError, StorageResult},
    query::{ITEM_PAGE_MAX, ItemPage, PageCursor},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, text_to_dt,
        uuid_to_blob,
    },
    error::{backend_msg, map_turso},
};

const INSERT: &str = "INSERT INTO playlists (\
    id, owner_id, title, overview, created_at, updated_at\
) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

const UPDATE: &str = "UPDATE playlists SET \
    owner_id = ?2, title = ?3, overview = ?4, created_at = ?5, updated_at = ?6 \
    WHERE id = ?1";

const SELECT: &str =
    "SELECT id, owner_id, title, overview, created_at, updated_at FROM playlists WHERE id = ?1";

const LIST_FOR_OWNER: &str = "SELECT id, owner_id, title, overview, created_at, updated_at FROM playlists \
    WHERE owner_id = ?1 AND (?2 IS NULL OR title > ?2 OR (title = ?2 AND id > ?3)) \
    ORDER BY title, id LIMIT ?4";

const WORK_EXISTS: &str = "SELECT 1 FROM works WHERE id = ?1";
const EDITION_EXISTS: &str = "SELECT 1 FROM editions WHERE id = ?1";

const DELETE: &str = "DELETE FROM playlists WHERE id = ?1";

const DELETE_ITEMS: &str = "DELETE FROM playlist_items WHERE playlist_id = ?1";

const INSERT_ITEM: &str = "INSERT INTO playlist_items (playlist_id, target_kind, target_id, position) \
    VALUES (?1, ?2, ?3, ?4)";

fn item_sql(select: &str, cursor_clause: &str, limit_clause: &str, enforce_access: bool) -> String {
    let access_clause = if enforce_access {
        let access = crate::repo::restrictions::accessible_work_predicate("w", 5);
        format!(
            "AND EXISTS (SELECT 1 FROM playlists p WHERE p.id = pi.playlist_id AND p.owner_id = ?5) \
             AND {access}"
        )
    } else {
        String::new()
    };
    format!(
        "SELECT {select} FROM playlist_items pi \
         LEFT JOIN editions e ON pi.target_kind = 'edition' AND e.id = pi.target_id \
         JOIN works w ON w.id = CASE WHEN pi.target_kind = 'work' THEN pi.target_id ELSE e.work_id END \
         WHERE pi.playlist_id = ?1 {cursor_clause} {access_clause} {limit_clause}"
    )
}

const APPEND_ITEM: &str = "INSERT OR IGNORE INTO playlist_items \
    (playlist_id, target_kind, target_id, position) \
    SELECT ?1, ?2, ?3, COALESCE(MAX(position), -1) + 1 \
    FROM playlist_items WHERE playlist_id = ?1";

fn encode_cursor(position: i64, target: PlayTarget) -> PageCursor {
    let kind = match target {
        PlayTarget::Work(_) => 'w',
        PlayTarget::Edition(_) => 'e',
    };
    PageCursor(format!(
        "{position:016x}{kind}{}",
        target.id_uuid().simple()
    ))
}

fn decode_cursor(cursor: &PageCursor) -> StorageResult<(i64, &'static str, uuid::Uuid)> {
    if cursor.0.len() != 49 || !cursor.0.is_ascii() {
        return Err(StorageError::InvalidInput(
            "invalid playlist item cursor".to_string(),
        ));
    }
    let position = i64::from_str_radix(&cursor.0[..16], 16)
        .map_err(|_| StorageError::InvalidInput("invalid playlist item cursor".to_string()))?;
    if position < 0 {
        return Err(StorageError::InvalidInput(
            "invalid playlist item cursor".to_string(),
        ));
    }
    let id = uuid::Uuid::parse_str(&cursor.0[17..])
        .map_err(|_| StorageError::InvalidInput("invalid playlist item cursor".to_string()))?;
    let (kind, target) = match &cursor.0[16..17] {
        "w" => ("work", PlayTarget::Work(WorkId::from_uuid(id))),
        "e" => ("edition", PlayTarget::Edition(EditionId::from_uuid(id))),
        _ => {
            return Err(StorageError::InvalidInput(
                "invalid playlist item cursor".to_string(),
            ));
        }
    };
    if encode_cursor(position, target) != *cursor {
        return Err(StorageError::InvalidInput(
            "invalid playlist item cursor".to_string(),
        ));
    }
    Ok((position, kind, id))
}

fn playlist_params(playlist: &Playlist) -> StorageResult<Vec<Value>> {
    Ok(vec![
        Value::Blob(uuid_to_blob(playlist.id.into_uuid())),
        Value::Blob(uuid_to_blob(playlist.owner_id.into_uuid())),
        Value::Text(playlist.title.clone()),
        playlist
            .overview
            .as_ref()
            .map(|s| Value::Text(s.clone()))
            .unwrap_or(Value::Null),
        Value::Text(dt_to_text(playlist.created_at)?),
        Value::Text(dt_to_text(playlist.updated_at)?),
    ])
}

fn row_to_playlist(row: &turso::Row) -> StorageResult<Playlist> {
    let id = row.get_value(0).map_err(map_turso)?;
    let owner_id = row.get_value(1).map_err(map_turso)?;
    let title = row.get_value(2).map_err(map_turso)?;
    let overview = row.get_value(3).map_err(map_turso)?;
    let created_at = row.get_value(4).map_err(map_turso)?;
    let updated_at = row.get_value(5).map_err(map_turso)?;
    Ok(Playlist {
        id: PlaylistId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        owner_id: UserId::from_uuid(blob_to_uuid(col_blob(&owner_id)?)?),
        title: col_text(&title)?.to_string(),
        overview: col_text_opt(&overview)?,
        created_at: text_to_dt(col_text(&created_at)?)?,
        updated_at: text_to_dt(col_text(&updated_at)?)?,
    })
}

pub(crate) async fn create(conn: &Connection, playlist: &Playlist) -> StorageResult<()> {
    conn.execute(INSERT, playlist_params(playlist)?)
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn update(conn: &Connection, playlist: &Playlist) -> StorageResult<()> {
    let changed = conn
        .execute(UPDATE, playlist_params(playlist)?)
        .await
        .map_err(map_turso)?;
    if changed == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub(crate) async fn get(conn: &Connection, id: PlaylistId) -> StorageResult<Option<Playlist>> {
    let mut rows = conn
        .query(SELECT, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    Ok(Some(row_to_playlist(&row)?))
}

pub(crate) async fn list_for_owner(
    conn: &Connection,
    owner: UserId,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<Playlist>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let decoded = cursor.as_ref().map(decode_text_cursor).transpose()?;
    let (title, id) = decoded.unzip();
    let mut rows = conn
        .query(
            LIST_FOR_OWNER,
            vec![
                Value::Blob(uuid_to_blob(owner.into_uuid())),
                title.map(Value::Text).unwrap_or(Value::Null),
                id.map(|id| Value::Blob(uuid_to_blob(id)))
                    .unwrap_or(Value::Null),
                Value::Integer(i64::from(limit) + 1),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut items = Vec::with_capacity(limit as usize + 1);
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        items.push(row_to_playlist(&row)?);
    }
    Ok(ItemPage::from_lookahead(items, limit, |row| {
        encode_text_cursor(&row.title, row.id.into_uuid())
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
        .ok_or_else(|| StorageError::InvalidInput("invalid playlist cursor".to_string()))?;
    if !encoded.is_ascii() || encoded.len() % 2 != 0 || id.len() != 32 {
        return Err(StorageError::InvalidInput(
            "invalid playlist cursor".to_string(),
        ));
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| StorageError::InvalidInput("invalid playlist cursor".to_string()))?;
    let value = String::from_utf8(bytes)
        .map_err(|_| StorageError::InvalidInput("invalid playlist cursor".to_string()))?;
    let id = uuid::Uuid::parse_str(id)
        .map_err(|_| StorageError::InvalidInput("invalid playlist cursor".to_string()))?;
    Ok((value, id))
}

async fn row_exists(conn: &Connection, sql: &str, params: Vec<Value>) -> StorageResult<bool> {
    let mut rows = conn.query(sql, params).await.map_err(map_turso)?;
    Ok(rows.next().await.map_err(map_turso)?.is_some())
}

async fn target_exists(conn: &Connection, target: PlayTarget) -> StorageResult<bool> {
    let sql = match target {
        PlayTarget::Work(_) => WORK_EXISTS,
        PlayTarget::Edition(_) => EDITION_EXISTS,
    };
    row_exists(conn, sql, vec![Value::Blob(uuid_to_blob(target.id_uuid()))]).await
}

pub(crate) async fn delete(conn: &Connection, id: PlaylistId) -> StorageResult<()> {
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
    id: PlaylistId,
    items: &[PlayTarget],
) -> StorageResult<()> {
    if get(conn, id).await?.is_none() {
        return Err(StorageError::NotFound);
    }
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    let unique = items
        .iter()
        .copied()
        .filter(|item| seen.insert(*item))
        .collect::<Vec<_>>();
    for item in &unique {
        if !target_exists(conn, *item).await? {
            return Err(StorageError::InvalidInput(
                "playlist item target does not exist".to_string(),
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
                Value::Text(item.as_str_kind().to_string()),
                Value::Blob(uuid_to_blob(item.id_uuid())),
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
    id: PlaylistId,
    principal: RequestUser,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<PlayTarget>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let decoded = cursor.as_ref().map(decode_cursor).transpose()?;
    let enforce_access = !principal.is_admin;
    let principal_param = || Value::Blob(uuid_to_blob(principal.user_id.into_uuid()));
    let (position, target_kind, target_id) = match decoded {
        Some((position, target_kind, target_id)) => {
            (Some(position), Some(target_kind), Some(target_id))
        }
        None => (None, None, None),
    };
    let limit_param = if enforce_access { 6 } else { 5 };
    let sql = item_sql(
        "pi.target_kind, pi.target_id, pi.position",
        "AND (?2 IS NULL OR pi.position > ?2 OR (pi.position = ?2 AND (pi.target_kind > ?3 OR (pi.target_kind = ?3 AND pi.target_id > ?4))))",
        &format!("ORDER BY pi.position, pi.target_kind, pi.target_id LIMIT ?{limit_param}"),
        enforce_access,
    );
    let mut params = vec![
        Value::Blob(uuid_to_blob(id.into_uuid())),
        position.map(Value::Integer).unwrap_or(Value::Null),
        target_kind
            .map(|kind| Value::Text(kind.to_string()))
            .unwrap_or(Value::Null),
        target_id
            .map(|id| Value::Blob(uuid_to_blob(id)))
            .unwrap_or(Value::Null),
    ];
    if enforce_access {
        params.push(principal_param());
    }
    params.push(Value::Integer(i64::from(limit) + 1));
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::with_capacity(limit as usize + 1);
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let kind = row.get_value(0).map_err(map_turso)?;
        let target_id = row.get_value(1).map_err(map_turso)?;
        let position = row.get_value(2).map_err(map_turso)?;
        let uuid = blob_to_uuid(col_blob(&target_id)?)?;
        let target = match col_text(&kind)? {
            "work" => PlayTarget::Work(WorkId::from_uuid(uuid)),
            "edition" => PlayTarget::Edition(EditionId::from_uuid(uuid)),
            other => return Err(backend_msg(format!("unknown playlist item kind {other}"))),
        };
        out.push((target, col_int(&position)?));
    }
    let has_next = out.len() > limit as usize;
    out.truncate(limit as usize);
    let item_cursors = out
        .iter()
        .map(|(target, position)| encode_cursor(*position, *target))
        .collect::<Vec<_>>();
    let end_cursor = item_cursors.last().cloned();
    Ok(ItemPage {
        items: out.into_iter().map(|(target, _)| target).collect(),
        item_cursors,
        end_cursor,
        has_next,
    })
}

pub(crate) async fn append_items(
    conn: &Connection,
    id: PlaylistId,
    items: &[PlayTarget],
) -> StorageResult<()> {
    if get(conn, id).await?.is_none() {
        return Err(StorageError::NotFound);
    }
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    let unique = items
        .iter()
        .copied()
        .filter(|item| seen.insert(*item))
        .collect::<Vec<_>>();
    for item in &unique {
        if !target_exists(conn, *item).await? {
            return Err(StorageError::InvalidInput(
                "playlist item target does not exist".to_string(),
            ));
        }
    }
    let playlist = Value::Blob(uuid_to_blob(id.into_uuid()));
    for item in unique {
        conn.execute(
            APPEND_ITEM,
            vec![
                playlist.clone(),
                Value::Text(item.as_str_kind().to_string()),
                Value::Blob(uuid_to_blob(item.id_uuid())),
            ],
        )
        .await
        .map_err(map_turso)?;
    }
    Ok(())
}
