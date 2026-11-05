// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{EmbeddingTarget, WorkId, WorkKind};
use noflex_storage::{
    domain::{EmbeddingEndpoint, EmbeddingModel, Work},
    error::{StorageError, StorageResult},
    query::{Page, PageCursor},
    search::{MATCHED_CREDITS, MATCHED_TITLE, SearchHit, VectorHit},
};
use turso::{Connection, Value};
use uuid::Uuid;

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_int_opt, col_text, col_text_opt, dt_to_text,
        uuid_to_blob,
    },
    error::map_turso,
    repo::works::row_to_work,
};

// Works crediting someone whose name holds ?1. Runs once rather than per work:
// people are far fewer than works, and a tag's assignments are keyed by tag.
const CREDITED: &str = "(SELECT ta.target_id FROM tags t \
    JOIN tag_assignments ta ON ta.tag_id = t.id AND ta.target_kind = 'work' \
    WHERE t.namespace = 'person' AND LOWER(t.display) LIKE ?1 ESCAPE '\\')";

const IN_TITLE: &str =
    "(LOWER(w.title) LIKE ?1 ESCAPE '\\' OR LOWER(w.sort_title) LIKE ?1 ESCAPE '\\')";

/// The lexical query up to its filters. ?1 is the query anywhere, ?2 the whole
/// title, ?3 its start, ?4 the start of a word.
fn search_lexical_sql() -> String {
    format!(
        "SELECT w.id, {IN_TITLE} AS in_title, w.id IN {CREDITED} AS in_credits \
         FROM works w \
         LEFT JOIN works parent ON parent.id = w.parent_id \
         LEFT JOIN works grandparent ON grandparent.id = parent.parent_id \
         WHERE ({IN_TITLE} \
            OR LOWER(COALESCE(w.overview, '')) LIKE ?1 ESCAPE '\\' \
            OR w.id IN {CREDITED})"
    )
}

/// A show matching the query already answers it, and its episodes' synopses tend
/// to repeat its name, as their credits do its regular cast. Episodes matching on
/// their own title still count, as do guest stars the show doesn't credit.
fn not_just_echoing_its_series_sql() -> String {
    format!(
        " AND (w.kind NOT IN ('season', 'episode', 'track') OR {IN_TITLE} \
         OR NOT (LOWER(COALESCE(parent.title, '')) LIKE ?1 ESCAPE '\\' \
             OR LOWER(COALESCE(grandparent.title, '')) LIKE ?1 ESCAPE '\\' \
             OR COALESCE(parent.id IN {CREDITED}, 0) \
             OR COALESCE(grandparent.id IN {CREDITED}, 0)))"
    )
}

// Titles, then credits, then synopses; whole titles and series before single episodes.
const LEXICAL_RELEVANCE: &str = " ORDER BY in_title DESC, in_credits DESC, \
    w.kind IN ('season', 'episode', 'track') ASC, \
    CASE WHEN LOWER(w.title) LIKE ?2 ESCAPE '\\' OR LOWER(w.sort_title) LIKE ?2 ESCAPE '\\' THEN 0 \
        WHEN LOWER(w.title) LIKE ?3 ESCAPE '\\' OR LOWER(w.sort_title) LIKE ?3 ESCAPE '\\' THEN 1 \
        WHEN LOWER(w.title) LIKE ?4 ESCAPE '\\' THEN 2 \
        ELSE 3 END, \
    w.added_at DESC";

const EMB_UPSERT: &str = "INSERT INTO embeddings (target_kind, target_id, namespace, model_id, dimensions, vector_blob, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
    ON CONFLICT(target_kind, target_id, namespace, model_id) DO UPDATE SET \
        dimensions = excluded.dimensions, \
        vector_blob = excluded.vector_blob, \
        updated_at = excluded.updated_at";

const EMB_FETCH: &str = "SELECT target_kind, target_id, namespace, model_id, dimensions, vector_blob \
    FROM embeddings WHERE namespace = ?1 AND target_kind = 'work'";

