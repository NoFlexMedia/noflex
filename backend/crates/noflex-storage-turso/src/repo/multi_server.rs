// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{InviteId, LibraryId, LinkedServerId, UserId};
use noflex_storage::{
    domain::{Invite, LinkedServer},
    error::StorageResult,
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, parse_json,
        text_to_dt, to_json, uuid_to_blob,
    },
    error::map_turso,
};

const INVITE_COLUMNS: &str =
    "id, created_by, library_ids_json, max_uses, uses, expires_at, revoked_at, created_at";

fn row_to_invite(row: &turso::Row) -> StorageResult<Invite> {
    let get = |index| row.get_value(index).map_err(map_turso);
    let libraries: Vec<LibraryId> = parse_json(col_text(&get(2)?)?)?;
    Ok(Invite {
        id: InviteId::from_uuid(blob_to_uuid(col_blob(&get(0)?)?)?),
        created_by: UserId::from_uuid(blob_to_uuid(col_blob(&get(1)?)?)?),
        library_ids: libraries,
        max_uses: col_int(&get(3)?)? as u32,
        uses: col_int(&get(4)?)? as u32,
        expires_at: text_to_dt(col_text(&get(5)?)?)?,
        revoked_at: col_text_opt(&get(6)?)?
            .as_deref()
            .map(text_to_dt)
            .transpose()?,
        created_at: text_to_dt(col_text(&get(7)?)?)?,
    })
}

pub(crate) async fn create_invite(
    conn: &Connection,
    invite: &Invite,
    token_hash: &str,
) -> StorageResult<()> {
    conn.execute(
        "INSERT INTO invites (id, token_hash, created_by, library_ids_json, max_uses, expires_at, \
            created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        vec![
            Value::Blob(uuid_to_blob(invite.id.into_uuid())),
            Value::Text(token_hash.to_string()),
            Value::Blob(uuid_to_blob(invite.created_by.into_uuid())),
            Value::Text(to_json(&invite.library_ids)?),
            Value::Integer(invite.max_uses.into()),
            Value::Text(dt_to_text(invite.expires_at)?),
            Value::Text(dt_to_text(invite.created_at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn list_invites(conn: &Connection) -> StorageResult<Vec<Invite>> {
    let sql = format!("SELECT {INVITE_COLUMNS} FROM invites ORDER BY created_at DESC, id DESC");
    let mut rows = conn.query(&sql, ()).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_invite(&row)?);
    }
    Ok(out)
}

pub(crate) async fn consume_invite(
    conn: &Connection,
    token_hash: &str,
    now: OffsetDateTime,
) -> StorageResult<Option<Invite>> {
    let sql = format!(
        "UPDATE invites SET uses = uses + 1 \
         WHERE token_hash = ?1 AND revoked_at IS NULL AND expires_at > ?2 AND uses < max_uses \
         RETURNING {INVITE_COLUMNS}"
    );
    let mut rows = conn
        .query(
            &sql,
            vec![
                Value::Text(token_hash.to_string()),
                Value::Text(dt_to_text(now)?),
            ],
        )
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_invite(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn release_invite_use(conn: &Connection, id: InviteId) -> StorageResult<()> {
    conn.execute(
        "UPDATE invites SET uses = uses - 1 WHERE id = ?1 AND uses > 0",
        vec![Value::Blob(uuid_to_blob(id.into_uuid()))],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn revoke_invite(
    conn: &Connection,
    id: InviteId,
    now: OffsetDateTime,
) -> StorageResult<bool> {
    let changed = conn
        .execute(
            "UPDATE invites SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            vec![
                Value::Blob(uuid_to_blob(id.into_uuid())),
                Value::Text(dt_to_text(now)?),
            ],
        )
        .await
        .map_err(map_turso)?;
    Ok(changed > 0)
}

const SERVER_COLUMNS: &str =
    "id, user_id, origin, name, api_key, sync_enabled, synced_until, created_at";

fn row_to_server(row: &turso::Row) -> StorageResult<LinkedServer> {
    let get = |index| row.get_value(index).map_err(map_turso);
    Ok(LinkedServer {
        id: LinkedServerId::from_uuid(blob_to_uuid(col_blob(&get(0)?)?)?),
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(&get(1)?)?)?),
        origin: col_text(&get(2)?)?.to_string(),
        name: col_text(&get(3)?)?.to_string(),
        api_key: col_text(&get(4)?)?.to_string(),
        sync_enabled: col_int(&get(5)?)? != 0,
        synced_until: col_text_opt(&get(6)?)?
            .as_deref()
            .map(text_to_dt)
            .transpose()?,
        created_at: text_to_dt(col_text(&get(7)?)?)?,
    })
}

pub(crate) async fn upsert_linked_server(
    conn: &Connection,
    server: &LinkedServer,
) -> StorageResult<LinkedServer> {
    // Relinking an origin replaces its key but remembers how far sync got.
    let sql = format!(
        "INSERT INTO linked_servers (id, user_id, origin, name, api_key, sync_enabled, \
            synced_until, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
         ON CONFLICT(user_id, origin) DO UPDATE SET name = excluded.name, \
            api_key = excluded.api_key, sync_enabled = excluded.sync_enabled, \
            synced_until = excluded.synced_until \
         RETURNING {SERVER_COLUMNS}"
    );
    let synced_until = match server.synced_until {
        Some(at) => Value::Text(dt_to_text(at)?),
        None => Value::Null,
    };
    let mut rows = conn
        .query(
            &sql,
            vec![
                Value::Blob(uuid_to_blob(server.id.into_uuid())),
                Value::Blob(uuid_to_blob(server.user_id.into_uuid())),
                Value::Text(server.origin.clone()),
                Value::Text(server.name.clone()),
                Value::Text(server.api_key.clone()),
                Value::Integer(server.sync_enabled.into()),
                synced_until,
                Value::Text(dt_to_text(server.created_at)?),
            ],
        )
        .await
        .map_err(map_turso)?;
    let row = rows
        .next()
        .await
        .map_err(map_turso)?
        .ok_or(noflex_storage::error::StorageError::NotFound)?;
    row_to_server(&row)
}

pub(crate) async fn linked_servers_for_user(
    conn: &Connection,
    user: UserId,
) -> StorageResult<Vec<LinkedServer>> {
    let sql =
        format!("SELECT {SERVER_COLUMNS} FROM linked_servers WHERE user_id = ?1 ORDER BY name, id");
    let mut rows = conn
        .query(&sql, vec![Value::Blob(uuid_to_blob(user.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_server(&row)?);
    }
    Ok(out)
}

pub(crate) async fn delete_linked_server(
    conn: &Connection,
    user: UserId,
    id: LinkedServerId,
) -> StorageResult<bool> {
    let removed = conn
        .execute(
            "DELETE FROM linked_servers WHERE id = ?1 AND user_id = ?2",
            vec![
                Value::Blob(uuid_to_blob(id.into_uuid())),
                Value::Blob(uuid_to_blob(user.into_uuid())),
            ],
        )
        .await
        .map_err(map_turso)?;
    Ok(removed > 0)
}

pub(crate) async fn linked_origins(conn: &Connection) -> StorageResult<Vec<String>> {
    let mut rows = conn
        .query(
            "SELECT DISTINCT origin FROM linked_servers ORDER BY origin",
            (),
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(col_text(&row.get_value(0).map_err(map_turso)?)?.to_string());
    }
    Ok(out)
}
