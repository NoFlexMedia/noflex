// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;

use noflex_core::{
    ColorPrimaries, DolbyVisionProfile, EditionId, HdrTransfer, MatrixCoefficients, PixelFormat,
    VideoCodec, VideoProfile, WorkId,
};
use noflex_storage::{
    domain::{AudioTrack, Chapter, Edition, IntroMarkers, SubtitleTrack},
    error::{StorageError, StorageResult},
    query::{Page, PageCursor},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int_opt, col_real_opt, col_text, col_text_opt, dt_to_text,
        parse_json, to_json, uuid_to_blob,
    },
    error::{backend_msg, map_turso},
    repo::search::parse_missing_cursor,
};

const UPSERT: &str = "INSERT INTO editions (\
    id, work_id, name, runtime_ms, intro_start_ms, intro_end_ms, credits_start_ms, chapters_json, \
    codec_video, codec_audio, width, height, bit_depth, hdr_format, \
    audio_tracks_json, subtitle_tracks_json, \
    video_codec, video_profile, video_level, pixel_format, color_primaries, \
    transfer_characteristics, matrix_coefficients, dovi_profile, \
    max_cll, max_fall, \
    mastering_primary_r_x, mastering_primary_r_y, \
    mastering_primary_g_x, mastering_primary_g_y, \
    mastering_primary_b_x, mastering_primary_b_y, \
    mastering_white_x, mastering_white_y, \
    mastering_min_lum, mastering_max_lum, \
    frame_rate\
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37) \
ON CONFLICT(work_id, name) DO UPDATE SET \
    work_id = excluded.work_id, \
    name = excluded.name, \
    runtime_ms = excluded.runtime_ms, \
    intro_start_ms = excluded.intro_start_ms, \
    intro_end_ms = excluded.intro_end_ms, \
    credits_start_ms = excluded.credits_start_ms, \
    chapters_json = excluded.chapters_json, \
    codec_video = excluded.codec_video, \
    codec_audio = excluded.codec_audio, \
    width = excluded.width, \
    height = excluded.height, \
    bit_depth = excluded.bit_depth, \
    hdr_format = excluded.hdr_format, \
    audio_tracks_json = excluded.audio_tracks_json, \
    subtitle_tracks_json = excluded.subtitle_tracks_json, \
    video_codec = excluded.video_codec, \
    video_profile = excluded.video_profile, \
    video_level = excluded.video_level, \
    pixel_format = excluded.pixel_format, \
    color_primaries = excluded.color_primaries, \
    transfer_characteristics = excluded.transfer_characteristics, \
    matrix_coefficients = excluded.matrix_coefficients, \
    dovi_profile = excluded.dovi_profile, \
    max_cll = excluded.max_cll, \
    max_fall = excluded.max_fall, \
    mastering_primary_r_x = excluded.mastering_primary_r_x, \
    mastering_primary_r_y = excluded.mastering_primary_r_y, \
    mastering_primary_g_x = excluded.mastering_primary_g_x, \
    mastering_primary_g_y = excluded.mastering_primary_g_y, \
    mastering_primary_b_x = excluded.mastering_primary_b_x, \
    mastering_primary_b_y = excluded.mastering_primary_b_y, \
    mastering_white_x = excluded.mastering_white_x, \
    mastering_white_y = excluded.mastering_white_y, \
    mastering_min_lum = excluded.mastering_min_lum, \
    mastering_max_lum = excluded.mastering_max_lum, \
    frame_rate = excluded.frame_rate \
RETURNING id";

// Same parameter layout as UPSERT. Run first so an existing id keeps its
// identity across a rename or re-parent instead of colliding on the PK.
const UPDATE_BY_ID: &str = "UPDATE editions SET \
    work_id = ?2, \
    name = ?3, \
    runtime_ms = ?4, \
    intro_start_ms = ?5, \
    intro_end_ms = ?6, \
    credits_start_ms = ?7, \
    chapters_json = ?8, \
    codec_video = ?9, \
    codec_audio = ?10, \
    width = ?11, \
    height = ?12, \
    bit_depth = ?13, \
    hdr_format = ?14, \
    audio_tracks_json = ?15, \
    subtitle_tracks_json = ?16, \
    video_codec = ?17, \
    video_profile = ?18, \
    video_level = ?19, \
    pixel_format = ?20, \
    color_primaries = ?21, \
    transfer_characteristics = ?22, \
    matrix_coefficients = ?23, \
    dovi_profile = ?24, \
    max_cll = ?25, \
    max_fall = ?26, \
    mastering_primary_r_x = ?27, \
    mastering_primary_r_y = ?28, \
    mastering_primary_g_x = ?29, \
    mastering_primary_g_y = ?30, \
    mastering_primary_b_x = ?31, \
    mastering_primary_b_y = ?32, \
    mastering_white_x = ?33, \
    mastering_white_y = ?34, \
    mastering_min_lum = ?35, \
    mastering_max_lum = ?36, \
    frame_rate = ?37 \
WHERE id = ?1";

