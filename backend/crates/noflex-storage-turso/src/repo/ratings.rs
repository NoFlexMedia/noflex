// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{WorkId, providers::Rating};
use noflex_storage::{
    domain::{Work, WorkRating},
    error::{StorageError, StorageResult},
    query::{Page, PageCursor},
};
use turso::{Connection, Value};

use crate::{
    convert::{col_int_opt, col_real, col_text, uuid_to_blob},
    error::map_turso,
    repo::{
        images::{HAS_PROVIDER_ID, WORK_COLUMNS, page_of_works},
        search::parse_missing_cursor,
    },
};

/// The normalized rating of the work `works.id` names, from its first row in
/// `rating_sources` order.
const DERIVED: &str = "(SELECT r.normalized FROM work_ratings r \
    JOIN rating_sources s ON s.source = r.source \
    WHERE r.work_id = works.id ORDER BY s.rank LIMIT 1)";

/// `locked_fields_json` holds snake_case `WorkField` names.
const UNLOCKED: &str = r#"instr(works.locked_fields_json, '"community_rating"') = 0"#;

const DELETE_PROVIDERS: &str = "DELETE FROM work_ratings WHERE work_id = ?1 AND provider = ?2";

const UPSERT: &str = "INSERT INTO work_ratings \
    (work_id, source, provider, value, scale, normalized, votes) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
    ON CONFLICT(work_id, source) DO UPDATE SET \
        provider = excluded.provider, \
        value = excluded.value, \
        scale = excluded.scale, \
        normalized = excluded.normalized, \
        votes = excluded.votes";

const CHECKED: &str = "INSERT INTO work_rating_checks (work_id, provider) VALUES (?1, ?2) \
    ON CONFLICT(work_id, provider) DO NOTHING";

const LIST: &str = "SELECT r.source, r.value, r.scale, r.normalized, r.votes, r.provider \
    FROM work_ratings r LEFT JOIN rating_sources s ON s.source = r.source \
    WHERE r.work_id = ?1 \
    ORDER BY s.rank IS NULL, s.rank, r.source";

/// Take `work`'s `community_rating` from its rows again, or every work's when
/// `work` is `None`. A locked rating is left alone, as is one already right.
pub(crate) async fn derive(conn: &Connection, work: Option<&[u8]>) -> StorageResult<()> {
    match work {
        Some(work) => {
            let sql = format!(
                "UPDATE works SET community_rating = {DERIVED} \
                 WHERE id = ?1 AND {UNLOCKED} AND community_rating IS NOT {DERIVED}"
            );
            conn.execute(&sql, [Value::Blob(work.to_vec())])
                .await
                .map_err(map_turso)?;
        }
        None => {
            let sql = format!(
                "UPDATE works SET community_rating = {DERIVED} \
                 WHERE {UNLOCKED} AND community_rating IS NOT {DERIVED}"
            );
            conn.execute(&sql, ()).await.map_err(map_turso)?;
        }
    }
    Ok(())
}

pub(crate) async fn replace(
    conn: &Connection,
    work: WorkId,
    provider: &str,
    ratings: &[Rating],
) -> StorageResult<()> {
    let id = uuid_to_blob(work.into_uuid());
    let mut rows = conn
        .query(
            "SELECT 1 FROM works WHERE id = ?1",
            [Value::Blob(id.clone())],
        )
        .await
        .map_err(map_turso)?;
    if rows.next().await.map_err(map_turso)?.is_none() {
        return Err(StorageError::NotFound);
    }
    drop(rows);
    let provider_value = Value::Text(provider.to_string());
    conn.execute(
        DELETE_PROVIDERS,
        [Value::Blob(id.clone()), provider_value.clone()],
    )
    .await
    .map_err(map_turso)?;
    for rating in ratings {
        let votes = match rating.votes {
            Some(votes) => Value::Integer(i64::try_from(votes).unwrap_or(i64::MAX)),
            None => Value::Null,
        };
        conn.execute(
            UPSERT,
            vec![
                Value::Blob(id.clone()),
                Value::Text(rating.source.clone()),
                provider_value.clone(),
                Value::Real(f64::from(rating.value)),
                Value::Real(f64::from(rating.scale)),
                Value::Real(f64::from(rating.normalized())),
                votes,
            ],
        )
        .await
        .map_err(map_turso)?;
    }
    conn.execute(CHECKED, [Value::Blob(id.clone()), provider_value])
        .await
        .map_err(map_turso)?;
    derive(conn, Some(&id)).await
}

pub(crate) async fn list(conn: &Connection, work: WorkId) -> StorageResult<Vec<WorkRating>> {
    let mut rows = conn
        .query(LIST, [Value::Blob(uuid_to_blob(work.into_uuid()))])
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let votes = col_int_opt(&row.get_value(4).map_err(map_turso)?)?;
        out.push(WorkRating {
            source: col_text(&row.get_value(0).map_err(map_turso)?)?.to_string(),
            value: col_real(&row.get_value(1).map_err(map_turso)?)?,
            scale: col_real(&row.get_value(2).map_err(map_turso)?)?,
            normalized: col_real(&row.get_value(3).map_err(map_turso)?)?,
            votes: votes.and_then(|votes| u64::try_from(votes).ok()),
            provider: col_text(&row.get_value(5).map_err(map_turso)?)?.to_string(),
        });
    }
    Ok(out)
}

pub(crate) async fn set_sources(conn: &Connection, sources: &[String]) -> StorageResult<()> {
    conn.execute("DELETE FROM rating_sources", ())
        .await
        .map_err(map_turso)?;
    for (rank, source) in sources.iter().enumerate() {
        conn.execute(
            "INSERT INTO rating_sources (rank, source) VALUES (?1, ?2) \
             ON CONFLICT(source) DO NOTHING",
            vec![Value::Integer(rank as i64), Value::Text(source.clone())],
        )
        .await
        .map_err(map_turso)?;
    }
    derive(conn, None).await
}

pub(crate) async fn identified_unrated_by(
    conn: &Connection,
    provider: &str,
    limit: u32,
    cursor: Option<&PageCursor>,
) -> StorageResult<Page<Work>> {
    let mut params = vec![Value::Text(provider.to_string())];
    let mut sql = format!(
        "SELECT {WORK_COLUMNS} FROM works w \
         WHERE {HAS_PROVIDER_ID} AND NOT EXISTS (SELECT 1 FROM work_rating_checks c \
             WHERE c.work_id = w.id AND c.provider = ?1)"
    );
    if let Some(cursor) = cursor {
        params.push(Value::Blob(parse_missing_cursor(cursor, "rating")?));
        sql.push_str(" AND w.id > ?2");
    }
    page_of_works(conn, sql, params, limit).await
}
