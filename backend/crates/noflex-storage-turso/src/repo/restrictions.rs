// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{EditionId, MediaFileId, TagId, UserId, WorkId};
use noflex_storage::{
    domain::{UserRestrictions, Work},
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{blob_to_uuid, col_blob, col_int, col_int_opt, uuid_to_blob},
    error::map_turso,
};

pub(crate) async fn get(
    conn: &Connection,
    user: UserId,
) -> StorageResult<Option<UserRestrictions>> {
    let user_value = Value::Blob(uuid_to_blob(user.into_uuid()));
    let mut rows = conn.query(
        "SELECT allow_unrated, max_stream_bitrate_bps, max_concurrent_transcodes FROM user_restrictions WHERE user_id = ?1",
        vec![user_value.clone()],
    ).await.map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let allow = row.get_value(0).map_err(map_turso)?;
    let bitrate = row.get_value(1).map_err(map_turso)?;
    let concurrent = row.get_value(2).map_err(map_turso)?;
    let mut rating_rows = conn
        .query(
            "SELECT tag_id FROM user_restriction_ratings WHERE user_id = ?1 ORDER BY tag_id",
            vec![user_value],
        )
        .await
        .map_err(map_turso)?;
    let mut ratings = Vec::new();
    while let Some(row) = rating_rows.next().await.map_err(map_turso)? {
        let value = row.get_value(0).map_err(map_turso)?;
        ratings.push(TagId::from_uuid(blob_to_uuid(col_blob(&value)?)?));
    }
    Ok(Some(UserRestrictions {
        user_id: user,
        allowed_parental_rating_ids: ratings,
        allow_unrated: col_int(&allow)? != 0,
        max_stream_bitrate_bps: col_int_opt(&bitrate)?
            .map(|v| {
                u64::try_from(v)
                    .map_err(|_| StorageError::InvalidInput("negative bitrate policy".into()))
            })
            .transpose()?,
        max_concurrent_transcodes: col_int_opt(&concurrent)?
            .map(|v| {
                u32::try_from(v)
                    .map_err(|_| StorageError::InvalidInput("invalid transcode limit".into()))
            })
            .transpose()?,
    }))
}

