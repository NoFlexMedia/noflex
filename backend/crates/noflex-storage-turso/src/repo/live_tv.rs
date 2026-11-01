// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{
    ChannelId, EpgProgramId, LiveTvSourceId, RecordingId, RecordingScheduleId, UserId,
};
use noflex_storage::{
    domain::{
        EpgProgram, LiveTvChannel, LiveTvSource, Recording, RecordingSchedule,
        RecordingScheduleStatus, RecordingStatus,
    },
    error::{StorageError, StorageResult},
    query::{ITEM_PAGE_MAX, ItemPage, PageCursor},
};
use time::OffsetDateTime;
use turso::{Connection, Value};
use uuid::Uuid;

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_blob_opt, col_int, col_int_opt, col_text, col_text_opt,
        dt_to_text, text_to_dt, uuid_to_blob,
    },
    error::map_turso,
};

trait IntoUuid {
    fn into_uuid_value(self) -> Uuid;
}
macro_rules! impl_into_uuid { ($($ty:ty),+ $(,)?) => { $(impl IntoUuid for $ty { fn into_uuid_value(self) -> Uuid { self.into_uuid() } })+ }; }
impl_into_uuid!(
    LiveTvSourceId,
    ChannelId,
    EpgProgramId,
    RecordingScheduleId,
    RecordingId,
    UserId
);
fn blob(id: impl IntoUuid) -> Value {
    Value::Blob(uuid_to_blob(id.into_uuid_value()))
}
fn opt_text(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |v| Value::Text(v.into()))
}
fn opt_dt(value: Option<OffsetDateTime>) -> StorageResult<Value> {
    Ok(value
        .map(|v| dt_to_text(v).map(Value::Text))
        .transpose()?
        .unwrap_or(Value::Null))
}
fn cursor_uuid(cursor: Option<PageCursor>) -> StorageResult<Option<Uuid>> {
    cursor
        .map(|c| {
            Uuid::parse_str(&c.0)
                .map_err(|_| StorageError::InvalidInput("invalid live TV cursor".into()))
        })
        .transpose()
}
fn id_cursor(id: impl IntoUuid) -> PageCursor {
    PageCursor(id.into_uuid_value().to_string())
}
/// Guide cursors are `<start_at>|<program uuid>`, matching the
/// `(start_at, id)` order.
fn guide_cursor(program: &EpgProgram) -> StorageResult<PageCursor> {
    Ok(PageCursor(format!(
        "{}|{}",
        dt_to_text(program.start_at)?,
        program.id.into_uuid()
    )))
}
fn parse_guide_cursor(cursor: &PageCursor) -> StorageResult<(OffsetDateTime, Uuid)> {
    let malformed = || StorageError::InvalidInput("invalid live TV guide cursor".into());
    let (at, id) = cursor.0.rsplit_once('|').ok_or_else(malformed)?;
    Ok((
        text_to_dt(at).map_err(|_| malformed())?,
        Uuid::parse_str(id).map_err(|_| malformed())?,
    ))
}

