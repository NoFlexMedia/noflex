// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{UserId, WatchRoomId};
use noflex_storage::{
    domain::{WatchRoom, WatchRoomMember, WatchRoomState},
    error::{StorageError, StorageResult},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{blob_to_uuid, col_blob, col_int, col_text, dt_to_text, text_to_dt, uuid_to_blob},
    error::map_turso,
};

fn state_text(state: WatchRoomState) -> &'static str {
    match state {
        WatchRoomState::Waiting => "waiting",
        WatchRoomState::Playing => "playing",
        WatchRoomState::Paused => "paused",
        WatchRoomState::Ended => "ended",
    }
}

fn parse_state(value: &str) -> StorageResult<WatchRoomState> {
    match value {
        "waiting" => Ok(WatchRoomState::Waiting),
        "playing" => Ok(WatchRoomState::Playing),
        "paused" => Ok(WatchRoomState::Paused),
        "ended" => Ok(WatchRoomState::Ended),
        _ => Err(StorageError::InvalidInput(
            "invalid watch room state".into(),
        )),
    }
}

pub(crate) async fn create(conn: &Connection, room: &WatchRoom) -> StorageResult<()> {
    conn.execute("INSERT INTO watch_rooms(id,host_user_id,media_file_id,state,position_ms,sequence,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", vec![
            Value::Blob(uuid_to_blob(room.id.into_uuid())), Value::Blob(uuid_to_blob(room.host_user_id.into_uuid())), Value::Blob(uuid_to_blob(room.media_file_id.into_uuid())), Value::Text(state_text(room.state).into()), Value::Integer(i64::try_from(room.position_ms).map_err(|_| StorageError::InvalidInput("position exceeds storage range".into()))?), Value::Integer(i64::try_from(room.sequence).map_err(|_| StorageError::InvalidInput("sequence exceeds storage range".into()))?), Value::Text(dt_to_text(room.created_at)?), Value::Text(dt_to_text(room.updated_at)?)
        ]).await.map_err(map_turso)?;
    conn.execute(
        "INSERT INTO watch_room_members(room_id,user_id,joined_at) VALUES (?1,?2,?3)",
        vec![
            Value::Blob(uuid_to_blob(room.id.into_uuid())),
            Value::Blob(uuid_to_blob(room.host_user_id.into_uuid())),
            Value::Text(dt_to_text(room.created_at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn get(conn: &Connection, id: WatchRoomId) -> StorageResult<Option<WatchRoom>> {
    let mut rows=conn.query("SELECT id,host_user_id,media_file_id,state,position_ms,sequence,created_at,updated_at FROM watch_rooms WHERE id=?1", vec![Value::Blob(uuid_to_blob(id.into_uuid()))]).await.map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_room(&row))
        .transpose()
}

pub(crate) async fn list(conn: &Connection, user: UserId) -> StorageResult<Vec<WatchRoom>> {
    let mut rows=conn.query("SELECT r.id,r.host_user_id,r.media_file_id,r.state,r.position_ms,r.sequence,r.created_at,r.updated_at FROM watch_rooms r JOIN watch_room_members m ON m.room_id=r.id WHERE m.user_id=?1 ORDER BY r.created_at DESC", vec![Value::Blob(uuid_to_blob(user.into_uuid()))]).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_room(&row)?);
    }
    Ok(out)
}

pub(crate) async fn members(
    conn: &Connection,
    room: WatchRoomId,
) -> StorageResult<Vec<WatchRoomMember>> {
    let mut rows=conn.query("SELECT room_id,user_id,joined_at FROM watch_room_members WHERE room_id=?1 ORDER BY joined_at,user_id", vec![Value::Blob(uuid_to_blob(room.into_uuid()))]).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let room_id = row.get_value(0).map_err(map_turso)?;
        let user = row.get_value(1).map_err(map_turso)?;
        let joined = row.get_value(2).map_err(map_turso)?;
        out.push(WatchRoomMember {
            room_id: WatchRoomId::from_uuid(blob_to_uuid(col_blob(&room_id)?)?),
            user_id: UserId::from_uuid(blob_to_uuid(col_blob(&user)?)?),
            joined_at: text_to_dt(col_text(&joined)?)?,
        });
    }
    Ok(out)
}

pub(crate) async fn join(
    conn: &Connection,
    member: &WatchRoomMember,
) -> StorageResult<Option<WatchRoom>> {
    let Some(room) = get(conn, member.room_id).await? else {
        return Ok(None);
    };
    if room.state == WatchRoomState::Ended {
        return Ok(None);
    }
    let Some(work) = crate::repo::restrictions::work_for_media(conn, room.media_file_id).await?
    else {
        return Ok(None);
    };
    if !crate::repo::restrictions::can_access(conn, member.user_id, work.id).await? {
        return Ok(None);
    }
    conn.execute("INSERT INTO watch_room_members(room_id,user_id,joined_at) VALUES (?1,?2,?3) ON CONFLICT(room_id,user_id) DO NOTHING", vec![Value::Blob(uuid_to_blob(member.room_id.into_uuid())),Value::Blob(uuid_to_blob(member.user_id.into_uuid())),Value::Text(dt_to_text(member.joined_at)?)]).await.map_err(map_turso)?;
    Ok(Some(room))
}

pub(crate) async fn leave(
    conn: &Connection,
    room: WatchRoomId,
    user: UserId,
) -> StorageResult<Option<WatchRoom>> {
    let Some(current) = get(conn, room).await? else {
        return Ok(None);
    };
    let changed = conn
        .execute(
            "DELETE FROM watch_room_members WHERE room_id=?1 AND user_id=?2",
            vec![
                Value::Blob(uuid_to_blob(room.into_uuid())),
                Value::Blob(uuid_to_blob(user.into_uuid())),
            ],
        )
        .await
        .map_err(map_turso)?;
    if changed == 0 || current.host_user_id != user {
        return Ok(None);
    }
    let mut rows = conn.query("UPDATE watch_rooms SET state='ended',sequence=sequence+1,updated_at=?2 WHERE id=?1 AND state != 'ended' RETURNING id,host_user_id,media_file_id,state,position_ms,sequence,created_at,updated_at",vec![Value::Blob(uuid_to_blob(room.into_uuid())),Value::Text(dt_to_text(OffsetDateTime::now_utc())?)]).await.map_err(map_turso)?;
    let ended = rows
        .next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_room(&row))
        .transpose()?;
    drop(rows);
    conn.execute(
        "DELETE FROM watch_room_members WHERE room_id=?1",
        vec![Value::Blob(uuid_to_blob(room.into_uuid()))],
    )
    .await
    .map_err(map_turso)?;
    Ok(ended)
}

