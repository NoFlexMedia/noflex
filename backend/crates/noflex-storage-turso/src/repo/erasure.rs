// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::UserId;
use noflex_storage::{
    domain::{AccountPurge, ActivityKind, ServerReset},
    error::{StorageError, StorageResult},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, dt_to_text, parse_json, text_to_dt, to_json,
        uuid_to_blob,
    },
    error::map_turso,
};

/// Kept through an erasure: what lets the database open as first-run setup
/// finds it, and the erasure's own record until it's done.
const KEPT: [&str; 2] = ["schema_meta", "server_reset"];

/// Kinds whose rows are a user's own doing, and go with them even where the
/// subject is something else.
const OWN_KINDS: [ActivityKind; 4] = [
    ActivityKind::PlayStarted,
    ActivityKind::PlayProgress,
    ActivityKind::PlayCompleted,
    ActivityKind::RefreshTokenReuse,
];

/// Whether `user` is an administrator, and whether anyone else is; `None`
/// without such a user.
async fn admin_standing(conn: &Connection, user: UserId) -> StorageResult<Option<(bool, bool)>> {
    let mut rows = conn
        .query(
            "SELECT is_admin, \
                 EXISTS (SELECT 1 FROM users WHERE is_admin = 1 AND id <> ?1) \
             FROM users WHERE id = ?1",
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let is_admin = col_int(&row.get_value(0).map_err(map_turso)?)? != 0;
    let others = col_int(&row.get_value(1).map_err(map_turso)?)? != 0;
    Ok(Some((is_admin, others)))
}

/// Must run in a serial transaction: the last-administrator check spans rows.
pub(crate) async fn delete_account(
    conn: &Connection,
    user: UserId,
    at: OffsetDateTime,
) -> StorageResult<Option<AccountPurge>> {
    let Some((is_admin, other_admins)) = admin_standing(conn, user).await? else {
        return Ok(None);
    };
    if is_admin && !other_admins {
        return Err(StorageError::Conflict(
            "the last administrator's account can only go with the server".into(),
        ));
    }
    let id = Value::Blob(uuid_to_blob(user.into_uuid()));
    let mut rows = conn
        .query(
            "SELECT output_path FROM export_jobs WHERE user_id = ?1 AND output_path IS NOT NULL \
             UNION SELECT path FROM recordings WHERE owner_id = ?1 AND path IS NOT NULL",
            vec![id.clone()],
        )
        .await
        .map_err(map_turso)?;
    let mut files = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        files.push(col_text(&row.get_value(0).map_err(map_turso)?)?.to_string());
    }
    let purge = AccountPurge {
        user_id: user,
        files,
        requested_at: at,
    };
    conn.execute(
        "INSERT INTO account_purges (user_id, files_json, requested_at) VALUES (?1, ?2, ?3)",
        vec![
            id.clone(),
            Value::Text(to_json(&purge.files)?),
            Value::Text(dt_to_text(at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    conn.execute("DELETE FROM users WHERE id = ?1", vec![id])
        .await
        .map_err(map_turso)?;
    Ok(Some(purge))
}

pub(crate) async fn pending_purges(conn: &Connection) -> StorageResult<Vec<AccountPurge>> {
    let mut rows = conn
        .query(
            "SELECT user_id, files_json, requested_at FROM account_purges \
             ORDER BY requested_at, user_id",
            (),
        )
        .await
        .map_err(map_turso)?;
    let mut purges = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let get = |index| row.get_value(index).map_err(map_turso);
        purges.push(AccountPurge {
            user_id: UserId::from_uuid(blob_to_uuid(col_blob(&get(0)?)?)?),
            files: parse_json(col_text(&get(1)?)?)?,
            requested_at: text_to_dt(col_text(&get(2)?)?)?,
        });
    }
    Ok(purges)
}

pub(crate) async fn forget_activity(conn: &Connection, user: UserId) -> StorageResult<u64> {
    let id = user.to_string();
    let mut params = vec![Value::Text(id.clone())];
    params.extend(
        OWN_KINDS
            .iter()
            .map(|kind| Value::Text(kind.as_str().to_string())),
    );
    let kinds = (2..=params.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let removed = conn
        .execute(
            &format!(
                "DELETE FROM activity_log \
                 WHERE subject = ?1 OR (actor = ?1 AND kind IN ({kinds}))"
            ),
            params,
        )
        .await
        .map_err(map_turso)?;
    // The append path stores an absent actor as empty text.
    let anonymised = conn
        .execute(
            "UPDATE activity_log SET actor = '' WHERE actor = ?1",
            vec![Value::Text(id)],
        )
        .await
        .map_err(map_turso)?;
    Ok(removed + anonymised)
}

pub(crate) async fn finish_purge(conn: &Connection, user: UserId) -> StorageResult<()> {
    conn.execute(
        "DELETE FROM account_purges WHERE user_id = ?1",
        vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

/// Must run in a serial transaction: the only-administrator check spans rows.
pub(crate) async fn request_reset(
    conn: &Connection,
    admin: UserId,
    reset: &ServerReset,
) -> StorageResult<bool> {
    if admin_standing(conn, admin).await? != Some((true, false)) {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO server_reset (id, folders_json, requested_at) VALUES (1, ?1, ?2) \
         ON CONFLICT(id) DO NOTHING",
        vec![
            Value::Text(to_json(&reset.folders)?),
            Value::Text(dt_to_text(reset.requested_at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    for table in ["sessions", "api_keys", "device_codes", "invites"] {
        conn.execute(&format!("DELETE FROM {table}"), ())
            .await
            .map_err(map_turso)?;
    }
    Ok(true)
}

pub(crate) async fn pending_reset(conn: &Connection) -> StorageResult<Option<ServerReset>> {
    let mut rows = conn
        .query(
            "SELECT folders_json, requested_at FROM server_reset WHERE id = 1",
            (),
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let get = |index| row.get_value(index).map_err(map_turso);
    Ok(Some(ServerReset {
        folders: parse_json(col_text(&get(0)?)?)?,
        requested_at: text_to_dt(col_text(&get(1)?)?)?,
    }))
}

/// Empties every table but [`KEPT`] and the engine's own. Each is its own statement, so no one
/// transaction holds a whole library's worth of row versions, and a cut-short
/// run picks up where it stopped. Newest tables first: a table only references
/// those made before it, so with children emptied ahead of their parents no
/// `ON DELETE RESTRICT` refuses and no cascade has anything left to do.
pub(crate) async fn erase_all(conn: &Connection) -> StorageResult<()> {
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_schema \
             WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                 AND name NOT LIKE '\\_\\_turso\\_internal%' ESCAPE '\\' \
             ORDER BY rowid DESC",
            (),
        )
        .await
        .map_err(map_turso)?;
    let mut tables = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let name = col_text(&row.get_value(0).map_err(map_turso)?)?.to_string();
        if !KEPT.contains(&name.as_str()) {
            tables.push(name);
        }
    }
    drop(rows);
    for table in tables {
        conn.execute(
            &format!("DELETE FROM \"{}\"", table.replace('"', "\"\"")),
            (),
        )
        .await
        .map_err(map_turso)?;
    }
    Ok(())
}

pub(crate) async fn finish_reset(conn: &Connection) -> StorageResult<()> {
    conn.execute("DELETE FROM server_reset", ())
        .await
        .map_err(map_turso)?;
    Ok(())
}