const FOR_WORK: &str = "SELECT id, work_id, name, runtime_ms, intro_start_ms, intro_end_ms, credits_start_ms, \
    chapters_json, codec_video, codec_audio, width, height, bit_depth, hdr_format, \
    audio_tracks_json, subtitle_tracks_json, \
    video_codec, video_profile, video_level, pixel_format, color_primaries, \
    transfer_characteristics, matrix_coefficients, dovi_profile, \
    max_cll, max_fall, \
    mastering_primary_r_x, mastering_primary_r_y, \
    mastering_primary_g_x, mastering_primary_g_y, \
    mastering_primary_b_x, mastering_primary_b_y, \
    mastering_white_x, mastering_white_y, \
    mastering_min_lum, mastering_max_lum, \
    frame_rate \
    FROM editions WHERE work_id = ?1 ORDER BY name";

const GET: &str = "SELECT id, work_id, name, runtime_ms, intro_start_ms, intro_end_ms, credits_start_ms, \
    chapters_json, codec_video, codec_audio, width, height, bit_depth, hdr_format, \
    audio_tracks_json, subtitle_tracks_json, \
    video_codec, video_profile, video_level, pixel_format, color_primaries, \
    transfer_characteristics, matrix_coefficients, dovi_profile, \
    max_cll, max_fall, \
    mastering_primary_r_x, mastering_primary_r_y, \
    mastering_primary_g_x, mastering_primary_g_y, \
    mastering_primary_b_x, mastering_primary_b_y, \
    mastering_white_x, mastering_white_y, \
    mastering_min_lum, mastering_max_lum, \
    frame_rate \
    FROM editions WHERE id = ?1";

const DELETE: &str = "DELETE FROM editions WHERE id = ?1";

// Marker columns only: the sweep worker must not clobber chapters,
// runtime, or probe metadata written by other tasks.
const SET_INTRO_MARKERS: &str = "UPDATE editions SET \
    intro_start_ms = ?1, intro_end_ms = ?2, credits_start_ms = ?3 \
    WHERE id = ?4";

const RECORD_INTRO_SCAN: &str = "INSERT INTO edition_intro_scans (edition_id, plugin_id, scanned_at, found) \
    VALUES (?1, ?2, ?3, ?4) \
    ON CONFLICT(edition_id, plugin_id) DO UPDATE SET \
        scanned_at = excluded.scanned_at, \
        found = excluded.found";

// Anti-join on `edition_intro_scans` for the detection sweep. Column
// list matches GET (e.-prefixed) so `row_to_edition` decoding is
// shared; ordering by `e.id` keeps the keyset cursor stable.
const MISSING_INTRO_SCAN: &str = "SELECT e.id, e.work_id, e.name, e.runtime_ms, e.intro_start_ms, \
    e.intro_end_ms, e.credits_start_ms, e.chapters_json, e.codec_video, e.codec_audio, \
    e.width, e.height, e.bit_depth, e.hdr_format, e.audio_tracks_json, e.subtitle_tracks_json, \
    e.video_codec, e.video_profile, e.video_level, e.pixel_format, e.color_primaries, \
    e.transfer_characteristics, e.matrix_coefficients, e.dovi_profile, \
    e.max_cll, e.max_fall, \
    e.mastering_primary_r_x, e.mastering_primary_r_y, \
    e.mastering_primary_g_x, e.mastering_primary_g_y, \
    e.mastering_primary_b_x, e.mastering_primary_b_y, \
    e.mastering_white_x, e.mastering_white_y, \
    e.mastering_min_lum, e.mastering_max_lum, \
    e.frame_rate \
    FROM editions e \
    WHERE NOT EXISTS (SELECT 1 FROM edition_intro_scans s \
        WHERE s.edition_id = e.id AND s.plugin_id = ?1)";

