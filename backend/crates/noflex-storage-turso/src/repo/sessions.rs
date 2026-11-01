// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{SessionId, UserId};
use noflex_storage::{
    domain::{RefreshTokenLookup, RetiredRefreshToken, Session},
    error::StorageResult,
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, text_to_dt,
        uuid_to_blob,
    },
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO sessions (\
    id, user_id, client, device, device_id, app_version, remote_addr, created_at, last_seen_at, \
    expires_at, refresh_token_hash\
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) \
ON CONFLICT(id) DO UPDATE SET \
    user_id = excluded.user_id, \
    client = excluded.client, \
    device = excluded.device, \
    device_id = excluded.device_id, \
    app_version = excluded.app_version, \
    remote_addr = excluded.remote_addr, \
    last_seen_at = excluded.last_seen_at, \
    expires_at = excluded.expires_at, \
    refresh_token_hash = excluded.refresh_token_hash";

const TOUCH: &str = "UPDATE sessions SET last_seen_at = ?1 WHERE id = ?2";

const ROTATE_REFRESH: &str = "UPDATE sessions \
    SET refresh_token_hash = ?1, expires_at = ?2, last_seen_at = ?3 \
    WHERE id = ?4 AND refresh_token_hash = ?5";

const RETIRE_REFRESH: &str = "INSERT INTO retired_refresh_tokens \
    (token_hash, session_id, replaced_by, retired_at) VALUES (?1, ?2, ?3, ?4)";

const GET_BY_ID: &str = "SELECT id, user_id, client, device, device_id, app_version, \
    remote_addr, created_at, last_seen_at, expires_at, refresh_token_hash \
    FROM sessions WHERE id = ?1";

// One statement, so current and retired matches come from a single
// snapshot. Columns 0..=10 are the session; 11 is NULL for a current
// match, else the retirement time; 12 flags the immediately-previous token.
const GET_BY_REFRESH_TOKEN: &str = "\
    SELECT id, user_id, client, device, device_id, app_version, remote_addr, created_at, \
        last_seen_at, expires_at, refresh_token_hash, NULL, 0 \
    FROM sessions WHERE refresh_token_hash = ?1 \
    UNION ALL \
    SELECT s.id, s.user_id, s.client, s.device, s.device_id, s.app_version, s.remote_addr, \
        s.created_at, s.last_seen_at, s.expires_at, s.refresh_token_hash, r.retired_at, \
        s.refresh_token_hash IS NOT NULL AND r.replaced_by = s.refresh_token_hash \
    FROM retired_refresh_tokens r JOIN sessions s ON s.id = r.session_id \
    WHERE r.token_hash = ?1";

const LIST_FOR_USER: &str = "SELECT id, user_id, client, device, device_id, app_version, \
    remote_addr, created_at, last_seen_at, expires_at, refresh_token_hash \
    FROM sessions WHERE user_id = ?1 \
    ORDER BY last_seen_at DESC";

const REVOKE: &str = "DELETE FROM sessions WHERE id = ?1";

const LIST_ALL: &str = "SELECT id, user_id, client, device, device_id, app_version, \
    remote_addr, created_at, last_seen_at, expires_at, refresh_token_hash \
    FROM sessions ORDER BY last_seen_at DESC";

const COUNT_USER: &str = "SELECT COUNT(*) FROM sessions WHERE user_id = ?1";

const REVOKE_USER: &str = "DELETE FROM sessions WHERE user_id = ?1";

const PRUNE_BEFORE: &str = "DELETE FROM sessions WHERE last_seen_at < ?1";

