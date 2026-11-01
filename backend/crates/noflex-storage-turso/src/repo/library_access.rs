// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{LibraryId, UserId};
use noflex_storage::error::StorageResult;
use turso::{Connection, Value};

use crate::{convert::uuid_to_blob, error::map_turso};

const GRANT: &str = "INSERT INTO user_library_access (user_id, library_id, granted_at) \
    VALUES (?1, ?2, ?3) \
    ON CONFLICT(user_id, library_id) DO NOTHING";

const REVOKE: &str = "DELETE FROM user_library_access WHERE user_id = ?1 AND library_id = ?2";

const LIST_FOR_USER: &str = "SELECT library_id FROM user_library_access WHERE user_id = ?1";

pub(crate) async fn grant(
    conn: &Connection,
    user: UserId,
    library: LibraryId,
) -> StorageResult<()> {
    let now = time::OffsetDateTime::now_utc();
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(user.into_uuid())),
        Value::Blob(uuid_to_blob(library.into_uuid())),
        Value::Text(crate::convert::dt_to_text(now)?),
    ];
    conn.execute(GRANT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn revoke(
    conn: &Connection,
    user: UserId,
    library: LibraryId,
) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(user.into_uuid())),
        Value::Blob(uuid_to_blob(library.into_uuid())),
    ];
    conn.execute(REVOKE, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn list_for_user(
    conn: &Connection,
    user: UserId,
) -> StorageResult<Vec<LibraryId>> {
    let mut rows = conn
        .query(
            LIST_FOR_USER,
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = row.get_value(0).map_err(map_turso)?;
        let bytes = match id {
            Value::Blob(b) => b,
            _ => continue,
        };
        let uuid = match bytes.len() {
            16 => {
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&bytes);
                uuid::Uuid::from_bytes(arr)
            }
            _ => continue,
        };
        out.push(LibraryId::from_uuid(uuid));
    }
    Ok(out)
}