fn optional_text(s: Option<&String>) -> Value {
    s.map(|v| Value::Text(v.clone())).unwrap_or(Value::Null)
}

fn optional_video_codec(c: Option<VideoCodec>) -> Value {
    c.map(|v| Value::Text(v.as_str().to_string()))
        .unwrap_or(Value::Null)
}

fn optional_video_profile(p: Option<VideoProfile>) -> Value {
    p.map(|v| Value::Text(v.as_str().to_string()))
        .unwrap_or(Value::Null)
}

fn optional_pixel_format(p: Option<PixelFormat>) -> Value {
    p.map(|v| Value::Text(v.as_str().to_string()))
        .unwrap_or(Value::Null)
}

fn optional_color_primaries(p: Option<ColorPrimaries>) -> Value {
    p.map(|v| Value::Text(v.as_str().to_string()))
        .unwrap_or(Value::Null)
}

fn optional_hdr_transfer(t: Option<HdrTransfer>) -> Value {
    t.map(|v| Value::Text(v.as_str().to_string()))
        .unwrap_or(Value::Null)
}

fn optional_matrix_coefficients(m: Option<MatrixCoefficients>) -> Value {
    m.map(|v| Value::Text(v.as_str().to_string()))
        .unwrap_or(Value::Null)
}

fn optional_dovi_profile(p: Option<DolbyVisionProfile>) -> Value {
    p.map(|v| Value::Integer(v.as_u8() as i64))
        .unwrap_or(Value::Null)
}

fn optional_chroma(p: Option<(u16, u16)>) -> (Value, Value) {
    match p {
        Some((x, y)) => (Value::Integer(x as i64), Value::Integer(y as i64)),
        None => (Value::Null, Value::Null),
    }
}

fn optional_tracks(t: &Option<Vec<AudioTrack>>) -> StorageResult<Value> {
    match t {
        Some(v) => Ok(Value::Text(to_json(v)?)),
        None => Ok(Value::Text("[]".to_string())),
    }
}

fn optional_sub_tracks(t: &Option<Vec<SubtitleTrack>>) -> StorageResult<Value> {
    match t {
        Some(v) => Ok(Value::Text(to_json(v)?)),
        None => Ok(Value::Text("[]".to_string())),
    }
}

pub(crate) async fn upsert(conn: &Connection, ed: &Edition) -> StorageResult<EditionId> {
    let (mastering_primary_r_x, mastering_primary_r_y) =
        optional_chroma(ed.mastering_display_primary_r);
    let (mastering_primary_g_x, mastering_primary_g_y) =
        optional_chroma(ed.mastering_display_primary_g);
    let (mastering_primary_b_x, mastering_primary_b_y) =
        optional_chroma(ed.mastering_display_primary_b);
    let (mastering_white_x, mastering_white_y) = optional_chroma(ed.mastering_display_white_point);

    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(ed.id.into_uuid())),
        Value::Blob(uuid_to_blob(ed.work_id.into_uuid())),
        Value::Text(ed.name.clone()),
        ed.runtime_ms.map(Value::Integer).unwrap_or(Value::Null),
        ed.intro_start_ms.map(Value::Integer).unwrap_or(Value::Null),
        ed.intro_end_ms.map(Value::Integer).unwrap_or(Value::Null),
        ed.credits_start_ms
            .map(Value::Integer)
            .unwrap_or(Value::Null),
        Value::Text(to_json(&ed.chapters)?),
        optional_text(ed.codec_video.as_ref()),
        optional_text(ed.codec_audio.as_ref()),
        ed.width
            .map(|v| Value::Integer(v as i64))
            .unwrap_or(Value::Null),
        ed.height
            .map(|v| Value::Integer(v as i64))
            .unwrap_or(Value::Null),
        ed.bit_depth
            .map(|v| Value::Integer(v as i64))
            .unwrap_or(Value::Null),
        optional_text(ed.hdr_format.as_ref()),
        optional_tracks(&ed.audio_tracks)?,
        optional_sub_tracks(&ed.subtitle_tracks)?,
        optional_video_codec(ed.video_codec),
        optional_video_profile(ed.video_profile),
        ed.video_level
            .map(|v| Value::Real(v as f64))
            .unwrap_or(Value::Null),
        optional_pixel_format(ed.pixel_format),
        optional_color_primaries(ed.color_primaries),
        optional_hdr_transfer(ed.transfer_characteristics),
        optional_matrix_coefficients(ed.matrix_coefficients),
        optional_dovi_profile(ed.dovi_profile),
        ed.max_cll
            .map(|v| Value::Integer(v as i64))
            .unwrap_or(Value::Null),
        ed.max_fall
            .map(|v| Value::Integer(v as i64))
            .unwrap_or(Value::Null),
        mastering_primary_r_x,
        mastering_primary_r_y,
        mastering_primary_g_x,
        mastering_primary_g_y,
        mastering_primary_b_x,
        mastering_primary_b_y,
        mastering_white_x,
        mastering_white_y,
        ed.mastering_display_min_luminance
            .map(|v| Value::Integer(v as i64))
            .unwrap_or(Value::Null),
        ed.mastering_display_max_luminance
            .map(|v| Value::Integer(v as i64))
            .unwrap_or(Value::Null),
        ed.frame_rate
            .map(|v| Value::Real(v as f64))
            .unwrap_or(Value::Null),
    ];
    let updated = conn
        .execute(UPDATE_BY_ID, params.clone())
        .await
        .map_err(map_turso)?;
    if updated != 0 {
        return Ok(ed.id);
    }
    let mut rows = conn.query(UPSERT, params).await.map_err(map_turso)?;
    let row = rows
        .next()
        .await
        .map_err(map_turso)?
        .ok_or_else(|| backend_msg("edition upsert returned no id"))?;
    let id = row.get_value(0).map_err(map_turso)?;
    Ok(EditionId::from_uuid(blob_to_uuid(col_blob(&id)?)?))
}

