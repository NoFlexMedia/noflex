// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::UserId;
use noflex_storage::{
    domain::ExternalIdentity,
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_text, col_text_opt, dt_to_text, text_to_dt, uuid_to_blob,
    },
    error::map_turso,
};

const COLUMNS: &str =
    "plugin_id, issuer, subject, user_id, username, display_name, email, linked_at";

pub(crate) async fn get(
    conn: &Connection,
    plugin_id: &str,
    issuer: &str,
    subject: &str,
) -> StorageResult<Option<ExternalIdentity>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM external_identities WHERE plugin_id=?1 AND issuer=?2 AND subject=?3"
    );
    let mut rows = conn
        .query(
            &sql,
            vec![
                Value::Text(plugin_id.to_owned()),
                Value::Text(issuer.to_owned()),
                Value::Text(subject.to_owned()),
            ],
        )
        .await
        .map_err(map_turso)?;
    rows.next()
        .await
        .map_err(map_turso)?
        .map(|row| from_row(&row))
        .transpose()
}

pub(crate) async fn list(conn: &Connection, user: UserId) -> StorageResult<Vec<ExternalIdentity>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM external_identities WHERE user_id=?1 ORDER BY plugin_id, issuer, subject"
    );
    let mut rows = conn
        .query(&sql, vec![Value::Blob(uuid_to_blob(user.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(from_row(&row)?);
    }
    Ok(out)
}

pub(crate) async fn link(conn: &Connection, identity: &ExternalIdentity) -> StorageResult<()> {
    // Checked up front so the UNIQUE(user_id, plugin_id) violation surfaces
    // as a Conflict rather than an anonymous constraint error; the single
    // writer connection makes check-then-insert race-free.
    let mut rows = conn
        .query(
            "SELECT 1 FROM external_identities WHERE user_id=?1 AND plugin_id=?2 AND NOT (issuer=?3 AND subject=?4)",
            vec![
                Value::Blob(uuid_to_blob(identity.user_id.into_uuid())),
                identity.plugin_id.clone().into(),
                identity.issuer.clone().into(),
                identity.subject.clone().into(),
            ],
        )
        .await
        .map_err(map_turso)?;
    if rows.next().await.map_err(map_turso)?.is_some() {
        return Err(StorageError::Conflict(
            "user already has an identity for this plugin".into(),
        ));
    }
    drop(rows);
    conn.execute(
        "INSERT INTO external_identities (plugin_id,issuer,subject,user_id,username,display_name,email,linked_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(plugin_id,issuer,subject) DO UPDATE SET username=excluded.username,display_name=excluded.display_name,email=excluded.email WHERE user_id=excluded.user_id",
        vec![
            identity.plugin_id.clone().into(),
            identity.issuer.clone().into(),
            identity.subject.clone().into(),
            Value::Blob(uuid_to_blob(identity.user_id.into_uuid())),
            identity.username.clone().map(Value::Text).unwrap_or(Value::Null),
            identity.display_name.clone().map(Value::Text).unwrap_or(Value::Null),
            identity.email.clone().map(Value::Text).unwrap_or(Value::Null),
            dt_to_text(identity.linked_at)?.into(),
        ],
    ).await.map_err(map_turso)?;
    let found = get(
        conn,
        &identity.plugin_id,
        &identity.issuer,
        &identity.subject,
    )
    .await?;
    if found
        .as_ref()
        .is_some_and(|row| row.user_id != identity.user_id)
    {
        return Err(StorageError::Conflict(
            "external identity belongs to another user".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn unlink(
    conn: &Connection,
    user: UserId,
    plugin_id: &str,
) -> StorageResult<bool> {
    let changed = conn
        .execute(
            "DELETE FROM external_identities WHERE user_id=?1 AND plugin_id=?2",
            vec![
                Value::Blob(uuid_to_blob(user.into_uuid())),
                plugin_id.to_owned().into(),
            ],
        )
        .await
        .map_err(map_turso)?;
    Ok(changed > 0)
}

fn from_row(row: &turso::Row) -> StorageResult<ExternalIdentity> {
    Ok(ExternalIdentity {
        plugin_id: col_text(&row.get_value(0).map_err(map_turso)?)?.to_owned(),
        issuer: col_text(&row.get_value(1).map_err(map_turso)?)?.to_owned(),
        subject: col_text(&row.get_value(2).map_err(map_turso)?)?.to_owned(),
        user_id: UserId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(3).map_err(map_turso)?,
        )?)?),
        username: col_text_opt(&row.get_value(4).map_err(map_turso)?)?,
        display_name: col_text_opt(&row.get_value(5).map_err(map_turso)?)?,
        email: col_text_opt(&row.get_value(6).map_err(map_turso)?)?,
        linked_at: text_to_dt(col_text(&row.get_value(7).map_err(map_turso)?)?)?,
    })
}