pub(crate) async fn upsert_source(conn: &Connection, source: &LiveTvSource) -> StorageResult<()> {
    conn.execute("INSERT INTO live_tv_sources(id,name,enabled,playlist_url,epg_url,last_refresh_at,last_error) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,enabled=excluded.enabled,playlist_url=excluded.playlist_url,epg_url=excluded.epg_url,last_refresh_at=excluded.last_refresh_at,last_error=excluded.last_error", vec![blob(source.id),Value::Text(source.name.clone()),Value::Integer(source.enabled.into()),Value::Text(source.playlist_url.clone()),opt_text(source.epg_url.as_deref()),opt_dt(source.last_refresh_at)?,opt_text(source.last_error.as_deref())]).await.map_err(map_turso)?;
    Ok(())
}
pub(crate) async fn get_source(
    conn: &Connection,
    id: LiveTvSourceId,
) -> StorageResult<Option<LiveTvSource>> {
    let mut rows=conn.query("SELECT id,name,enabled,playlist_url,epg_url,last_refresh_at,last_error FROM live_tv_sources WHERE id=?1",vec![blob(id)]).await.map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|r| source_row(&r))
        .transpose()
}
pub(crate) async fn list_sources(conn: &Connection) -> StorageResult<Vec<LiveTvSource>> {
    let mut rows=conn.query("SELECT id,name,enabled,playlist_url,epg_url,last_refresh_at,last_error FROM live_tv_sources ORDER BY name,id",()).await.map_err(map_turso)?;
    let mut out = vec![];
    while let Some(r) = rows.next().await.map_err(map_turso)? {
        out.push(source_row(&r)?)
    }
    Ok(out)
}
pub(crate) async fn delete_source(conn: &Connection, id: LiveTvSourceId) -> StorageResult<()> {
    conn.execute("DELETE FROM live_tv_sources WHERE id=?1", vec![blob(id)])
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn upsert_channel(conn: &Connection, c: &LiveTvChannel) -> StorageResult<()> {
    conn.execute("INSERT INTO live_tv_channels(id,source_id,external_id,name,number,logo_url,channel_group,enabled,stream_url) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(source_id,external_id) DO UPDATE SET name=excluded.name,number=excluded.number,logo_url=excluded.logo_url,channel_group=excluded.channel_group,enabled=excluded.enabled,stream_url=excluded.stream_url",vec![blob(c.id),blob(c.source_id),Value::Text(c.external_id.clone()),Value::Text(c.name.clone()),opt_text(c.number.as_deref()),opt_text(c.logo_url.as_deref()),opt_text(c.group.as_deref()),Value::Integer(c.enabled.into()),Value::Text(c.stream_url.clone())]).await.map_err(map_turso)?;
    Ok(())
}
pub(crate) async fn get_channel(
    conn: &Connection,
    id: ChannelId,
) -> StorageResult<Option<LiveTvChannel>> {
    let mut rows=conn.query("SELECT id,source_id,external_id,name,number,logo_url,channel_group,enabled,stream_url FROM live_tv_channels WHERE id=?1",vec![blob(id)]).await.map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|r| channel_row(&r))
        .transpose()
}
pub(crate) async fn list_channels(
    conn: &Connection,
    source: Option<LiveTvSourceId>,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<LiveTvChannel>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let after = cursor_uuid(cursor)?.map_or(Value::Null, |v| Value::Blob(uuid_to_blob(v)));
    let src = source.map_or(Value::Null, blob);
    let mut rows=conn.query("SELECT id,source_id,external_id,name,number,logo_url,channel_group,enabled,stream_url FROM live_tv_channels WHERE enabled=1 AND (?1 IS NULL OR source_id=?1) AND (?2 IS NULL OR id>?2) ORDER BY id LIMIT ?3",vec![src,after,Value::Integer(i64::from(limit.saturating_add(1))) ]).await.map_err(map_turso)?;
    let mut items = vec![];
    while let Some(r) = rows.next().await.map_err(map_turso)? {
        items.push(channel_row(&r)?)
    }
    Ok(ItemPage::from_lookahead(items, limit, |v| id_cursor(v.id)))
}
pub(crate) async fn apply_refresh(
    conn: &Connection,
    source: LiveTvSourceId,
    channels: &[LiveTvChannel],
    programs: &[EpgProgram],
    refreshed_at: OffsetDateTime,
) -> StorageResult<()> {
    conn.execute(
        "UPDATE live_tv_channels SET enabled=0 WHERE source_id=?1",
        vec![blob(source)],
    )
    .await
    .map_err(map_turso)?;
    let mut resolved = std::collections::HashMap::new();
    for channel in channels {
        upsert_channel(conn, channel).await?;
        let mut rows = conn
            .query(
                "SELECT id FROM live_tv_channels WHERE source_id=?1 AND external_id=?2",
                vec![blob(source), Value::Text(channel.external_id.clone())],
            )
            .await
            .map_err(map_turso)?;
        let row = rows
            .next()
            .await
            .map_err(map_turso)?
            .ok_or(StorageError::NotFound)?;
        let value = row.get_value(0).map_err(map_turso)?;
        resolved.insert(
            channel.id,
            ChannelId::from_uuid(blob_to_uuid(col_blob(&value)?)?),
        );
    }
    // Every row this refresh writes carries its stamp; rows still holding
    // an older stamp were absent from the feed and are swept below. This
    // keeps the delete O(1) in SQL size regardless of feed length.
    let stamp = Value::Blob(uuid_to_blob(Uuid::now_v7()));
    for program in programs {
        let channel_id = resolved.get(&program.channel_id).copied().ok_or_else(|| {
            StorageError::InvalidInput("program references unknown channel".into())
        })?;
        conn.execute(
                "INSERT INTO epg_programs(id,channel_id,external_id,title,subtitle,description,category,start_at,end_at,refresh_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(channel_id,external_id,start_at) DO UPDATE SET title=excluded.title,subtitle=excluded.subtitle,description=excluded.description,category=excluded.category,end_at=excluded.end_at,refresh_id=excluded.refresh_id",
                vec![blob(program.id),blob(channel_id),Value::Text(program.external_id.clone()),Value::Text(program.title.clone()),opt_text(program.subtitle.as_deref()),opt_text(program.description.as_deref()),opt_text(program.category.as_deref()),Value::Text(dt_to_text(program.start_at)?),Value::Text(dt_to_text(program.end_at)?),stamp.clone()],
            ).await.map_err(map_turso)?;
    }
    conn.execute(
        "DELETE FROM epg_programs WHERE channel_id IN (SELECT id FROM live_tv_channels WHERE source_id=?1) AND (refresh_id IS NULL OR refresh_id<>?2)",
        vec![blob(source), stamp],
    )
    .await
    .map_err(map_turso)?;
    let changed = conn
        .execute(
            "UPDATE live_tv_sources SET last_refresh_at=?2,last_error=NULL WHERE id=?1",
            vec![blob(source), Value::Text(dt_to_text(refreshed_at)?)],
        )
        .await
        .map_err(map_turso)?;
    if changed == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub(crate) async fn record_refresh_error(
    conn: &Connection,
    source: LiveTvSourceId,
    error: &str,
    attempted_at: OffsetDateTime,
) -> StorageResult<()> {
    let at = dt_to_text(attempted_at)?;
    conn.execute(
        "UPDATE live_tv_sources SET last_refresh_at=?2,last_error=?3 WHERE id=?1 AND (last_refresh_at IS NULL OR last_refresh_at<=?2)",
        vec![blob(source), Value::Text(at), Value::Text(error.into())],
    ).await.map_err(map_turso)?;
    Ok(())
}

/// Bound on bound parameters per guide query; SQLite's default limit is
/// far higher, but long IN lists also slow planning.
const GUIDE_CHUNK: usize = 500;

pub(crate) async fn guide(
    conn: &Connection,
    channels: &[ChannelId],
    start: OffsetDateTime,
    end: OffsetDateTime,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<EpgProgram>> {
    let limit = limit.max(1);
    let after = cursor.as_ref().map(parse_guide_cursor).transpose()?;
    let mut unique = channels.to_vec();
    unique.sort_unstable_by_key(|id| id.into_uuid());
    unique.dedup();
    let start = Value::Text(dt_to_text(start)?);
    let end = Value::Text(dt_to_text(end)?);
    let (after_at, after_id) = match after {
        Some((at, id)) => (Value::Text(dt_to_text(at)?), Value::Blob(uuid_to_blob(id))),
        None => (Value::Null, Value::Null),
    };
    let fetch = i64::from(limit) + 1;
    let mut out = vec![];
    // Each chunk yields its own first `limit + 1` rows by (start_at, id)
    // past the cursor, so the merged top `limit + 1` is exact.
    for chunk in unique.chunks(GUIDE_CHUNK) {
        let placeholders = (0..chunk.len())
            .map(|i| format!("?{}", i + 6))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id,channel_id,external_id,title,subtitle,description,category,start_at,end_at FROM epg_programs WHERE start_at<?2 AND end_at>?1 AND (?4 IS NULL OR (start_at,id)>(?4,?5)) AND channel_id IN ({placeholders}) ORDER BY start_at,id LIMIT ?3"
        );
        let mut params = Vec::with_capacity(chunk.len() + 5);
        params.extend([
            start.clone(),
            end.clone(),
            Value::Integer(fetch),
            after_at.clone(),
            after_id.clone(),
        ]);
        params.extend(chunk.iter().map(|id| blob(*id)));
        let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
        while let Some(r) = rows.next().await.map_err(map_turso)? {
            out.push(program_row(&r)?)
        }
    }
    out.sort_by_key(|p| (p.start_at, p.id.into_uuid()));
    ItemPage::try_from_lookahead(out, limit, guide_cursor)
}
pub(crate) async fn get_program(
    conn: &Connection,
    id: EpgProgramId,
) -> StorageResult<Option<EpgProgram>> {
    let mut rows=conn.query("SELECT id,channel_id,external_id,title,subtitle,description,category,start_at,end_at FROM epg_programs WHERE id=?1",vec![blob(id)]).await.map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|r| program_row(&r))
        .transpose()
}

