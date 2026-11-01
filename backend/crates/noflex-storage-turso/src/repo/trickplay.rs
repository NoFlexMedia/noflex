// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::EditionId;
use noflex_storage::{
    domain::{Edition, TrickplayAsset, TrickplayChapterCue, TrickplaySheet},
    error::StorageResult,
    query::{Page, PageCursor},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, dt_to_text, parse_json, text_to_dt, to_json,
        uuid_to_blob,
    },
    error::map_turso,
    repo::{editions::row_to_edition, search::parse_missing_cursor},
};

const GET: &str = "SELECT edition_id, vtt_path, thumb_width, thumb_height, tile_columns, \
    tile_rows, interval_ms, source_media_mtime, source_fingerprint, generated_at, chapter_cues_json \
    FROM trickplay_assets WHERE edition_id = ?1";

const UPSERT: &str = "INSERT INTO trickplay_assets (edition_id, vtt_path, thumb_width, thumb_height, \
    tile_columns, tile_rows, interval_ms, source_media_mtime, source_fingerprint, generated_at, \
    chapter_cues_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) \
    ON CONFLICT(edition_id) DO UPDATE SET vtt_path=excluded.vtt_path, \
    thumb_width=excluded.thumb_width, thumb_height=excluded.thumb_height, \
    tile_columns=excluded.tile_columns, tile_rows=excluded.tile_rows, \
    interval_ms=excluded.interval_ms, source_media_mtime=excluded.source_media_mtime, \
    source_fingerprint=excluded.source_fingerprint, generated_at=excluded.generated_at, \
    chapter_cues_json=excluded.chapter_cues_json";

// The printf mirrors `noflex_storage::canonical::trickplay_source_fingerprint`;
// `m.mtime` is already stored as `canonical_timestamp` text.
const MISSING: &str = "SELECT e.id, e.work_id, e.name, e.runtime_ms, e.intro_start_ms, \
    e.intro_end_ms, e.credits_start_ms, e.chapters_json, e.codec_video, e.codec_audio, \
    e.width, e.height, e.bit_depth, e.hdr_format, e.audio_tracks_json, e.subtitle_tracks_json, \
    e.video_codec, e.video_profile, e.video_level, e.pixel_format, e.color_primaries, \
    e.transfer_characteristics, e.matrix_coefficients, e.dovi_profile, e.max_cll, e.max_fall, \
    e.mastering_primary_r_x, e.mastering_primary_r_y, e.mastering_primary_g_x, \
    e.mastering_primary_g_y, e.mastering_primary_b_x, e.mastering_primary_b_y, \
    e.mastering_white_x, e.mastering_white_y, e.mastering_min_lum, e.mastering_max_lum, \
    e.frame_rate FROM editions e JOIN media_files m ON m.id = (SELECT pm.id FROM media_files pm \
    WHERE pm.edition_id = e.id ORDER BY pm.part_index ASC, pm.id ASC LIMIT 1) \
    LEFT JOIN trickplay_assets t ON t.edition_id = e.id \
    WHERE (t.edition_id IS NULL OR t.source_media_mtime <> m.mtime \
        OR t.source_fingerprint <> printf('%s\n%d\n%s', m.path, m.size_bytes, m.mtime))";