// Kind-filtered variant: joins `works` so the filter applies before
// scoring/truncation. Column order matches EMB_FETCH so the row
// decoding below is shared.
const EMB_FETCH_BY_KIND: &str = "SELECT e.target_kind, e.target_id, e.namespace, e.model_id, e.dimensions, e.vector_blob \
    FROM embeddings e JOIN works w ON e.target_kind = 'work' AND e.target_id = w.id \
    WHERE e.namespace = ?1";

// Anti-join on `embeddings` for the backfill sweep. Ordering by `w.id`
// keeps the keyset cursor stable: a work's id is fixed at upsert, so
// the same page can be re-resumed after a crash.
const MISSING_EMBEDDING: &str = "SELECT w.id, w.kind, w.parent_id, w.library_id, w.title, w.sort_title, \
    w.year, w.release_date, w.overview, w.external_ids_json, w.index_number, w.index_end, \
    w.parent_index, w.metadata_status, w.added_at, w.updated_at, w.tagline, w.original_title, \
    w.community_rating, w.locked_fields_json \
    FROM works w \
    WHERE NOT EXISTS (SELECT 1 FROM embeddings e \
        WHERE e.target_kind = 'work' AND e.target_id = w.id AND e.namespace = ?1)";

pub(crate) async fn search_lexical(
    conn: &Connection,
    query: &str,
    kinds: &[WorkKind],
    access_user_id: Option<noflex_core::UserId>,
    limit: u32,
) -> StorageResult<Vec<SearchHit>> {
    let escaped = escape_like(query);
    let mut sql = search_lexical_sql();
    let mut params = vec![
        Value::Text(format!("%{escaped}%")),
        Value::Text(escaped.clone()),
        Value::Text(format!("{escaped}%")),
        Value::Text(format!("% {escaped}%")),
    ];
    // Asking for episodes by kind means wanting them, however they matched.
    if kinds.is_empty() {
        sql.push_str(&not_just_echoing_its_series_sql());
    }
    push_kind_filter(&mut sql, &mut params, kinds, "w.kind");
    if let Some(user) = access_user_id {
        push_access_filter(&mut sql, &mut params, user, "w");
    }
    sql.push_str(LEXICAL_RELEVANCE);
    sql.push_str(&format!(" LIMIT ?{}", params.len() + 1));
    params.push(Value::Integer(limit as i64));
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = row.get_value(0).map_err(map_turso)?;
        let matched = if col_int(&row.get_value(1).map_err(map_turso)?)? != 0 {
            MATCHED_TITLE
        } else if col_int(&row.get_value(2).map_err(map_turso)?)? != 0 {
            MATCHED_CREDITS
        } else {
            "overview"
        };
        out.push(SearchHit {
            work: WorkId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
            access_checked: access_user_id.is_some(),
            score: 0.0,
            matched_fields: vec![matched.to_string()],
        });
    }
    Ok(out)
}

