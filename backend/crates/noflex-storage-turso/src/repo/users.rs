// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{UserId, UserRole};
use noflex_storage::{
    domain::User,
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{blob_to_uuid, col_blob, col_int, col_text, dt_to_text, text_to_dt, uuid_to_blob},
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO users (id, name, password_hash, is_admin, role, created_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
    ON CONFLICT(id) DO UPDATE SET \
        name = excluded.name, \
        password_hash = excluded.password_hash, \
        is_admin = excluded.is_admin, \
        role = excluded.role";

const GET_BY_ID: &str = "SELECT id, name, password_hash, is_admin, role, created_at \
    FROM users WHERE id = ?1";

const GET_BY_NAME: &str = "SELECT id, name, password_hash, is_admin, role, created_at \
    FROM users WHERE name = ?1";

const LIST: &str = "SELECT id, name, password_hash, is_admin, role, created_at \
    FROM users ORDER BY name";

const DELETE: &str = "DELETE FROM users WHERE id = ?1";

pub(crate) async fn upsert(conn: &Connection, user: &User) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(user.id.into_uuid())),
        Value::Text(user.name.clone()),
        Value::Text(user.password_hash.clone()),
        Value::Integer(user.is_admin as i64),
        Value::Text(user.role_as_text().to_string()),
        Value::Text(dt_to_text(user.created_at)?),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

const INSERT_IF_EMPTY: &str = "INSERT INTO users (id, name, password_hash, is_admin, role, created_at) \
    SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE NOT EXISTS (SELECT 1 FROM users)";

pub(crate) async fn create_initial_admin(conn: &Connection, user: &User) -> StorageResult<bool> {
    if !user.is_admin || user.role != UserRole::Admin {
        return Err(StorageError::InvalidInput(
            "initial user must be an administrator".into(),
        ));
    }
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(user.id.into_uuid())),
        Value::Text(user.name.clone()),
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
        .query(GET_BY_ID, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_user(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_by_name(conn: &Connection, name: &str) -> StorageResult<Option<User>> {
    let mut rows = conn
        .query(GET_BY_NAME, vec![Value::Text(name.to_string())])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_user(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_all(conn: &Connection) -> StorageResult<Vec<User>> {
    let mut rows = conn.query(LIST, ()).await.map_err(map_turso)?;
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

fn row_to_user(row: &turso::Row) -> StorageResult<User> {
    let id = row.get_value(0).map_err(map_turso)?;
    let name = row.get_value(1).map_err(map_turso)?;
    let password_hash = row.get_value(2).map_err(map_turso)?;
    let is_admin = row.get_value(3).map_err(map_turso)?;
    let role = row.get_value(4).map_err(map_turso)?;
    let created_at = row.get_value(5).map_err(map_turso)?;
    let is_admin = col_int(&is_admin)? != 0;
    let role = text_to_role(col_text(&role)?).unwrap_or(if is_admin {
        UserRole::Admin
    } else {
        UserRole::User
    });
    Ok(User {
        id: UserId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        name: col_text(&name)?.to_string(),
        password_hash: col_text(&password_hash)?.to_string(),
        is_admin,
        role,
        created_at: text_to_dt(col_text(&created_at)?)?,
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
