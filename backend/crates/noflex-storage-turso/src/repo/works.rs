// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;

use noflex_core::{ExternalId, ExternalIdKind, LibraryId, WorkField, WorkId};
use noflex_storage::{domain::Work, error::StorageResult};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_blob_opt, col_int_opt, col_real_opt, col_text, col_text_opt,
        dt_to_text, ext_id_kind_to_text, metadata_status_to_text, parse_json, text_to_dt,
        text_to_metadata_status, text_to_work_kind, to_json, uuid_to_blob, work_kind_to_text,
    },
    error::map_turso,
};

/// Leaves an unchanged row alone, `updated_at` included, so a rescan
/// rewrites neither it nor its search index. Every column the update sets
/// belongs in its WHERE, or a change to that column alone is dropped.
const UPSERT_WORK: &str = "INSERT INTO works (\
    id, kind, parent_id, library_id, title, sort_title, year, release_date, overview, \
    external_ids_json, index_number, index_end, parent_index, metadata_status, added_at, updated_at, \
    tagline, original_title, community_rating, locked_fields_json, metadata_provider, root_id\
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22) \
ON CONFLICT(id) DO UPDATE SET \
    kind = excluded.kind, \
    parent_id = excluded.parent_id, \
    root_id = excluded.root_id, \
    library_id = excluded.library_id, \
    title = excluded.title, \
    sort_title = excluded.sort_title, \
    year = excluded.year, \
    release_date = excluded.release_date, \
    overview = excluded.overview, \
    external_ids_json = excluded.external_ids_json, \
    index_number = excluded.index_number, \
    index_end = excluded.index_end, \
    parent_index = excluded.parent_index, \
    metadata_status = excluded.metadata_status, \
    tagline = excluded.tagline, \
    original_title = excluded.original_title, \
    community_rating = excluded.community_rating, \
    locked_fields_json = excluded.locked_fields_json, \
    metadata_provider = excluded.metadata_provider, \
    updated_at = excluded.updated_at \
WHERE works.kind IS NOT excluded.kind OR \
    works.parent_id IS NOT excluded.parent_id OR \
    works.root_id IS NOT excluded.root_id OR \
    works.library_id IS NOT excluded.library_id OR \
    works.title IS NOT excluded.title OR \
    works.sort_title IS NOT excluded.sort_title OR \
    works.year IS NOT excluded.year OR \
    works.release_date IS NOT excluded.release_date OR \
    works.overview IS NOT excluded.overview OR \
    works.external_ids_json IS NOT excluded.external_ids_json OR \
    works.index_number IS NOT excluded.index_number OR \
    works.index_end IS NOT excluded.index_end OR \
    works.parent_index IS NOT excluded.parent_index OR \
    works.metadata_status IS NOT excluded.metadata_status OR \
    works.tagline IS NOT excluded.tagline OR \
    works.original_title IS NOT excluded.original_title OR \
    works.community_rating IS NOT excluded.community_rating OR \
    works.locked_fields_json IS NOT excluded.locked_fields_json OR \
    works.metadata_provider IS NOT excluded.metadata_provider";

const GET_BY_ID: &str = "SELECT id, kind, parent_id, library_id, title, sort_title, year, \
    release_date, overview, external_ids_json, index_number, index_end, parent_index, \
    metadata_status, added_at, updated_at, tagline, original_title, community_rating, \
    locked_fields_json, metadata_provider FROM works WHERE id = ?1";

const DELETE: &str = "DELETE FROM works WHERE id = ?1";

const DELETE_EXT_IDS: &str = "DELETE FROM work_external_ids WHERE work_id = ?1";

const INSERT_EXT_ID: &str = "INSERT INTO work_external_ids(kind, value, work_id) VALUES (?1,?2,?3) \
    ON CONFLICT(kind, value) DO UPDATE SET work_id = excluded.work_id";

const BY_EXT_ID: &str = "SELECT w.id, w.kind, w.parent_id, w.library_id, w.title, w.sort_title, \
    w.year, w.release_date, w.overview, w.external_ids_json, w.index_number, w.index_end, \
    w.parent_index, w.metadata_status, w.added_at, w.updated_at, w.tagline, w.original_title, \
    w.community_rating, w.locked_fields_json, w.metadata_provider \
    FROM works w JOIN work_external_ids x ON x.work_id = w.id \
    WHERE x.kind = ?1 AND x.value = ?2";

/// `work`'s stored root, if it has been written.
async fn stored_root(conn: &Connection, work: &[u8]) -> StorageResult<Option<Vec<u8>>> {
    let mut rows = conn
        .query(
            "SELECT root_id FROM works WHERE id = ?1",
            vec![Value::Blob(work.to_vec())],
        )
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    match row.get_value(0).map_err(map_turso)? {
        Value::Blob(root) => Ok(Some(root)),
        _ => Ok(None),
    }
}