pub(crate) async fn active_embedding_model(
    conn: &Connection,
) -> StorageResult<Option<EmbeddingModel>> {
    let mut rows = conn
        .query(
            "SELECT namespace, model_id, dimensions FROM embedding_index WHERE id = 1",
            (),
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    Ok(Some(EmbeddingModel {
        namespace: col_text(&row.get_value(0).map_err(map_turso)?)?.to_string(),
        model_id: col_text(&row.get_value(1).map_err(map_turso)?)?.to_string(),
        dimensions: col_int(&row.get_value(2).map_err(map_turso)?)? as u32,
    }))
}

pub(crate) async fn activate_embedding_model(
    conn: &Connection,
    model: &EmbeddingModel,
) -> StorageResult<Option<EmbeddingModel>> {
    let previous = active_embedding_model(conn).await?;
    if previous.as_ref() == Some(model) {
        return Ok(None);
    }
    conn.execute(
        "DELETE FROM embeddings WHERE namespace = ?1 AND (model_id <> ?2 OR dimensions <> ?3)",
        vec![
            Value::Text(model.namespace.clone()),
            Value::Text(model.model_id.clone()),
            Value::Integer(i64::from(model.dimensions)),
        ],
    )
    .await
    .map_err(map_turso)?;
    conn.execute(
        "INSERT INTO embedding_index (id, namespace, model_id, dimensions, activated_at) \
         VALUES (1, ?1, ?2, ?3, ?4) \
         ON CONFLICT(id) DO UPDATE SET namespace = excluded.namespace, \
             model_id = excluded.model_id, dimensions = excluded.dimensions, \
             activated_at = excluded.activated_at",
        vec![
            Value::Text(model.namespace.clone()),
            Value::Text(model.model_id.clone()),
            Value::Integer(i64::from(model.dimensions)),
            Value::Text(dt_to_text(time::OffsetDateTime::now_utc())?),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(previous)
}

pub(crate) async fn prune_embeddings(conn: &Connection, namespace: &str) -> StorageResult<u64> {
    conn.execute(
        "DELETE FROM embeddings WHERE namespace <> ?1",
        vec![Value::Text(namespace.to_string())],
    )
    .await
    .map_err(map_turso)
}

pub(crate) async fn embedding_endpoint(
    conn: &Connection,
) -> StorageResult<Option<EmbeddingEndpoint>> {
    let mut rows = conn
        .query(
            "SELECT url, api_key, model, dimensions, request_dimensions, query_prefix, \
             document_prefix FROM embedding_endpoint WHERE id = 1",
            (),
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    Ok(Some(EmbeddingEndpoint {
        url: col_text(&row.get_value(0).map_err(map_turso)?)?.to_string(),
        api_key: col_text_opt(&row.get_value(1).map_err(map_turso)?)?,
        model: col_text(&row.get_value(2).map_err(map_turso)?)?.to_string(),
        dimensions: col_int(&row.get_value(3).map_err(map_turso)?)? as u32,
        request_dimensions: col_int_opt(&row.get_value(4).map_err(map_turso)?)?
            .map(|dimensions| dimensions as u32),
        query_prefix: col_text(&row.get_value(5).map_err(map_turso)?)?.to_string(),
        document_prefix: col_text(&row.get_value(6).map_err(map_turso)?)?.to_string(),
    }))
}

pub(crate) async fn set_embedding_endpoint(
    conn: &Connection,
    endpoint: Option<&EmbeddingEndpoint>,
) -> StorageResult<()> {
    let Some(endpoint) = endpoint else {
        conn.execute("DELETE FROM embedding_endpoint", ())
            .await
            .map_err(map_turso)?;
        return Ok(());
    };
    conn.execute(
        "INSERT INTO embedding_endpoint (id, url, api_key, model, dimensions, \
             request_dimensions, query_prefix, document_prefix) \
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(id) DO UPDATE SET url = excluded.url, api_key = excluded.api_key, \
             model = excluded.model, dimensions = excluded.dimensions, \
             request_dimensions = excluded.request_dimensions, \
             query_prefix = excluded.query_prefix, document_prefix = excluded.document_prefix",
        vec![
            Value::Text(endpoint.url.clone()),
            endpoint.api_key.clone().map_or(Value::Null, Value::Text),
            Value::Text(endpoint.model.clone()),
            Value::Integer(i64::from(endpoint.dimensions)),
            endpoint
                .request_dimensions
                .map_or(Value::Null, |dimensions| {
                    Value::Integer(i64::from(dimensions))
                }),
            Value::Text(endpoint.query_prefix.clone()),
            Value::Text(endpoint.document_prefix.clone()),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn upsert_embedding(
    conn: &Connection,
    target: EmbeddingTarget,
    namespace: &str,
    model_id: &str,
    vector: &[f32],
) -> StorageResult<()> {
    let (kind, id) = emb_target_parts(&target);
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for f in vector {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    let params: Vec<Value> = vec![
        Value::Text(kind.to_string()),
        Value::Blob(uuid_to_blob(id)),
        Value::Text(namespace.to_string()),
        Value::Text(model_id.to_string()),
        Value::Integer(vector.len() as i64),
        Value::Blob(bytes),
        Value::Text(dt_to_text(time::OffsetDateTime::now_utc())?),
    ];
    conn.execute(EMB_UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn search_vector(
    conn: &Connection,
    namespace: &str,
    query_vector: &[f32],
    kinds: &[WorkKind],
    access_user_id: Option<noflex_core::UserId>,
    limit: u32,
) -> StorageResult<Vec<VectorHit>> {
    let mut params = vec![Value::Text(namespace.to_string())];
    let sql = if kinds.is_empty() && access_user_id.is_none() {
        EMB_FETCH.to_string()
    } else {
        let mut sql = String::from(EMB_FETCH_BY_KIND);
        push_kind_filter(&mut sql, &mut params, kinds, "w.kind");
        if let Some(user) = access_user_id {
            push_access_filter(&mut sql, &mut params, user, "w");
        }
        sql
    };
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut scored: Vec<(Uuid, f32)> = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let target_kind = row.get_value(0).map_err(map_turso)?;
        let target_id_blob = row.get_value(1).map_err(map_turso)?;
        let dims = row.get_value(4).map_err(map_turso)?;
        let vec_blob = row.get_value(5).map_err(map_turso)?;
        let _ = col_text(&target_kind)?;
        let target_id = blob_to_uuid(col_blob(&target_id_blob)?)?;
        let stored_dims = col_int(&dims)?;
        if stored_dims != query_vector.len() as i64 {
            continue;
        }
        let stored_bytes = match vec_blob {
            Value::Blob(b) => b,
            _ => continue,
        };
        let Some(stored) = decode_embedding(&stored_bytes, stored_dims) else {
            tracing::warn!(%target_id, stored_dims, "skipping corrupt embedding");
            continue;
        };
        scored.push((target_id, l2_to_similarity(&stored, query_vector)));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit as usize);
    Ok(scored
        .into_iter()
        .map(|(id, sim)| VectorHit {
            work: WorkId::from_uuid(id),
            similarity: sim,
        })
        .collect())
}

pub(crate) async fn delete_work_embeddings(conn: &Connection, work: WorkId) -> StorageResult<()> {
    conn.execute(
        "DELETE FROM embeddings WHERE target_kind = 'work' AND target_id = ?1",
        vec![Value::Blob(uuid_to_blob(work.into_uuid()))],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn work_embeddings(
    conn: &Connection,
    namespace: &str,
    works: &[WorkId],
) -> StorageResult<Vec<(WorkId, Vec<f32>)>> {
    if works.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (0..works.len())
        .map(|index| format!("?{}", index + 2))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT target_id, dimensions, vector_blob FROM embeddings \
         WHERE namespace = ?1 AND target_kind = 'work' AND target_id IN ({placeholders})"
    );
    let mut params = vec![Value::Text(namespace.to_string())];
    params.extend(
        works
            .iter()
            .map(|work| Value::Blob(uuid_to_blob(work.into_uuid()))),
    );
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let target_id = blob_to_uuid(col_blob(&row.get_value(0).map_err(map_turso)?)?)?;
        let dimensions = col_int(&row.get_value(1).map_err(map_turso)?)?;
        let Value::Blob(bytes) = row.get_value(2).map_err(map_turso)? else {
            continue;
        };
        let Some(vector) = decode_embedding(&bytes, dimensions) else {
            tracing::warn!(%target_id, dimensions, "skipping corrupt embedding");
            continue;
        };
        out.push((WorkId::from_uuid(target_id), vector));
    }
    Ok(out)
}

pub(crate) async fn works_missing_embedding(
    conn: &Connection,
    namespace: &str,
    limit: u32,
    cursor: Option<&PageCursor>,
) -> StorageResult<Page<Work>> {
    let mut sql = String::from(MISSING_EMBEDDING);
    let mut params: Vec<Value> = vec![Value::Text(namespace.to_string())];
    if let Some(c) = cursor {
        let id = parse_missing_cursor(c, "embedding")?;
        sql.push_str(&format!(" AND w.id > ?{}", params.len() + 1));
        params.push(Value::Blob(id));
    }
    sql.push_str(" ORDER BY w.id ASC");
    let limit = limit.max(1) as i64;
    sql.push_str(&format!(" LIMIT {}", limit + 1));

    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut items: Vec<Work> = Vec::new();
    let mut ids: Vec<Vec<u8>> = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = row.get_value(0).map_err(map_turso)?;
        ids.push(col_blob(&id)?.to_vec());
        items.push(row_to_work(&row)?);
    }
    let next_cursor = if items.len() as i64 > limit {
        // Drop the look-ahead row; the cursor keys off the last row
        // the caller actually receives, not the extra one.
        items.pop();
        ids.pop();
        ids.last().map(|id| {
            let hex: String = id.iter().map(|b| format!("{:02x}", b)).collect();
            PageCursor(hex)
        })
    } else {
        None
    };
    Ok(Page { items, next_cursor })
}

/// Cursors are client-supplied: a UUID can't be encoded any other way
/// here, so anything that isn't exactly 32 hex chars is rejected
/// outright — silently dropping a bad cursor would re-serve page one.
pub(crate) fn parse_missing_cursor(c: &PageCursor, kind: &str) -> StorageResult<Vec<u8>> {
    let malformed = || StorageError::InvalidInput(format!("malformed {kind} cursor: {:?}", c.0));
    if c.0.len() != 32 || !c.0.is_ascii() {
        return Err(malformed());
    }
    (0..c.0.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&c.0[i..i + 2], 16).map_err(|_| malformed()))
        .collect()
}

fn emb_target_parts(target: &EmbeddingTarget) -> (&'static str, Uuid) {
    match target {
        EmbeddingTarget::Work(id) => ("work", id.into_uuid()),
        EmbeddingTarget::Edition(id) => ("edition", id.into_uuid()),
        EmbeddingTarget::MediaFile(id) => ("media_file", id.into_uuid()),
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Append `AND <column> IN (?,…)` for a non-empty kinds filter, using
/// the same placeholder convention as `list_works`. Empty kinds means
/// "all kinds", so nothing is appended.
fn push_kind_filter(sql: &mut String, params: &mut Vec<Value>, kinds: &[WorkKind], column: &str) {
    if kinds.is_empty() {
        return;
    }
    let placeholders: Vec<String> = kinds
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", params.len() + 1 + i))
        .collect();
    for k in kinds {
        params.push(Value::Text(
            crate::convert::work_kind_to_text(*k).to_string(),
        ));
    }
    sql.push_str(&format!(" AND {column} IN ({})", placeholders.join(",")));
}

fn push_access_filter(
    sql: &mut String,
    params: &mut Vec<Value>,
    user: noflex_core::UserId,
    works: &str,
) {
    let p = params.len() + 1;
    sql.push_str(" AND ");
    sql.push_str(&crate::repo::restrictions::accessible_work_predicate(
        works, p,
    ));
    params.push(Value::Blob(uuid_to_blob(user.into_uuid())));
}

fn decode_embedding(bytes: &[u8], dimensions: i64) -> Option<Vec<f32>> {
    let dimensions = usize::try_from(dimensions).ok()?;
    if dimensions.checked_mul(std::mem::size_of::<f32>())? != bytes.len() {
        return None;
    }
    let values: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(values)
}

/// `1 / (1 + d)` so the RRF fuser doesn't need to know per-backend scale.
fn l2_to_similarity(a: &[f32], b: &[f32]) -> f32 {
    let sum: f32 = a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum();
    1.0 / (1.0 + sum.sqrt())
}

#[cfg(test)]
mod tests {
    use super::decode_embedding;

    #[test]
    fn embedding_decoder_rejects_corrupt_shapes_and_values() {
        assert!(decode_embedding(&[0; 3], 1).is_none());
        assert!(decode_embedding(&[0; 4], 2).is_none());
        assert!(decode_embedding(&[0; 4], -1).is_none());
        assert!(decode_embedding(&f32::NAN.to_le_bytes(), 1).is_none());
    }

    #[test]
    fn embedding_decoder_reads_little_endian_values() {
        let bytes = [1.25_f32.to_le_bytes(), (-2.5_f32).to_le_bytes()].concat();
        assert_eq!(decode_embedding(&bytes, 2), Some(vec![1.25, -2.5]));
    }
}