pub(crate) async fn set(conn: &Connection, policy: &UserRestrictions) -> StorageResult<()> {
    let bitrate = policy
        .max_stream_bitrate_bps
        .map(|v| {
            i64::try_from(v)
                .map_err(|_| StorageError::InvalidInput("max bitrate exceeds storage range".into()))
        })
        .transpose()?;
    let user = Value::Blob(uuid_to_blob(policy.user_id.into_uuid()));
    conn.execute(
            "INSERT INTO user_restrictions(user_id, allow_unrated, max_stream_bitrate_bps, max_concurrent_transcodes) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(user_id) DO UPDATE SET allow_unrated=excluded.allow_unrated, max_stream_bitrate_bps=excluded.max_stream_bitrate_bps, max_concurrent_transcodes=excluded.max_concurrent_transcodes",
            vec![user.clone(), Value::Integer(policy.allow_unrated as i64), bitrate.map_or(Value::Null, Value::Integer), policy.max_concurrent_transcodes.map_or(Value::Null, |v| Value::Integer(i64::from(v)))],
        ).await.map_err(map_turso)?;
    conn.execute(
        "DELETE FROM user_restriction_ratings WHERE user_id = ?1",
        vec![user.clone()],
    )
    .await
    .map_err(map_turso)?;
    for tag in &policy.allowed_parental_rating_ids {
        let changed = conn.execute(
                "INSERT INTO user_restriction_ratings(user_id, tag_id) SELECT ?1, id FROM tags WHERE id = ?2 AND namespace = 'parental'",
                vec![user.clone(), Value::Blob(uuid_to_blob(tag.into_uuid()))],
            ).await.map_err(map_turso)?;
        if changed == 0 {
            return Err(StorageError::InvalidInput(
                "allowed rating tag must exist in the parental namespace".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) async fn delete(conn: &Connection, user: UserId) -> StorageResult<bool> {
    Ok(conn
        .execute(
            "DELETE FROM user_restrictions WHERE user_id = ?1",
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?
        != 0)
}

pub(crate) async fn work_for_edition(
    conn: &Connection,
    edition: EditionId,
) -> StorageResult<Option<Work>> {
    let mut rows = conn
        .query(
            "SELECT work_id FROM editions WHERE id = ?1",
            vec![Value::Blob(uuid_to_blob(edition.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let value = row.get_value(0).map_err(map_turso)?;
    crate::repo::works::get(conn, WorkId::from_uuid(blob_to_uuid(col_blob(&value)?)?)).await
}

pub(crate) async fn work_for_media(
    conn: &Connection,
    media: MediaFileId,
) -> StorageResult<Option<Work>> {
    let mut rows = conn.query("SELECT e.work_id FROM media_files m JOIN editions e ON e.id=m.edition_id WHERE m.id=?1", vec![Value::Blob(uuid_to_blob(media.into_uuid()))]).await.map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let value = row.get_value(0).map_err(map_turso)?;
    crate::repo::works::get(conn, WorkId::from_uuid(blob_to_uuid(col_blob(&value)?)?)).await
}

/// The single SQL definition of work visibility for user `?{user_param}`:
/// administrators see everything; everyone else needs a library grant
/// and, when a restriction row exists, must pass the parental policy
/// documented on `UserRestrictions`. Callers bind the user id blob at
/// `user_param`.
pub(crate) fn accessible_work_predicate(work_alias: &str, user_param: usize) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM users u WHERE u.id = ?{user_param} AND u.is_admin = 1) OR (\
         EXISTS (SELECT 1 FROM user_library_access ula WHERE ula.user_id = ?{user_param} AND ula.library_id = {work_alias}.library_id) AND (\
         NOT EXISTS (SELECT 1 FROM user_restrictions ur WHERE ur.user_id = ?{user_param}) OR \
         (NOT EXISTS (SELECT 1 FROM tag_assignments ta JOIN tags t ON t.id = ta.tag_id WHERE ta.target_kind = 'work' AND ta.target_id = {work_alias}.id AND t.namespace = 'parental') \
          AND EXISTS (SELECT 1 FROM user_restrictions ur WHERE ur.user_id = ?{user_param} AND ur.allow_unrated = 1)) OR \
         (EXISTS (SELECT 1 FROM tag_assignments ta JOIN tags t ON t.id = ta.tag_id WHERE ta.target_kind = 'work' AND ta.target_id = {work_alias}.id AND t.namespace = 'parental') \
          AND (NOT EXISTS (SELECT 1 FROM user_restriction_ratings urr WHERE urr.user_id = ?{user_param}) OR \
               NOT EXISTS (SELECT 1 FROM tag_assignments ta JOIN tags t ON t.id = ta.tag_id WHERE ta.target_kind = 'work' AND ta.target_id = {work_alias}.id AND t.namespace = 'parental' AND NOT EXISTS (SELECT 1 FROM user_restriction_ratings urr WHERE urr.user_id = ?{user_param} AND urr.tag_id = ta.tag_id)))))))"
    )
}

pub(crate) async fn accessible_work_ids(
    conn: &Connection,
    user: UserId,
    works: &[WorkId],
) -> StorageResult<Vec<WorkId>> {
    if works.is_empty() {
        return Ok(Vec::new());
    }
    let user_param = works.len() + 1;
    let placeholders = (1..=works.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let predicate = accessible_work_predicate("w", user_param);
    let sql = format!("SELECT w.id FROM works w WHERE w.id IN ({placeholders}) AND {predicate}");
    let mut params = works
        .iter()
        .map(|work| Value::Blob(uuid_to_blob(work.into_uuid())))
        .collect::<Vec<_>>();
    params.push(Value::Blob(uuid_to_blob(user.into_uuid())));
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut accessible = std::collections::HashSet::with_capacity(works.len());
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = row.get_value(0).map_err(map_turso)?;
        accessible.insert(WorkId::from_uuid(blob_to_uuid(col_blob(&id)?)?));
    }
    Ok(works
        .iter()
        .copied()
        .filter(|work| accessible.contains(work))
        .collect())
}

pub(crate) async fn can_access(
    conn: &Connection,
    user: UserId,
    work: WorkId,
) -> StorageResult<bool> {
    Ok(!accessible_work_ids(conn, user, &[work]).await?.is_empty())
}
