// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::LibraryId;
use noflex_storage::{
    domain::{ArtworkPreference, Library, ScanSchedule},
    error::{StorageError, StorageResult},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_text, library_kind_to_text, parse_json,
        text_to_library_kind, to_json, uuid_to_blob,
    },
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO libraries (\
        id, name, kind, paths_json, \
        config_languages_json, config_providers_json, config_excluded_json, \
        config_watch_enabled, config_scan_schedule, config_artwork_preference) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
    ON CONFLICT(id) DO UPDATE SET \
        name = excluded.name, \
        kind = excluded.kind, \
        paths_json = excluded.paths_json, \
        config_languages_json = excluded.config_languages_json, \
        config_providers_json = excluded.config_providers_json, \
        config_excluded_json = excluded.config_excluded_json, \
        config_watch_enabled = excluded.config_watch_enabled, \
        config_scan_schedule = excluded.config_scan_schedule, \
        config_artwork_preference = excluded.config_artwork_preference";

const LIST: &str = "SELECT id, name, kind, paths_json, \
        config_languages_json, config_providers_json, config_excluded_json, \
        config_watch_enabled, config_scan_schedule, config_artwork_preference \
    FROM libraries ORDER BY name";

const DELETE: &str = "DELETE FROM libraries WHERE id = ?1";

const SCAN_VERSION: &str = "SELECT scan_version FROM libraries WHERE id = ?1";

const SET_SCAN_VERSION: &str = "UPDATE libraries SET scan_version = ?2 WHERE id = ?1";

fn schedule_to_text(s: &ScanSchedule) -> &'static str {
    match s {
        ScanSchedule::Disabled => "DISABLED",
        ScanSchedule::Hourly => "HOURLY",
        ScanSchedule::Daily => "DAILY",
        ScanSchedule::Weekly => "WEEKLY",
    }
}

fn text_to_schedule(s: &str) -> StorageResult<ScanSchedule> {
    Ok(match s {
        "DISABLED" => ScanSchedule::Disabled,
        "HOURLY" => ScanSchedule::Hourly,
        "DAILY" => ScanSchedule::Daily,
        "WEEKLY" => ScanSchedule::Weekly,
        other => {
            return Err(StorageError::Backend(
                format!("unknown scan schedule: {other}").into(),
            ));
        }
    })
}

fn artwork_to_text(p: ArtworkPreference) -> &'static str {
    match p {
        ArtworkPreference::PreferProvider => "PREFER_PROVIDER",
        ArtworkPreference::PreferLocal => "PREFER_LOCAL",
    }
}

fn text_to_artwork(s: &str) -> StorageResult<ArtworkPreference> {
    Ok(match s {
        "PREFER_PROVIDER" => ArtworkPreference::PreferProvider,
        "PREFER_LOCAL" => ArtworkPreference::PreferLocal,
        other => {
            return Err(StorageError::Backend(
                format!("unknown artwork preference: {other}").into(),
            ));
        }
    })
}

pub(crate) async fn upsert(conn: &Connection, lib: &Library) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(lib.id.into_uuid())),
        Value::Text(lib.name.clone()),
        Value::Text(library_kind_to_text(lib.kind).to_string()),
        Value::Text(to_json(&lib.paths)?),
        Value::Text(to_json(&lib.config.preferred_languages)?),
        Value::Text(to_json(&lib.config.preferred_providers)?),
        Value::Text(to_json(&lib.config.excluded_folders)?),
        Value::Integer(if lib.config.watch_enabled { 1 } else { 0 }),
        Value::Text(schedule_to_text(&lib.config.scan_schedule).to_string()),
        Value::Text(artwork_to_text(lib.config.artwork_preference).to_string()),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn list_all(conn: &Connection) -> StorageResult<Vec<Library>> {
    let mut rows = conn.query(LIST, ()).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = row.get_value(0).map_err(map_turso)?;
        let name = row.get_value(1).map_err(map_turso)?;
        let kind = row.get_value(2).map_err(map_turso)?;
        let paths = row.get_value(3).map_err(map_turso)?;
        let langs = row.get_value(4).map_err(map_turso)?;
        let provs = row.get_value(5).map_err(map_turso)?;
        let excl = row.get_value(6).map_err(map_turso)?;
        let watch = row.get_value(7).map_err(map_turso)?;
        let sched = row.get_value(8).map_err(map_turso)?;
        let artwork = row.get_value(9).map_err(map_turso)?;
        out.push(Library {
            id: LibraryId::from_uuid(blob_to_uuid(col_blob(&id)?)?),
            name: col_text(&name)?.to_string(),
            kind: text_to_library_kind(col_text(&kind)?)?,
            paths: parse_json(col_text(&paths)?)?,
            config: noflex_storage::domain::LibraryConfig {
                preferred_languages: parse_json(col_text(&langs)?)?,
                preferred_providers: parse_json(col_text(&provs)?)?,
                excluded_folders: parse_json(col_text(&excl)?)?,
                watch_enabled: col_int(&watch)? != 0,
                scan_schedule: text_to_schedule(col_text(&sched)?)?,
                artwork_preference: text_to_artwork(col_text(&artwork)?)?,
            },
        });
    }
    Ok(out)
}

pub(crate) async fn delete(conn: &Connection, id: LibraryId) -> StorageResult<()> {
    let params: Vec<Value> = vec![Value::Blob(uuid_to_blob(id.into_uuid()))];
    conn.execute(DELETE, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn scan_version(conn: &Connection, id: LibraryId) -> StorageResult<u32> {
    let params: Vec<Value> = vec![Value::Blob(uuid_to_blob(id.into_uuid()))];
    let mut rows = conn.query(SCAN_VERSION, params).await.map_err(map_turso)?;
    let row = rows
        .next()
        .await
        .map_err(map_turso)?
        .ok_or(StorageError::NotFound)?;
    let version = col_int(&row.get_value(0).map_err(map_turso)?)?;
    u32::try_from(version)
        .map_err(|_| StorageError::Backend(format!("scan_version out of range: {version}").into()))
}

pub(crate) async fn set_scan_version(
    conn: &Connection,
    id: LibraryId,
    version: u32,
) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(id.into_uuid())),
        Value::Integer(i64::from(version)),
    ];
    let changed = conn
        .execute(SET_SCAN_VERSION, params)
        .await
        .map_err(map_turso)?;
    if changed == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}