pub(crate) async fn create(conn: &Connection, session: &Session) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(session.id.into_uuid())),
        Value::Blob(uuid_to_blob(session.user_id.into_uuid())),
        session
            .client
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        session
            .device
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        session
            .device_id
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        session
            .app_version
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        session
            .remote_addr
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Value::Text(dt_to_text(session.created_at)?),
        Value::Text(dt_to_text(session.last_seen_at)?),
        Value::Text(dt_to_text(session.expires_at)?),
        session
            .refresh_token_hash
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn touch(
    conn: &Connection,
    id: SessionId,
    at: time::OffsetDateTime,
) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Text(dt_to_text(at)?),
        Value::Blob(uuid_to_blob(id.into_uuid())),
    ];
    conn.execute(TOUCH, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn get(conn: &Connection, id: SessionId) -> StorageResult<Option<Session>> {
    let mut rows = conn
        .query(GET_BY_ID, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_session(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_by_refresh_token(
    conn: &Connection,
    hash: &str,
) -> StorageResult<RefreshTokenLookup> {
    let mut rows = conn
        .query(GET_BY_REFRESH_TOKEN, vec![Value::Text(hash.to_string())])
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(RefreshTokenLookup::Unknown);
    };
    let session = row_to_session(&row)?;
    let retired_at = row.get_value(11).map_err(map_turso)?;
    let Some(retired_at) = col_text_opt(&retired_at)? else {
        return Ok(RefreshTokenLookup::Current(session));
    };
    let latest = row.get_value(12).map_err(map_turso)?;
    Ok(RefreshTokenLookup::Retired(RetiredRefreshToken {
        session,
        retired_at: text_to_dt(&retired_at)?,
        latest: col_int(&latest)? != 0,
    }))
}

/// Must run inside a write transaction: the swap and the retirement
/// record land together or not at all.
pub(crate) async fn rotate_refresh_token(
    conn: &Connection,
    id: SessionId,
    expected_hash: &str,
    new_hash: &str,
    new_expires_at: time::OffsetDateTime,
    now: time::OffsetDateTime,
) -> StorageResult<bool> {
    let id_blob = uuid_to_blob(id.into_uuid());
    let now = dt_to_text(now)?;
    let swapped = conn
        .execute(
            ROTATE_REFRESH,
            vec![
                Value::Text(new_hash.to_string()),
                Value::Text(dt_to_text(new_expires_at)?),
                Value::Text(now.clone()),
                Value::Blob(id_blob.clone()),
                Value::Text(expected_hash.to_string()),
            ],
        )
        .await
        .map_err(map_turso)?;
    if swapped != 1 {
        return Ok(false);
    }
    conn.execute(
        RETIRE_REFRESH,
        vec![
            Value::Text(expected_hash.to_string()),
            Value::Blob(id_blob),
            Value::Text(new_hash.to_string()),
            Value::Text(now),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(true)
}

pub(crate) async fn list_for_user(
    conn: &Connection,
    user_id: UserId,
) -> StorageResult<Vec<Session>> {
    let mut rows = conn
        .query(
            LIST_FOR_USER,
            vec![Value::Blob(uuid_to_blob(user_id.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_session(&row)?);
    }
    Ok(out)
}

pub(crate) async fn revoke(conn: &Connection, id: SessionId) -> StorageResult<()> {
    conn.execute(REVOKE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn list_all(conn: &Connection) -> StorageResult<Vec<Session>> {
    let mut rows = conn.query(LIST_ALL, ()).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_session(&row)?);
    }
    Ok(out)
}

pub(crate) async fn count_user(conn: &Connection, user: UserId) -> StorageResult<u32> {
    let mut rows = conn
        .query(
            COUNT_USER,
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let row = rows
        .next()
        .await
        .map_err(map_turso)?
        .ok_or_else(|| crate::error::backend_msg("COUNT(*) returned no row"))?;
    let n = row.get_value(0).map_err(map_turso)?;
    Ok(crate::convert::col_int(&n)? as u32)
}

pub(crate) async fn revoke_user(conn: &Connection, user: UserId) -> StorageResult<u32> {
    let n = conn
        .execute(
            REVOKE_USER,
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    Ok(n as u32)
}

pub(crate) async fn prune_before(
    conn: &Connection,
    before: time::OffsetDateTime,
) -> StorageResult<u32> {
    let n = conn
        .execute(
            PRUNE_BEFORE,
            vec![Value::Text(crate::convert::dt_to_text(before)?)],
        )
        .await
        .map_err(map_turso)?;
    Ok(n as u32)
}

fn row_to_session(row: &turso::Row) -> StorageResult<Session> {
    let id = row.get_value(0).map_err(map_turso)?;
    let user_id = row.get_value(1).map_err(map_turso)?;
    let client = row.get_value(2).map_err(map_turso)?;
    let device = row.get_value(3).map_err(map_turso)?;
    let device_id = row.get_value(4).map_err(map_turso)?;
    let app_version = row.get_value(5).map_err(map_turso)?;
    let remote_addr = row.get_value(6).map_err(map_turso)?;
    let created_at = row.get_value(7).map_err(map_turso)?;
    let last_seen_at = row.get_value(8).map_err(map_turso)?;
    let expires_at = row.get_value(9).map_err(map_turso)?;
    let refresh_token_hash = row.get_value(10).map_err(map_turso)?;
    Ok(Session {
        id: SessionId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(&user_id)?)?),
        client: col_text_opt(&client)?,
        device: col_text_opt(&device)?,
        device_id: col_text_opt(&device_id)?,
        app_version: col_text_opt(&app_version)?,
        remote_addr: col_text_opt(&remote_addr)?,
        created_at: text_to_dt(col_text(&created_at)?)?,
        last_seen_at: text_to_dt(col_text(&last_seen_at)?)?,
        expires_at: text_to_dt(col_text(&expires_at)?)?,
        refresh_token_hash: col_text_opt(&refresh_token_hash)?,
    })
}