fn read_chroma(x: &Value, y: &Value) -> StorageResult<Option<(u16, u16)>> {
    match (x, y) {
        (Value::Null, _) | (_, Value::Null) => Ok(None),
        (Value::Integer(xi), Value::Integer(yi)) => {
            let xv = i64_to_u16_sat(*xi, "mastering chroma x")?;
            let yv = i64_to_u16_sat(*yi, "mastering chroma y")?;
            Ok(Some((xv, yv)))
        }
        (other_x, other_y) => Err(backend_msg(format!(
            "expected INTEGER or NULL for chroma tuple, got ({other_x:?}, {other_y:?})"
        ))),
    }
}

fn i64_to_u16_sat(v: i64, label: &str) -> StorageResult<u16> {
    if v < 0 || v > u16::MAX as i64 {
        return Err(backend_msg(format!("{label} value {v} out of u16 range")));
    }
    Ok(v as u16)
}

fn read_text_enum<T, F>(v: &Value, parse: F) -> StorageResult<Option<T>>
where
    F: Fn(&str) -> Result<T, String>,
    T: 'static,
{
    match v {
        Value::Null => Ok(None),
        Value::Text(s) => parse(s).map(Some).map_err(backend_msg),
        other => Err(backend_msg(format!(
            "expected TEXT or NULL for enum column, got {other:?}"
        ))),
    }
}

fn read_dovi_profile(v: &Value) -> StorageResult<Option<DolbyVisionProfile>> {
    match v {
        Value::Null => Ok(None),
        Value::Integer(i) => {
            let n = u8::try_from(*i)
                .map_err(|_| backend_msg(format!("dovi_profile value {i} out of u8 range")))?;
            DolbyVisionProfile::try_from(n)
                .map(Some)
                .map_err(backend_msg)
        }
        other => Err(backend_msg(format!(
            "expected INTEGER or NULL for dovi_profile, got {other:?}"
        ))),
    }
}

pub(crate) async fn get(conn: &Connection, id: EditionId) -> StorageResult<Option<Edition>> {
    let mut rows = conn
        .query(GET, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    Ok(Some(row_to_edition(&row)?))
}

pub(crate) async fn get_many(conn: &Connection, ids: &[EditionId]) -> StorageResult<Vec<Edition>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = GET.replace("id = ?1", &format!("id IN ({placeholders})"));
    let params = ids
        .iter()
        .map(|id| Value::Blob(uuid_to_blob(id.into_uuid())))
        .collect::<Vec<_>>();
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut by_id = HashMap::with_capacity(ids.len());
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let edition = row_to_edition(&row)?;
        by_id.insert(edition.id, edition);
    }
    Ok(ids.iter().filter_map(|id| by_id.get(id).cloned()).collect())
}

