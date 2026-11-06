// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{UserId, UserRole};
use noflex_storage::{
    domain::User,
    error::{StorageError, StorageResult},
};
use time::OffsetDateTime;
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, col_text_opt, dt_to_text, text_to_dt,
        uuid_to_blob,
    },
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO users (id, username, display_name, password_hash, is_admin, role, created_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
    ON CONFLICT(id) DO UPDATE SET \
        username = excluded.username, \
        display_name = excluded.display_name, \
        password_hash = excluded.password_hash, \
        is_admin = excluded.is_admin, \
        role = excluded.role";

const SELECT: &str = "SELECT u.id, u.username, u.display_name, u.password_hash, u.is_admin, \
    u.role, u.created_at, a.updated_at \
    FROM users u LEFT JOIN user_avatars a ON a.user_id = u.id";

const DELETE: &str = "DELETE FROM users WHERE id = ?1";

pub(crate) async fn upsert(conn: &Connection, user: &User) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(user.id.into_uuid())),
        Value::Text(user.username.clone()),
        user.display_name
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Value::Text(user.password_hash.clone()),
        Value::Integer(user.is_admin as i64),
        Value::Text(user.role_as_text().to_string()),
        Value::Text(dt_to_text(user.created_at)?),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

const INSERT_IF_EMPTY: &str = "INSERT INTO users (id, username, display_name, password_hash, is_admin, role, created_at) \
    SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE NOT EXISTS (SELECT 1 FROM users)";

pub(crate) async fn create_initial_admin(conn: &Connection, user: &User) -> StorageResult<bool> {
    if !user.is_admin || user.role != UserRole::Admin {
        return Err(StorageError::InvalidInput(
            "initial user must be an administrator".into(),
        ));
    }
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(user.id.into_uuid())),
        Value::Text(user.username.clone()),
        user.display_name
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Value::Text(user.password_hash.clone()),
        Value::Integer(1),
        Value::Text(user.role_as_text().to_string()),
        Value::Text(dt_to_text(user.created_at)?),
    ];
    let inserted = conn
        .execute(INSERT_IF_EMPTY, params)
        .await
        .map_err(map_turso)?;
    Ok(inserted == 1)
}

pub(crate) async fn get(conn: &Connection, id: UserId) -> StorageResult<Option<User>> {
    let mut rows = conn
        .query(
            &format!("{SELECT} WHERE u.id = ?1"),
            vec![Value::Blob(uuid_to_blob(id.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_user(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_by_username(
    conn: &Connection,
    username: &str,
) -> StorageResult<Option<User>> {
    let mut rows = conn
        .query(
            &format!("{SELECT} WHERE u.username = ?1"),
            vec![Value::Text(username.to_string())],
        )
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_user(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_all(conn: &Connection) -> StorageResult<Vec<User>> {
    let mut rows = conn
        .query(&format!("{SELECT} ORDER BY u.username"), ())
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_user(&row)?);
    }
    Ok(out)
}

pub(crate) async fn delete(conn: &Connection, id: UserId) -> StorageResult<()> {
    conn.execute(DELETE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn set_avatar(
    conn: &Connection,
    user: UserId,
    image: &[u8],
    at: OffsetDateTime,
) -> StorageResult<()> {
    conn.execute(
        "INSERT INTO user_avatars (user_id, image, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(user_id) DO UPDATE SET image = excluded.image, updated_at = excluded.updated_at",
        vec![
            Value::Blob(uuid_to_blob(user.into_uuid())),
            Value::Blob(image.to_vec()),
            Value::Text(dt_to_text(at)?),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn avatar(conn: &Connection, user: UserId) -> StorageResult<Option<Vec<u8>>> {
    let mut rows = conn
        .query(
            "SELECT image FROM user_avatars WHERE user_id = ?1",
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let image = row.get_value(0).map_err(map_turso)?;
    Ok(Some(col_blob(&image)?.to_vec()))
}

pub(crate) async fn delete_avatar(conn: &Connection, user: UserId) -> StorageResult<bool> {
    let removed = conn
        .execute(
            "DELETE FROM user_avatars WHERE user_id = ?1",
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    Ok(removed > 0)
}

fn row_to_user(row: &turso::Row) -> StorageResult<User> {
    let id = row.get_value(0).map_err(map_turso)?;
    let username = row.get_value(1).map_err(map_turso)?;
    let display_name = row.get_value(2).map_err(map_turso)?;
    let password_hash = row.get_value(3).map_err(map_turso)?;
    let is_admin = row.get_value(4).map_err(map_turso)?;
    let role = row.get_value(5).map_err(map_turso)?;
    let created_at = row.get_value(6).map_err(map_turso)?;
    let avatar_updated_at = row.get_value(7).map_err(map_turso)?;
    let is_admin = col_int(&is_admin)? != 0;
    let role = text_to_role(col_text(&role)?).unwrap_or(if is_admin {
        UserRole::Admin
    } else {
        UserRole::User
    });
    Ok(User {
        id: UserId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        username: col_text(&username)?.to_string(),
        display_name: col_text_opt(&display_name)?,
        password_hash: col_text(&password_hash)?.to_string(),
        is_admin,
        role,
        created_at: text_to_dt(col_text(&created_at)?)?,
        avatar_updated_at: col_text_opt(&avatar_updated_at)?
            .map(|at| text_to_dt(&at))
            .transpose()?,
    })
}

fn text_to_role(s: &str) -> Option<UserRole> {
    match s {
        "admin" => Some(UserRole::Admin),
        "user" => Some(UserRole::User),
        "guest" => Some(UserRole::Guest),
        _ => None,
    }
}
