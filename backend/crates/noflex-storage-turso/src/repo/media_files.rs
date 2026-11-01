// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{EditionId, LibraryId, MediaFileId, WorkId};
use noflex_storage::{
    domain::{MediaFile, MediaRemoval},
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, parse_json,
        text_to_dt, to_json, uuid_to_blob,
    },
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO media_files (\
    id, edition_id, path, part_index, size_bytes, mtime, container, probe_json, subtitle_tracks_json\
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
ON CONFLICT(id) DO UPDATE SET \
    edition_id = excluded.edition_id, \
    path = excluded.path, \
    part_index = excluded.part_index, \
    size_bytes = excluded.size_bytes, \
    mtime = excluded.mtime, \
    container = excluded.container, \
    probe_json = excluded.probe_json, \
    subtitle_tracks_json = excluded.subtitle_tracks_json";

const FOR_EDITION: &str = "SELECT id, edition_id, path, part_index, size_bytes, mtime, container, \
    probe_json, subtitle_tracks_json FROM media_files WHERE edition_id = ?1 ORDER BY part_index";

const BY_PATH: &str = "SELECT id, edition_id, path, part_index, size_bytes, mtime, container, \
    probe_json, subtitle_tracks_json FROM media_files WHERE path = ?1";

const BY_ID: &str = "SELECT id, edition_id, path, part_index, size_bytes, mtime, container, \
    probe_json, subtitle_tracks_json FROM media_files WHERE id = ?1";

const FOR_LIBRARY: &str = "SELECT m.id, m.edition_id, m.path, m.part_index, m.size_bytes, \
    m.mtime, m.container, m.probe_json, m.subtitle_tracks_json \
    FROM media_files m \
    JOIN editions e ON e.id = m.edition_id \
    JOIN works w ON w.id = e.work_id \
    WHERE w.library_id = ?1";

const DELETE: &str = "DELETE FROM media_files WHERE id = ?1";

pub(crate) async fn upsert(conn: &Connection, mf: &MediaFile) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(mf.id.into_uuid())),
        Value::Blob(uuid_to_blob(mf.edition_id.into_uuid())),
        Value::Text(mf.path.clone()),
        Value::Integer(mf.part_index as i64),
        Value::Integer(mf.size_bytes as i64),
        Value::Text(dt_to_text(mf.mtime)?),
        mf.container.clone().map(Value::Text).unwrap_or(Value::Null),
        Value::Text(to_json(&mf.probe)?),
        Value::Text(to_json(&mf.subtitle_tracks)?),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn delete(conn: &Connection, id: MediaFileId) -> StorageResult<()> {
    conn.execute(DELETE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    Ok(())
}

/// Runs inside the caller's transaction. Target-polymorphic rows have no
/// foreign keys, so they are removed explicitly before their catalog target.
pub(crate) async fn remove_and_prune(
    conn: &Connection,
    id: MediaFileId,
) -> StorageResult<MediaRemoval> {
    let media_id = uuid_to_blob(id.into_uuid());
    let mut rows = conn
        .query(
            "SELECT e.id, e.work_id FROM media_files m \
             JOIN editions e ON e.id = m.edition_id WHERE m.id = ?1",
            vec![Value::Blob(media_id.clone())],
        )
        .await
        .map_err(map_turso)?;
    let row = rows
        .next()
        .await
        .map_err(map_turso)?
        .ok_or(StorageError::NotFound)?;
    let edition_id = col_blob(&row.get_value(0).map_err(map_turso)?)?.to_vec();
    let work_id = col_blob(&row.get_value(1).map_err(map_turso)?)?.to_vec();
    drop(rows);

    delete_target_rows(conn, "media_file", &media_id).await?;
    conn.execute(DELETE, vec![Value::Blob(media_id)])
        .await
        .map_err(map_turso)?;
    prune_edition_rows(conn, edition_id, work_id).await
}

/// Runs inside the caller's transaction.
pub(crate) async fn prune_edition(
    conn: &Connection,
    edition: EditionId,
) -> StorageResult<MediaRemoval> {
    let edition_id = uuid_to_blob(edition.into_uuid());
    let mut rows = conn
        .query(
            "SELECT work_id FROM editions WHERE id = ?1",
            vec![Value::Blob(edition_id.clone())],
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(MediaRemoval::default());
    };
    let work_id = col_blob(&row.get_value(0).map_err(map_turso)?)?.to_vec();
    drop(rows);
    prune_edition_rows(conn, edition_id, work_id).await
}

/// Deletes the edition unless media files still reference it, then walks
/// up from `work_id` deleting works left with no editions or children.
async fn prune_edition_rows(
    conn: &Connection,
    edition_id: Vec<u8>,
    mut work_id: Vec<u8>,
) -> StorageResult<MediaRemoval> {
    if exists(
        conn,
        "SELECT 1 FROM media_files WHERE edition_id = ?1",
        &edition_id,
    )
    .await?
    {
        return Ok(MediaRemoval::default());
    }
    delete_target_rows(conn, "edition", &edition_id).await?;
    conn.execute(
        "DELETE FROM editions WHERE id = ?1",
        vec![Value::Blob(edition_id)],
    )
    .await
    .map_err(map_turso)?;

    let mut removed_works = Vec::new();
    loop {
        if exists(conn, "SELECT 1 FROM editions WHERE work_id = ?1", &work_id).await?
            || exists(conn, "SELECT 1 FROM works WHERE parent_id = ?1", &work_id).await?
        {
            break;
        }
        let parent = optional_parent(conn, &work_id).await?;
        delete_target_rows(conn, "work", &work_id).await?;
        conn.execute(
            "DELETE FROM works WHERE id = ?1",
            vec![Value::Blob(work_id.clone())],
        )
        .await
        .map_err(map_turso)?;
        removed_works.push(WorkId::from_uuid(blob_to_uuid(&work_id)?));
        let Some(parent) = parent else {
            break;
        };
        work_id = parent;
    }

    Ok(MediaRemoval { removed_works })
}

async fn exists(conn: &Connection, sql: &str, id: &[u8]) -> StorageResult<bool> {
    let mut rows = conn
        .query(sql, vec![Value::Blob(id.to_vec())])
        .await
        .map_err(map_turso)?;
    Ok(rows.next().await.map_err(map_turso)?.is_some())
}

async fn optional_parent(conn: &Connection, work_id: &[u8]) -> StorageResult<Option<Vec<u8>>> {
    let mut rows = conn
        .query(
            "SELECT parent_id FROM works WHERE id = ?1",
            vec![Value::Blob(work_id.to_vec())],
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    match row.get_value(0).map_err(map_turso)? {
        Value::Null => Ok(None),
        Value::Blob(id) => Ok(Some(id)),
        _ => Err(crate::error::backend_msg("work parent_id is not a blob")),
    }
}

async fn delete_target_rows(conn: &Connection, kind: &str, id: &[u8]) -> StorageResult<()> {
    for table in [
        "playlist_items",
        "user_data",
        "tag_assignments",
        "images",
        "embeddings",
    ] {
        let sql = format!("DELETE FROM {table} WHERE target_kind = ?1 AND target_id = ?2");
        conn.execute(
            &sql,
            vec![Value::Text(kind.to_string()), Value::Blob(id.to_vec())],
        )
        .await
        .map_err(map_turso)?;
    }
    Ok(())
}

pub(crate) async fn for_edition(
    conn: &Connection,
    edition: EditionId,
) -> StorageResult<Vec<MediaFile>> {
    let mut rows = conn
        .query(
            FOR_EDITION,
            vec![Value::Blob(uuid_to_blob(edition.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_mf(&row)?);
    }
    Ok(out)
}

pub(crate) async fn by_path(conn: &Connection, path: &str) -> StorageResult<Option<MediaFile>> {
    let mut rows = conn
        .query(BY_PATH, vec![Value::Text(path.to_string())])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_mf(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn by_id(conn: &Connection, id: MediaFileId) -> StorageResult<Option<MediaFile>> {
    let mut rows = conn
        .query(BY_ID, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_mf(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn for_library(
    conn: &Connection,
    library: LibraryId,
) -> StorageResult<Vec<MediaFile>> {
    let mut rows = conn
        .query(
            FOR_LIBRARY,
            vec![Value::Blob(uuid_to_blob(library.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_mf(&row)?);
    }
    Ok(out)
}

fn row_to_mf(row: &turso::Row) -> StorageResult<MediaFile> {
    let id = row.get_value(0).map_err(map_turso)?;
    let edition_id = row.get_value(1).map_err(map_turso)?;
    let path = row.get_value(2).map_err(map_turso)?;
    let part_index = row.get_value(3).map_err(map_turso)?;
    let size_bytes = row.get_value(4).map_err(map_turso)?;
    let mtime = row.get_value(5).map_err(map_turso)?;
    let container = row.get_value(6).map_err(map_turso)?;
    let probe_json = row.get_value(7).map_err(map_turso)?;
    let subtitle_tracks_json = row.get_value(8).map_err(map_turso)?;
    Ok(MediaFile {
        id: MediaFileId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        edition_id: EditionId::from_uuid(blob_to_uuid(col_blob(&edition_id)?)?),
        path: col_text(&path)?.to_string(),
        part_index: col_int(&part_index)? as u32,
        size_bytes: col_int(&size_bytes)? as u64,
        mtime: text_to_dt(col_text(&mtime)?)?,
        container: col_text_opt(&container)?,
        probe: parse_json(col_text(&probe_json)?)?,
        subtitle_tracks: parse_json(col_text(&subtitle_tracks_json)?)?,
    })
}