pub(crate) async fn for_work(conn: &Connection, work: WorkId) -> StorageResult<Vec<Edition>> {
    let mut rows = conn
        .query(FOR_WORK, vec![Value::Blob(uuid_to_blob(work.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_edition(&row)?);
    }
    Ok(out)
}

pub(super) fn row_to_edition(row: &turso::Row) -> StorageResult<Edition> {
    let id = row.get_value(0).map_err(map_turso)?;
    let work_id = row.get_value(1).map_err(map_turso)?;
    let name = row.get_value(2).map_err(map_turso)?;
    let runtime_ms = row.get_value(3).map_err(map_turso)?;
    let intro_start_ms = row.get_value(4).map_err(map_turso)?;
    let intro_end_ms = row.get_value(5).map_err(map_turso)?;
    let credits_start_ms = row.get_value(6).map_err(map_turso)?;
    let chapters_json = row.get_value(7).map_err(map_turso)?;
    let codec_video = row.get_value(8).map_err(map_turso)?;
    let codec_audio = row.get_value(9).map_err(map_turso)?;
    let width = row.get_value(10).map_err(map_turso)?;
    let height = row.get_value(11).map_err(map_turso)?;
    let bit_depth = row.get_value(12).map_err(map_turso)?;
    let hdr_format = row.get_value(13).map_err(map_turso)?;
    let audio_tracks_json = row.get_value(14).map_err(map_turso)?;
    let subtitle_tracks_json = row.get_value(15).map_err(map_turso)?;
    let video_codec = row.get_value(16).map_err(map_turso)?;
    let video_profile = row.get_value(17).map_err(map_turso)?;
    let video_level = row.get_value(18).map_err(map_turso)?;
    let pixel_format = row.get_value(19).map_err(map_turso)?;
    let color_primaries = row.get_value(20).map_err(map_turso)?;
    let transfer_characteristics = row.get_value(21).map_err(map_turso)?;
    let matrix_coefficients = row.get_value(22).map_err(map_turso)?;
    let dovi_profile = row.get_value(23).map_err(map_turso)?;
    let max_cll = row.get_value(24).map_err(map_turso)?;
    let max_fall = row.get_value(25).map_err(map_turso)?;
    let mastering_primary_r_x = row.get_value(26).map_err(map_turso)?;
    let mastering_primary_r_y = row.get_value(27).map_err(map_turso)?;
    let mastering_primary_g_x = row.get_value(28).map_err(map_turso)?;
    let mastering_primary_g_y = row.get_value(29).map_err(map_turso)?;
    let mastering_primary_b_x = row.get_value(30).map_err(map_turso)?;
    let mastering_primary_b_y = row.get_value(31).map_err(map_turso)?;
    let mastering_white_x = row.get_value(32).map_err(map_turso)?;
    let mastering_white_y = row.get_value(33).map_err(map_turso)?;
    let mastering_min_lum = row.get_value(34).map_err(map_turso)?;
    let mastering_max_lum = row.get_value(35).map_err(map_turso)?;
    let frame_rate = row.get_value(36).map_err(map_turso)?;
    let chapters: Vec<Chapter> = parse_json(col_text(&chapters_json)?)?;
    let audio_tracks: Option<Vec<AudioTrack>> = parse_json(col_text(&audio_tracks_json)?)?;
    let subtitle_tracks: Option<Vec<SubtitleTrack>> = parse_json(col_text(&subtitle_tracks_json)?)?;
    let mastering_primary_r = read_chroma(&mastering_primary_r_x, &mastering_primary_r_y)?;
    let mastering_primary_g = read_chroma(&mastering_primary_g_x, &mastering_primary_g_y)?;
    let mastering_primary_b = read_chroma(&mastering_primary_b_x, &mastering_primary_b_y)?;
    let mastering_white_point = read_chroma(&mastering_white_x, &mastering_white_y)?;
    Ok(Edition {
        id: EditionId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        work_id: WorkId::from_uuid(blob_to_uuid(col_blob(&work_id)?)?),
        name: col_text(&name)?.to_string(),
        runtime_ms: col_int_opt(&runtime_ms)?,
        intro_start_ms: col_int_opt(&intro_start_ms)?,
        intro_end_ms: col_int_opt(&intro_end_ms)?,
        credits_start_ms: col_int_opt(&credits_start_ms)?,
        chapters,
        codec_video: col_text_opt(&codec_video)?,
        codec_audio: col_text_opt(&codec_audio)?,
        width: col_int_opt(&width)?.map(|v| v as u32),
        height: col_int_opt(&height)?.map(|v| v as u32),
        bit_depth: col_int_opt(&bit_depth)?.map(|v| v as u32),
        hdr_format: col_text_opt(&hdr_format)?,
        audio_tracks,
        subtitle_tracks,
        video_codec: read_text_enum(&video_codec, |s| s.parse::<VideoCodec>())?,
        video_profile: read_text_enum(&video_profile, |s| s.parse::<VideoProfile>())?,
        video_level: col_real_opt(&video_level)?,
        pixel_format: read_text_enum(&pixel_format, |s| s.parse::<PixelFormat>())?,
        color_primaries: read_text_enum(&color_primaries, |s| s.parse::<ColorPrimaries>())?,
        transfer_characteristics: read_text_enum(&transfer_characteristics, |s| {
            s.parse::<HdrTransfer>()
        })?,
        matrix_coefficients: read_text_enum(&matrix_coefficients, |s| {
            s.parse::<MatrixCoefficients>()
        })?,
        dovi_profile: read_dovi_profile(&dovi_profile)?,
        max_cll: col_int_opt(&max_cll)?.map(|v| v as u32),
        max_fall: col_int_opt(&max_fall)?.map(|v| v as u32),
        mastering_display_primary_r: mastering_primary_r,
        mastering_display_primary_g: mastering_primary_g,
        mastering_display_primary_b: mastering_primary_b,
        mastering_display_white_point: mastering_white_point,
        mastering_display_min_luminance: col_int_opt(&mastering_min_lum)?.map(|v| v as u32),
        mastering_display_max_luminance: col_int_opt(&mastering_max_lum)?.map(|v| v as u32),
        frame_rate: col_real_opt(&frame_rate)?,
    })
}

pub(crate) async fn delete(conn: &Connection, id: EditionId) -> StorageResult<()> {
    conn.execute(DELETE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn set_intro_markers(
    conn: &Connection,
    id: EditionId,
    markers: &IntroMarkers,
) -> StorageResult<()> {
    let changed = conn
        .execute(
            SET_INTRO_MARKERS,
            vec![
                markers
                    .intro_start_ms
                    .map(Value::Integer)
                    .unwrap_or(Value::Null),
                markers
                    .intro_end_ms
                    .map(Value::Integer)
                    .unwrap_or(Value::Null),
                markers
                    .credits_start_ms
                    .map(Value::Integer)
                    .unwrap_or(Value::Null),
                Value::Blob(uuid_to_blob(id.into_uuid())),
            ],
        )
        .await
        .map_err(map_turso)?;
    if changed == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub(crate) async fn record_intro_scan(
    conn: &Connection,
    id: EditionId,
    plugin_id: &str,
    found: bool,
) -> StorageResult<()> {
    let scanned_at = dt_to_text(OffsetDateTime::now_utc())?;
    conn.execute(
        RECORD_INTRO_SCAN,
        vec![
            Value::Blob(uuid_to_blob(id.into_uuid())),
            Value::Text(plugin_id.to_string()),
            Value::Text(scanned_at),
            Value::Integer(i64::from(found)),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn missing_intro_scan(
    conn: &Connection,
    plugin_id: &str,
    limit: u32,
    cursor: Option<&PageCursor>,
) -> StorageResult<Page<Edition>> {
    let mut sql = String::from(MISSING_INTRO_SCAN);
    let mut params: Vec<Value> = vec![Value::Text(plugin_id.to_string())];
    if let Some(c) = cursor {
        let id = parse_missing_cursor(c, "intro-scan")?;
        sql.push_str(&format!(" AND e.id > ?{}", params.len() + 1));
        params.push(Value::Blob(id));
    }
    sql.push_str(" ORDER BY e.id ASC");
    let limit = limit.max(1) as i64;
    sql.push_str(&format!(" LIMIT {}", limit + 1));

    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut items: Vec<Edition> = Vec::new();
    let mut ids: Vec<Vec<u8>> = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = row.get_value(0).map_err(map_turso)?;
        ids.push(col_blob(&id)?.to_vec());
        items.push(row_to_edition(&row)?);
    }
    let next_cursor = if items.len() as i64 > limit {
        // Drop the look-ahead row; the cursor keys off the last row
        // the caller actually receives, not the extra one.
        items.pop();
        ids.pop();
        ids.last().map(|id| {
            let hex: String = id.iter().map(|b| format!("{:02x}", b)).collect();
            PageCursor(hex)
        })
    } else {
        None
    };
    Ok(Page { items, next_cursor })
}