pub(crate) async fn upsert(conn: &Connection, work: &Work) -> StorageResult<()> {
    let id_blob = uuid_to_blob(work.id.into_uuid());
    // A parent not written yet counts as a root until it arrives and corrects this.
    let root = match work.parent_id {
        Some(parent) => {
            let parent = uuid_to_blob(parent.into_uuid());
            stored_root(conn, &parent).await?.unwrap_or(parent)
        }
        None => id_blob.clone(),
    };
    let previous_root = stored_root(conn, &id_blob).await?;
    let params: Vec<Value> = vec![
        Value::Blob(id_blob.clone()),
        Value::Text(work_kind_to_text(work.kind).to_string()),
        match work.parent_id {
            Some(p) => Value::Blob(uuid_to_blob(p.into_uuid())),
            None => Value::Null,
        },
        Value::Blob(uuid_to_blob(work.library_id.into_uuid())),
        Value::Text(work.title.clone()),
        Value::Text(work.sort_title.clone()),
        work.year
            .map(|y| Value::Integer(y as i64))
            .unwrap_or(Value::Null),
        work.release_date
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        work.overview
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Value::Text(to_json(&work.external_ids)?),
        work.index_number
            .map(|n| Value::Integer(n as i64))
            .unwrap_or(Value::Null),
        work.index_end
            .map(|n| Value::Integer(n as i64))
            .unwrap_or(Value::Null),
        work.parent_index
            .map(|n| Value::Integer(n as i64))
            .unwrap_or(Value::Null),
        Value::Text(metadata_status_to_text(work.metadata_status).to_string()),
        Value::Text(dt_to_text(work.added_at)?),
        Value::Text(dt_to_text(work.updated_at)?),
        work.tagline.clone().map(Value::Text).unwrap_or(Value::Null),
        work.original_title
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        work.community_rating
            .map(|r| Value::Real(r as f64))
            .unwrap_or(Value::Null),
        Value::Text(to_json(&work.locked_fields)?),
        work.metadata_provider
            .clone()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Value::Blob(root.clone()),
    ];
    let written = conn.execute(UPSERT_WORK, params).await.map_err(map_turso)?;
    if written == 0 {
        return Ok(());
    }

    // Whatever sits beneath it moved too. Parents nest two levels at most.
    if previous_root.as_ref() != Some(&root) {
        for sql in [
            "UPDATE works SET root_id = ?1 WHERE parent_id = ?2",
            "UPDATE works SET root_id = ?1 WHERE parent_id IN (SELECT id FROM works WHERE parent_id = ?2)",
        ] {
            conn.execute(
                sql,
                vec![Value::Blob(root.clone()), Value::Blob(id_blob.clone())],
            )
            .await
            .map_err(map_turso)?;
        }
    }

    // Rebuild the work_external_ids rows from scratch — simplest correct path
    // since the canonical truth is the json column on `works`.
    conn.execute(DELETE_EXT_IDS, vec![Value::Blob(id_blob.clone())])
        .await
        .map_err(map_turso)?;
    for ext in &work.external_ids {
        let p: Vec<Value> = vec![
            Value::Text(ext_id_kind_to_text(ext.kind).to_string()),
            Value::Text(ext.value.clone()),
            Value::Blob(id_blob.clone()),
        ];
        conn.execute(INSERT_EXT_ID, p).await.map_err(map_turso)?;
    }
    Ok(())
}