pub(crate) async fn create_schedule(conn: &Connection, s: &RecordingSchedule) -> StorageResult<()> {
    conn.execute("INSERT INTO recording_schedules(id,owner_id,channel_id,program_id,start_at,end_at,padding_before_secs,padding_after_secs,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",vec![blob(s.id),blob(s.owner_id),blob(s.channel_id),s.program_id.map_or(Value::Null,blob),Value::Text(dt_to_text(s.start_at)?),Value::Text(dt_to_text(s.end_at)?),Value::Integer(i64::from(s.padding_before_secs)),Value::Integer(i64::from(s.padding_after_secs)),Value::Text(schedule_status(s.status).into()),Value::Text(dt_to_text(s.created_at)?),Value::Text(dt_to_text(s.updated_at)?)]).await.map_err(map_turso)?;
    Ok(())
}
pub(crate) async fn get_schedule(
    conn: &Connection,
    id: RecordingScheduleId,
) -> StorageResult<Option<RecordingSchedule>> {
    let mut rows = conn
        .query(&format!("{} WHERE id=?1", SCHEDULE_SELECT), vec![blob(id)])
        .await
        .map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|r| schedule_row(&r))
        .transpose()
}
pub(crate) async fn list_schedules(
    conn: &Connection,
    owner: UserId,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<RecordingSchedule>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let before = cursor_uuid(cursor)?.map_or(Value::Null, |v| Value::Blob(uuid_to_blob(v)));
    let mut rows = conn
        .query(
            &format!(
                "{} WHERE owner_id=?1 AND (?2 IS NULL OR id<?2) ORDER BY id DESC LIMIT ?3",
                SCHEDULE_SELECT
            ),
            vec![
                blob(owner),
                before,
                Value::Integer(i64::from(limit.saturating_add(1))),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut items = vec![];
    while let Some(r) = rows.next().await.map_err(map_turso)? {
        items.push(schedule_row(&r)?)
    }
    Ok(ItemPage::from_lookahead(items, limit, |v| id_cursor(v.id)))
}
pub(crate) async fn cancel_schedule(
    conn: &Connection,
    id: RecordingScheduleId,
    owner: UserId,
    now: OffsetDateTime,
) -> StorageResult<Option<RecordingSchedule>> {
    let at = Value::Text(dt_to_text(now)?);
    let scheduled = conn.execute(
            "UPDATE recording_schedules SET status='cancelled',updated_at=?3 WHERE id=?1 AND owner_id=?2 AND status='scheduled'",
            vec![blob(id), blob(owner), at.clone()],
        ).await.map_err(map_turso)?;
    if scheduled == 0 {
        let recording = conn.execute(
                "UPDATE recordings SET status='cancelled',completed_at=?3,updated_at=?3 WHERE schedule_id=?1 AND owner_id=?2 AND status='running'",
                vec![blob(id), blob(owner), at.clone()],
            ).await.map_err(map_turso)?;
        if recording == 0 {
            return Ok(None);
        }
        conn.execute(
                "UPDATE recording_schedules SET status='cancelled',updated_at=?3 WHERE id=?1 AND owner_id=?2 AND status='triggered'",
                vec![blob(id), blob(owner), at],
            ).await.map_err(map_turso)?;
    }
    get_schedule(conn, id).await
}
pub(crate) async fn claim_due(
    conn: &Connection,
    now: OffsetDateTime,
) -> StorageResult<Option<(RecordingSchedule, Recording)>> {
    let mut rows=conn.query(&format!("UPDATE recording_schedules SET status='triggered',updated_at=?1 WHERE id=(SELECT id FROM recording_schedules WHERE status='scheduled' AND datetime(start_at,'-'||padding_before_secs||' seconds')<=datetime(?1) ORDER BY start_at LIMIT 1) AND status='scheduled' RETURNING {}",SCHEDULE_COLS),vec![Value::Text(dt_to_text(now)?)]).await.map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let schedule = schedule_row(&row)?;
    drop(rows);
    let recording = Recording {
        id: RecordingId::new(),
        schedule_id: Some(schedule.id),
        owner_id: schedule.owner_id,
        channel_id: Some(schedule.channel_id),
        status: RecordingStatus::Running,
        path: None,
        error: None,
        size_bytes: None,
        started_at: Some(now),
        completed_at: None,
        created_at: now,
        updated_at: now,
    };
    upsert_recording(conn, &recording).await?;
    Ok(Some((schedule, recording)))
}

pub(crate) async fn upsert_recording(conn: &Connection, r: &Recording) -> StorageResult<()> {
    conn.execute("INSERT INTO recordings(id,schedule_id,owner_id,channel_id,status,path,error,size_bytes,started_at,completed_at,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(schedule_id) DO UPDATE SET status=excluded.status,path=excluded.path,error=excluded.error,size_bytes=excluded.size_bytes,started_at=excluded.started_at,completed_at=excluded.completed_at,updated_at=excluded.updated_at",vec![blob(r.id),r.schedule_id.map_or(Value::Null,blob),blob(r.owner_id),r.channel_id.map_or(Value::Null,blob),Value::Text(recording_status(r.status).into()),opt_text(r.path.as_deref()),opt_text(r.error.as_deref()),r.size_bytes.map(|v|i64::try_from(v).map(Value::Integer).map_err(|_|StorageError::InvalidInput("recording too large".into()))).transpose()?.unwrap_or(Value::Null),opt_dt(r.started_at)?,opt_dt(r.completed_at)?,Value::Text(dt_to_text(r.created_at)?),Value::Text(dt_to_text(r.updated_at)?)]).await.map_err(map_turso)?;
    Ok(())
}
pub(crate) async fn get_recording(
    conn: &Connection,
    id: RecordingId,
) -> StorageResult<Option<Recording>> {
    one_recording(conn, "id", blob(id)).await
}
pub(crate) async fn recording_for_schedule(
    conn: &Connection,
    id: RecordingScheduleId,
) -> StorageResult<Option<Recording>> {
    one_recording(conn, "schedule_id", blob(id)).await
}
async fn one_recording(
    conn: &Connection,
    column: &str,
    value: Value,
) -> StorageResult<Option<Recording>> {
    let mut rows = conn
        .query(
            &format!("{} WHERE {}=?1", RECORDING_SELECT, column),
            vec![value],
        )
        .await
        .map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|r| recording_row(&r))
        .transpose()
}
pub(crate) async fn list_recordings(
    conn: &Connection,
    owner: UserId,
    limit: u32,
    cursor: Option<PageCursor>,
) -> StorageResult<ItemPage<Recording>> {
    let limit = limit.clamp(1, ITEM_PAGE_MAX);
    let before = cursor_uuid(cursor)?.map_or(Value::Null, |v| Value::Blob(uuid_to_blob(v)));
    let mut rows = conn
        .query(
            &format!(
                "{} WHERE owner_id=?1 AND (?2 IS NULL OR id<?2) ORDER BY id DESC LIMIT ?3",
                RECORDING_SELECT
            ),
            vec![
                blob(owner),
                before,
                Value::Integer(i64::from(limit.saturating_add(1))),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut items = vec![];
    while let Some(r) = rows.next().await.map_err(map_turso)? {
        items.push(recording_row(&r)?)
    }
    Ok(ItemPage::from_lookahead(items, limit, |v| id_cursor(v.id)))
}
pub(crate) async fn update_recording(
    conn: &Connection,
    id: RecordingId,
    status: RecordingStatus,
    path: Option<&str>,
    error: Option<&str>,
    size: Option<u64>,
    at: OffsetDateTime,
) -> StorageResult<bool> {
    if !matches!(status, RecordingStatus::Completed | RecordingStatus::Failed) {
        return Err(StorageError::InvalidInput(
            "recording finish status must be completed or failed".into(),
        ));
    }
    let size = size
        .map(|v| {
            i64::try_from(v)
                .map(Value::Integer)
                .map_err(|_| StorageError::InvalidInput("recording too large".into()))
        })
        .transpose()?
        .unwrap_or(Value::Null);
    let changed = conn.execute("UPDATE recordings SET status=?2,path=?3,error=?4,size_bytes=?5,completed_at=?6,updated_at=?6 WHERE id=?1 AND status='running'",vec![blob(id),Value::Text(recording_status(status).into()),opt_text(path),opt_text(error),size,Value::Text(dt_to_text(at)?)]).await.map_err(map_turso)?;
    Ok(changed != 0)
}
pub(crate) async fn delete_recording(
    conn: &Connection,
    id: RecordingId,
    owner: UserId,
) -> StorageResult<Option<Recording>> {
    let found = get_recording(conn, id)
        .await?
        .filter(|r| r.owner_id == owner);
    if found.as_ref().is_some_and(|recording| {
        matches!(
            recording.status,
            RecordingStatus::Scheduled | RecordingStatus::Running
        )
    }) {
        return Err(StorageError::Conflict("recording is active".into()));
    }
    if found.is_some() {
        conn.execute(
            "DELETE FROM recordings WHERE id=?1 AND owner_id=?2",
            vec![blob(id), blob(owner)],
        )
        .await
        .map_err(map_turso)?;
    }
    Ok(found)
}
pub(crate) async fn fail_running(
    conn: &Connection,
    at: OffsetDateTime,
    error: &str,
) -> StorageResult<u64> {
    conn.execute("UPDATE recordings SET status='failed',error=?1,completed_at=?2,updated_at=?2 WHERE status='running'",vec![Value::Text(error.into()),Value::Text(dt_to_text(at)?)]).await.map_err(map_turso)
}

const SCHEDULE_COLS: &str = "id,owner_id,channel_id,program_id,start_at,end_at,padding_before_secs,padding_after_secs,status,created_at,updated_at";
const SCHEDULE_SELECT: &str = "SELECT id,owner_id,channel_id,program_id,start_at,end_at,padding_before_secs,padding_after_secs,status,created_at,updated_at FROM recording_schedules";
const RECORDING_SELECT: &str = "SELECT id,schedule_id,owner_id,channel_id,status,path,error,size_bytes,started_at,completed_at,created_at,updated_at FROM recordings";
fn schedule_status(v: RecordingScheduleStatus) -> &'static str {
    match v {
        RecordingScheduleStatus::Scheduled => "scheduled",
        RecordingScheduleStatus::Cancelled => "cancelled",
        RecordingScheduleStatus::Triggered => "triggered",
    }
}
fn recording_status(v: RecordingStatus) -> &'static str {
    match v {
        RecordingStatus::Scheduled => "scheduled",
        RecordingStatus::Running => "running",
        RecordingStatus::Completed => "completed",
        RecordingStatus::Failed => "failed",
        RecordingStatus::Cancelled => "cancelled",
    }
}
fn parse_schedule(v: &str) -> StorageResult<RecordingScheduleStatus> {
    match v {
        "scheduled" => Ok(RecordingScheduleStatus::Scheduled),
        "cancelled" => Ok(RecordingScheduleStatus::Cancelled),
        "triggered" => Ok(RecordingScheduleStatus::Triggered),
        _ => Err(StorageError::InvalidInput(
            "invalid recording schedule status".into(),
        )),
    }
}
fn parse_recording(v: &str) -> StorageResult<RecordingStatus> {
    match v {
        "scheduled" => Ok(RecordingStatus::Scheduled),
        "running" => Ok(RecordingStatus::Running),
        "completed" => Ok(RecordingStatus::Completed),
        "failed" => Ok(RecordingStatus::Failed),
        "cancelled" => Ok(RecordingStatus::Cancelled),
        _ => Err(StorageError::InvalidInput(
            "invalid recording status".into(),
        )),
    }
}

fn source_row(r: &turso::Row) -> StorageResult<LiveTvSource> {
    let v = (0..7)
        .map(|i| r.get_value(i).map_err(map_turso))
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(LiveTvSource {
        id: LiveTvSourceId::from_uuid(blob_to_uuid(col_blob(&v[0])?)?),
        name: col_text(&v[1])?.into(),
        enabled: col_int(&v[2])? != 0,
        playlist_url: col_text(&v[3])?.into(),
        epg_url: col_text_opt(&v[4])?,
        last_refresh_at: col_text_opt(&v[5])?.map(|x| text_to_dt(&x)).transpose()?,
        last_error: col_text_opt(&v[6])?,
    })
}
fn channel_row(r: &turso::Row) -> StorageResult<LiveTvChannel> {
    let v = (0..9)
        .map(|i| r.get_value(i).map_err(map_turso))
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(LiveTvChannel {
        id: ChannelId::from_uuid(blob_to_uuid(col_blob(&v[0])?)?),
        source_id: LiveTvSourceId::from_uuid(blob_to_uuid(col_blob(&v[1])?)?),
        external_id: col_text(&v[2])?.into(),
        name: col_text(&v[3])?.into(),
        number: col_text_opt(&v[4])?,
        logo_url: col_text_opt(&v[5])?,
        group: col_text_opt(&v[6])?,
        enabled: col_int(&v[7])? != 0,
        stream_url: col_text(&v[8])?.into(),
    })
}
fn program_row(r: &turso::Row) -> StorageResult<EpgProgram> {
    let v = (0..9)
        .map(|i| r.get_value(i).map_err(map_turso))
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(EpgProgram {
        id: EpgProgramId::from_uuid(blob_to_uuid(col_blob(&v[0])?)?),
        channel_id: ChannelId::from_uuid(blob_to_uuid(col_blob(&v[1])?)?),
        external_id: col_text(&v[2])?.into(),
        title: col_text(&v[3])?.into(),
        subtitle: col_text_opt(&v[4])?,
        description: col_text_opt(&v[5])?,
        category: col_text_opt(&v[6])?,
        start_at: text_to_dt(col_text(&v[7])?)?,
        end_at: text_to_dt(col_text(&v[8])?)?,
    })
}
fn schedule_row(r: &turso::Row) -> StorageResult<RecordingSchedule> {
    let v = (0..11)
        .map(|i| r.get_value(i).map_err(map_turso))
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(RecordingSchedule {
        id: RecordingScheduleId::from_uuid(blob_to_uuid(col_blob(&v[0])?)?),
        owner_id: UserId::from_uuid(blob_to_uuid(col_blob(&v[1])?)?),
        channel_id: ChannelId::from_uuid(blob_to_uuid(col_blob(&v[2])?)?),
        program_id: col_blob_opt(&v[3])?
            .map(|x| blob_to_uuid(&x).map(EpgProgramId::from_uuid))
            .transpose()?,
        start_at: text_to_dt(col_text(&v[4])?)?,
        end_at: text_to_dt(col_text(&v[5])?)?,
        padding_before_secs: u32::try_from(col_int(&v[6])?)
            .map_err(|_| StorageError::InvalidInput("invalid padding".into()))?,
        padding_after_secs: u32::try_from(col_int(&v[7])?)
            .map_err(|_| StorageError::InvalidInput("invalid padding".into()))?,
        status: parse_schedule(col_text(&v[8])?)?,
        created_at: text_to_dt(col_text(&v[9])?)?,
        updated_at: text_to_dt(col_text(&v[10])?)?,
    })
}
fn recording_row(r: &turso::Row) -> StorageResult<Recording> {
    let v = (0..12)
        .map(|i| r.get_value(i).map_err(map_turso))
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(Recording {
        id: RecordingId::from_uuid(blob_to_uuid(col_blob(&v[0])?)?),
        schedule_id: col_blob_opt(&v[1])?
            .map(|x| blob_to_uuid(&x).map(RecordingScheduleId::from_uuid))
            .transpose()?,
        owner_id: UserId::from_uuid(blob_to_uuid(col_blob(&v[2])?)?),
        channel_id: col_blob_opt(&v[3])?
            .map(|x| blob_to_uuid(&x).map(ChannelId::from_uuid))
            .transpose()?,
        status: parse_recording(col_text(&v[4])?)?,
        path: col_text_opt(&v[5])?,
        error: col_text_opt(&v[6])?,
        size_bytes: col_int_opt(&v[7])?
            .map(|x| {
                u64::try_from(x)
                    .map_err(|_| StorageError::InvalidInput("invalid recording size".into()))
            })
            .transpose()?,
        started_at: col_text_opt(&v[8])?.map(|x| text_to_dt(&x)).transpose()?,
        completed_at: col_text_opt(&v[9])?.map(|x| text_to_dt(&x)).transpose()?,
        created_at: text_to_dt(col_text(&v[10])?)?,
        updated_at: text_to_dt(col_text(&v[11])?)?,
    })
}