pub(crate) async fn control(
    conn: &Connection,
    room: WatchRoomId,
    host: UserId,
    state: Option<WatchRoomState>,
    position_ms: Option<u64>,
    updated_at: OffsetDateTime,
) -> StorageResult<Option<WatchRoom>> {
    let state = state.map_or(Value::Null, |state| Value::Text(state_text(state).into()));
    let position = position_ms
        .map(|position| {
            i64::try_from(position)
                .map(Value::Integer)
                .map_err(|_| StorageError::InvalidInput("position exceeds storage range".into()))
        })
        .transpose()?
        .unwrap_or(Value::Null);
    let mut rows=conn.query("UPDATE watch_rooms SET state=COALESCE(?3,state),position_ms=COALESCE(?4,position_ms),sequence=sequence+1,updated_at=?5 WHERE id=?1 AND host_user_id=?2 AND state != 'ended' RETURNING id,host_user_id,media_file_id,state,position_ms,sequence,created_at,updated_at",vec![Value::Blob(uuid_to_blob(room.into_uuid())),Value::Blob(uuid_to_blob(host.into_uuid())),state,position,Value::Text(dt_to_text(updated_at)?)]).await.map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|row| row_to_room(&row))
        .transpose()
}

fn row_to_room(row: &turso::Row) -> StorageResult<WatchRoom> {
    let id = row.get_value(0).map_err(map_turso)?;
    let host = row.get_value(1).map_err(map_turso)?;
    let media = row.get_value(2).map_err(map_turso)?;
    let state = row.get_value(3).map_err(map_turso)?;
    let position = row.get_value(4).map_err(map_turso)?;
    let sequence = row.get_value(5).map_err(map_turso)?;
    let created = row.get_value(6).map_err(map_turso)?;
    let updated = row.get_value(7).map_err(map_turso)?;
    Ok(WatchRoom {
        id: WatchRoomId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        host_user_id: UserId::from_uuid(blob_to_uuid(col_blob(&host)?)?),
        media_file_id: noflex_core::MediaFileId::from_uuid(blob_to_uuid(col_blob(&media)?)?),
        state: parse_state(col_text(&state)?)?,
        position_ms: u64::try_from(col_int(&position)?)
            .map_err(|_| StorageError::InvalidInput("negative watch position".into()))?,
        sequence: u64::try_from(col_int(&sequence)?)
            .map_err(|_| StorageError::InvalidInput("negative watch sequence".into()))?,
        created_at: text_to_dt(col_text(&created)?)?,
        updated_at: text_to_dt(col_text(&updated)?)?,
    })
}