pub(crate) async fn get(
    conn: &Connection,
    edition: EditionId,
) -> StorageResult<Option<TrickplayAsset>> {
    let id = Value::Blob(uuid_to_blob(edition.into_uuid()));
    let mut rows = conn.query(GET, vec![id.clone()]).await.map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let mut sheet_rows = conn
        .query(
            "SELECT sheet_index, path, tile_count FROM trickplay_sheets \
             WHERE edition_id = ?1 ORDER BY sheet_index",
            vec![id],
        )
        .await
        .map_err(map_turso)?;
    let mut sheets = Vec::new();
    while let Some(sheet) = sheet_rows.next().await.map_err(map_turso)? {
        sheets.push(TrickplaySheet {
            edition_id: edition,
            index: col_int(&sheet.get_value(0).map_err(map_turso)?)? as u32,
            path: col_text(&sheet.get_value(1).map_err(map_turso)?)?.to_string(),
            tile_count: col_int(&sheet.get_value(2).map_err(map_turso)?)? as u32,
        });
    }
    let mut chapter_cues: Vec<TrickplayChapterCue> =
        parse_json(col_text(&row.get_value(10).map_err(map_turso)?)?)?;
    for cue in &mut chapter_cues {
        cue.edition_id = edition;
    }
    Ok(Some(TrickplayAsset {
        edition_id: EditionId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(0).map_err(map_turso)?,
        )?)?),
        vtt_path: col_text(&row.get_value(1).map_err(map_turso)?)?.to_string(),
        thumb_width: col_int(&row.get_value(2).map_err(map_turso)?)? as u32,
        thumb_height: col_int(&row.get_value(3).map_err(map_turso)?)? as u32,
        tile_columns: col_int(&row.get_value(4).map_err(map_turso)?)? as u32,
        tile_rows: col_int(&row.get_value(5).map_err(map_turso)?)? as u32,
        interval_ms: col_int(&row.get_value(6).map_err(map_turso)?)? as u64,
        source_media_mtime: text_to_dt(col_text(&row.get_value(7).map_err(map_turso)?)?)?,
        source_fingerprint: col_text(&row.get_value(8).map_err(map_turso)?)?.to_string(),
        generated_at: text_to_dt(col_text(&row.get_value(9).map_err(map_turso)?)?)?,
        sheets,
        chapter_cues,
    }))
}

pub(crate) async fn upsert(conn: &Connection, asset: &TrickplayAsset) -> StorageResult<()> {
    let id = Value::Blob(uuid_to_blob(asset.edition_id.into_uuid()));
    let asset_params = vec![
        id.clone(),
        Value::Text(asset.vtt_path.clone()),
        Value::Integer(asset.thumb_width as i64),
        Value::Integer(asset.thumb_height as i64),
        Value::Integer(asset.tile_columns as i64),
        Value::Integer(asset.tile_rows as i64),
        Value::Integer(asset.interval_ms as i64),
        Value::Text(dt_to_text(asset.source_media_mtime)?),
        Value::Text(asset.source_fingerprint.clone()),
        Value::Text(dt_to_text(asset.generated_at)?),
        Value::Text(to_json(&asset.chapter_cues)?),
    ];
    conn.execute(UPSERT, asset_params)
        .await
        .map_err(map_turso)?;
    conn.execute(
        "DELETE FROM trickplay_sheets WHERE edition_id = ?1",
        vec![id.clone()],
    )
    .await
    .map_err(map_turso)?;
    for sheet in &asset.sheets {
        conn.execute(
            "INSERT INTO trickplay_sheets (edition_id, sheet_index, path, tile_count) \
                 VALUES (?1, ?2, ?3, ?4)",
            vec![
                id.clone(),
                Value::Integer(sheet.index as i64),
                Value::Text(sheet.path.clone()),
                Value::Integer(sheet.tile_count as i64),
            ],
        )
        .await
        .map_err(map_turso)?;
    }
    Ok(())
}

pub(crate) async fn delete(conn: &Connection, edition: EditionId) -> StorageResult<()> {
    conn.execute(
        "DELETE FROM trickplay_assets WHERE edition_id = ?1",
        vec![Value::Blob(uuid_to_blob(edition.into_uuid()))],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn missing(
    conn: &Connection,
    limit: u32,
    cursor: Option<&PageCursor>,
) -> StorageResult<Page<Edition>> {
    let mut sql = String::from(MISSING);
    let mut params = Vec::new();
    if let Some(cursor) = cursor {
        params.push(Value::Blob(parse_missing_cursor(cursor, "trickplay")?));
        sql.push_str(" AND e.id > ?1");
    }
    sql.push_str(" ORDER BY e.id ASC");
    let limit = limit.max(1) as usize;
    sql.push_str(&format!(" LIMIT {}", limit + 1));
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut items = Vec::new();
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        ids.push(col_blob(&row.get_value(0).map_err(map_turso)?)?.to_vec());
        items.push(row_to_edition(&row)?);
    }
    let next_cursor = if items.len() > limit {
        items.pop();
        ids.pop();
        ids.last().map(|id| PageCursor(hex(id)))
    } else {
        None
    };
    Ok(Page { items, next_cursor })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
