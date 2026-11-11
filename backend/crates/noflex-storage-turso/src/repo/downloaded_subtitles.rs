// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{DownloadedSubtitleId, MediaFileId, SubtitleCodec, UserId};
use noflex_storage::{
    domain::DownloadedSubtitle,
    error::{StorageError, StorageResult},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_blob_opt, col_int, col_text, col_text_opt, dt_to_text,
        text_to_dt, uuid_to_blob,
    },
    error::{backend_msg, map_turso},
};

const COLUMNS: &str = "id, media_file_id, provider, handle, language, format, release, \
    hearing_impaired, forced, hash_match, file_name, downloaded_by, created_at, last_used_at";

fn row_to_subtitle(row: &turso::Row) -> StorageResult<DownloadedSubtitle> {
    let get = |index| row.get_value(index).map_err(map_turso);
    let format = get(5)?;
    let format = col_text(&format)?;
    Ok(DownloadedSubtitle {
        id: DownloadedSubtitleId::from_uuid(blob_to_uuid(col_blob(&get(0)?)?)?),
        media_file_id: MediaFileId::from_uuid(blob_to_uuid(col_blob(&get(1)?)?)?),
        provider: col_text(&get(2)?)?.to_string(),
        handle: col_text(&get(3)?)?.to_string(),
        language: col_text(&get(4)?)?.to_string(),
        format: format
            .parse::<SubtitleCodec>()
            .map_err(|e| backend_msg(format!("downloaded subtitle format: {e}")))?,
        release: col_text_opt(&get(6)?)?,
        hearing_impaired: col_int(&get(7)?)? != 0,
        forced: col_int(&get(8)?)? != 0,
        hash_match: col_int(&get(9)?)? != 0,
        file_name: col_text(&get(10)?)?.to_string(),
        downloaded_by: col_blob_opt(&get(11)?)?
            .map(|blob| blob_to_uuid(&blob).map(UserId::from_uuid))
            .transpose()?,
        created_at: text_to_dt(col_text(&get(12)?)?)?,
        last_used_at: text_to_dt(col_text(&get(13)?)?)?,
    })
}

async fn collect(mut rows: turso::Rows) -> StorageResult<Vec<DownloadedSubtitle>> {
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_subtitle(&row)?);
    }
    Ok(out)
}

fn id_value(id: DownloadedSubtitleId) -> Value {
    Value::Blob(uuid_to_blob(id.into_uuid()))
}

pub(crate) async fn insert(
    conn: &Connection,
    subtitle: &DownloadedSubtitle,
) -> StorageResult<DownloadedSubtitle> {
    let media = Value::Blob(uuid_to_blob(subtitle.media_file_id.into_uuid()));
    // Concurrent writers never overlap the serial deletes of media files, so
    // a file seen here outlives this insert.
    let mut exists = conn
        .query(
            "SELECT 1 FROM media_files WHERE id = ?1",
            vec![media.clone()],
        )
        .await
        .map_err(map_turso)?;
    if exists.next().await.map_err(map_turso)?.is_none() {
        return Err(StorageError::NotFound);
    }
    let flag = |on: bool| Value::Integer(i64::from(on));
    conn.execute(
        &format!(
            "INSERT INTO downloaded_subtitles ({COLUMNS}) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
             ON CONFLICT(media_file_id, provider, handle) DO NOTHING"
        ),
        vec![
            id_value(subtitle.id),
            media.clone(),
            Value::Text(subtitle.provider.clone()),
            Value::Text(subtitle.handle.clone()),
            Value::Text(subtitle.language.clone()),
            Value::Text(subtitle.format.as_str().to_string()),
            subtitle.release.clone().map_or(Value::Null, Value::Text),
            flag(subtitle.hearing_impaired),
            flag(subtitle.forced),
            flag(subtitle.hash_match),
            Value::Text(subtitle.file_name.clone()),
            subtitle.downloaded_by.map_or(Value::Null, |user| {
                Value::Blob(uuid_to_blob(user.into_uuid()))
            }),
            Value::Text(dt_to_text(subtitle.created_at)?),
            Value::Text(dt_to_text(subtitle.last_used_at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    let mut rows = conn
        .query(
            &format!(
                "SELECT {COLUMNS} FROM downloaded_subtitles \
                 WHERE media_file_id = ?1 AND provider = ?2 AND handle = ?3"
            ),
            vec![
                media,
                Value::Text(subtitle.provider.clone()),
                Value::Text(subtitle.handle.clone()),
            ],
        )
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => row_to_subtitle(&row),
        None => Err(StorageError::NotFound),
    }
}

pub(crate) async fn get(
    conn: &Connection,
    id: DownloadedSubtitleId,
) -> StorageResult<Option<DownloadedSubtitle>> {
    let mut rows = conn
        .query(
            &format!("SELECT {COLUMNS} FROM downloaded_subtitles WHERE id = ?1"),
            vec![id_value(id)],
        )
        .await
        .map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_subtitle(&row))
        .transpose()
}

pub(crate) async fn for_media(
    conn: &Connection,
    media: MediaFileId,
) -> StorageResult<Vec<DownloadedSubtitle>> {
    collect(
        conn.query(
            &format!(
                "SELECT {COLUMNS} FROM downloaded_subtitles WHERE media_file_id = ?1 \
                 ORDER BY created_at, id"
            ),
            vec![Value::Blob(uuid_to_blob(media.into_uuid()))],
        )
        .await
        .map_err(map_turso)?,
    )
    .await
}

pub(crate) async fn delete(
    conn: &Connection,
    id: DownloadedSubtitleId,
) -> StorageResult<Option<DownloadedSubtitle>> {
    let mut rows = conn
        .query(
            &format!("DELETE FROM downloaded_subtitles WHERE id = ?1 RETURNING {COLUMNS}"),
            vec![id_value(id)],
        )
        .await
        .map_err(map_turso)?;
    let removed = rows
        .next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_subtitle(&row))
        .transpose()?;
    // Drain so the statement finishes before the transaction commits.
    while rows.next().await.map_err(map_turso)?.is_some() {}
    Ok(removed)
}

pub(crate) async fn touch(
    conn: &Connection,
    id: DownloadedSubtitleId,
    at: OffsetDateTime,
) -> StorageResult<()> {
    conn.execute(
        "UPDATE downloaded_subtitles SET last_used_at = ?2 WHERE id = ?1 AND last_used_at < ?2",
        vec![id_value(id), Value::Text(dt_to_text(at)?)],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn delete_unused_since(
    conn: &Connection,
    cutoff: OffsetDateTime,
) -> StorageResult<Vec<DownloadedSubtitle>> {
    collect(
        conn.query(
            &format!(
                "DELETE FROM downloaded_subtitles WHERE last_used_at < ?1 RETURNING {COLUMNS}"
            ),
            vec![Value::Text(dt_to_text(cutoff)?)],
        )
        .await
        .map_err(map_turso)?,
    )
    .await
}