pub(crate) async fn get(conn: &Connection, id: WorkId) -> StorageResult<Option<Work>> {
    let mut rows = conn
        .query(GET_BY_ID, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(Some(row_to_work(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_many(conn: &Connection, ids: &[WorkId]) -> StorageResult<Vec<Work>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, kind, parent_id, library_id, title, sort_title, year, \
         release_date, overview, external_ids_json, index_number, index_end, parent_index, \
         metadata_status, added_at, updated_at, tagline, original_title, community_rating, \
         locked_fields_json, metadata_provider FROM works WHERE id IN ({placeholders})"
    );
    let params = ids
        .iter()
        .map(|id| Value::Blob(uuid_to_blob(id.into_uuid())))
        .collect::<Vec<_>>();
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut by_id = HashMap::with_capacity(ids.len());
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let work = row_to_work(&row)?;
        by_id.insert(work.id, work);
    }
    Ok(ids.iter().filter_map(|id| by_id.get(id).cloned()).collect())
}

pub(crate) async fn delete(conn: &Connection, id: WorkId) -> StorageResult<()> {
    conn.execute(DELETE, vec![Value::Blob(uuid_to_blob(id.into_uuid()))])
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn by_external_id(
    conn: &Connection,
    kind: ExternalIdKind,
    value: &str,
) -> StorageResult<Vec<Work>> {
    let params: Vec<Value> = vec![
        Value::Text(ext_id_kind_to_text(kind).to_string()),
        Value::Text(value.to_string()),
    ];
    let mut rows = conn.query(BY_EXT_ID, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(row_to_work(&row)?);
    }
    Ok(out)
}

pub(crate) fn row_to_work(row: &turso::Row) -> StorageResult<Work> {
    let id = row.get_value(0).map_err(map_turso)?;
    let kind = row.get_value(1).map_err(map_turso)?;
    let parent_id = row.get_value(2).map_err(map_turso)?;
    let library_id = row.get_value(3).map_err(map_turso)?;
    let title = row.get_value(4).map_err(map_turso)?;
    let sort_title = row.get_value(5).map_err(map_turso)?;
    let year = row.get_value(6).map_err(map_turso)?;
    let release_date = row.get_value(7).map_err(map_turso)?;
    let overview = row.get_value(8).map_err(map_turso)?;
    let ext_ids = row.get_value(9).map_err(map_turso)?;
    let index_number = row.get_value(10).map_err(map_turso)?;
    let index_end = row.get_value(11).map_err(map_turso)?;
    let parent_index = row.get_value(12).map_err(map_turso)?;
    let metadata_status = row.get_value(13).map_err(map_turso)?;
    let added_at = row.get_value(14).map_err(map_turso)?;
    let updated_at = row.get_value(15).map_err(map_turso)?;
    let tagline = row.get_value(16).map_err(map_turso)?;
    let original_title = row.get_value(17).map_err(map_turso)?;
    let community_rating = row.get_value(18).map_err(map_turso)?;
    let locked_fields = row.get_value(19).map_err(map_turso)?;
    let metadata_provider = row.get_value(20).map_err(map_turso)?;

    let parent = col_blob_opt(&parent_id)?
        .map(|b| blob_to_uuid(&b).map(WorkId::from_uuid))
        .transpose()?;
    let external_ids: Vec<ExternalId> = parse_json(col_text(&ext_ids)?)?;
    let locked_fields: Vec<WorkField> = parse_json(col_text(&locked_fields)?)?;

    Ok(Work {
        id: WorkId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
        kind: text_to_work_kind(col_text(&kind)?)?,
        parent_id: parent,
        library_id: LibraryId::from_uuid(blob_to_uuid(col_blob(&library_id)?)?),
        title: col_text(&title)?.to_string(),
        sort_title: col_text(&sort_title)?.to_string(),
        year: col_int_opt(&year)?.map(|i| i as i32),
        release_date: col_text_opt(&release_date)?,
        overview: col_text_opt(&overview)?,
        external_ids,
        index_number: col_int_opt(&index_number)?.map(|i| i as i32),
        index_end: col_int_opt(&index_end)?.map(|i| i as i32),
        parent_index: col_int_opt(&parent_index)?.map(|i| i as i32),
        metadata_status: text_to_metadata_status(col_text(&metadata_status)?)?,
        tagline: col_text_opt(&tagline)?,
        original_title: col_text_opt(&original_title)?,
        community_rating: col_real_opt(&community_rating)?,
        locked_fields,
        metadata_provider: col_text_opt(&metadata_provider)?,
        added_at: text_to_dt(col_text(&added_at)?)?,
        updated_at: text_to_dt(col_text(&updated_at)?)?,
    })
}

#[cfg(test)]
mod tests {
    use noflex_core::{MetadataStatus, WorkKind};
    use noflex_storage::{
        Storage,
        domain::{Library, LibraryKind},
    };
    use time::{Duration, OffsetDateTime};

    use super::*;
    use crate::TursoStore;

    #[tokio::test]
    async fn rewriting_an_unchanged_work_leaves_it_alone_but_one_change_lands() {
        let store = TursoStore::open_temp().await.unwrap();
        store.migrate().await.unwrap();
        let library = Library {
            id: LibraryId::new(),
            name: "Films".to_string(),
            kind: LibraryKind::Movies,
            paths: vec!["/srv/films".to_string()],
            ..Default::default()
        };
        store.upsert_library(&library).await.unwrap();
        let written = OffsetDateTime::now_utc() - Duration::days(1);
        let mut work = Work {
            id: WorkId::new(),
            kind: WorkKind::Movie,
            parent_id: None,
            library_id: library.id,
            title: "Harbor".to_string(),
            sort_title: "harbor".to_string(),
            year: None,
            release_date: None,
            overview: None,
            external_ids: vec![],
            index_number: None,
            index_end: None,
            parent_index: None,
            metadata_status: MetadataStatus::Unconfirmed,
            tagline: None,
            original_title: None,
            community_rating: Some(7.5),
            locked_fields: vec![],
            metadata_provider: None,
            added_at: written,
            updated_at: written,
        };
        store.upsert_work(&work).await.unwrap();

        work.updated_at = OffsetDateTime::now_utc();
        store.upsert_work(&work).await.unwrap();
        let stored = store.get_work(work.id).await.unwrap().unwrap();
        assert_eq!(stored.updated_at, written, "nothing changed");

        // The last column the update compares, and the external ids it skips
        // rebuilding when the row is unchanged.
        work.metadata_provider = Some("tmdb".to_string());
        work.external_ids = vec![ExternalId {
            kind: ExternalIdKind::Tmdb,
            value: "603".to_string(),
        }];
        store.upsert_work(&work).await.unwrap();
        let stored = store.get_work(work.id).await.unwrap().unwrap();
        assert_eq!(stored.metadata_provider.as_deref(), Some("tmdb"));
        assert_eq!(stored.updated_at, work.updated_at);
        let found = store
            .works_by_external_id(ExternalIdKind::Tmdb, "603")
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
    }
}
