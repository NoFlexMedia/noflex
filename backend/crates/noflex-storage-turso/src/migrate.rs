// SPDX-License-Identifier: AGPL-3.0-only
//
// Apply migration scripts in lexical order. Each script is identified by its
// filename; the filename is recorded in `schema_meta` so that subsequent
// `migrate()` calls are idempotent.

use noflex_storage::error::StorageResult;
use turso::Connection;

use crate::error::map_turso;

/// `(filename, sql)` pairs in lexical order. 0001 is the schema squashed
/// before the first release; add new migrations after it — never edit a
/// migration that has shipped.
pub(crate) const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init.sql", include_str!("../migrations/0001_init.sql")),
    (
        "0002_play_sessions.sql",
        include_str!("../migrations/0002_play_sessions.sql"),
    ),
    (
        "0003_library_scan_version.sql",
        include_str!("../migrations/0003_library_scan_version.sql"),
    ),
    (
        "0004_image_sources.sql",
        include_str!("../migrations/0004_image_sources.sql"),
    ),
    (
        "0005_library_artwork_preference.sql",
        include_str!("../migrations/0005_library_artwork_preference.sql"),
    ),
    (
        "0006_library_content_category.sql",
        include_str!("../migrations/0006_library_content_category.sql"),
    ),
    (
        "0007_smart_transcoding.sql",
        include_str!("../migrations/0007_smart_transcoding.sql"),
    ),
    (
        "0008_multi_server.sql",
        include_str!("../migrations/0008_multi_server.sql"),
    ),
    (
        "0009_ladder_in_pixels.sql",
        include_str!("../migrations/0009_ladder_in_pixels.sql"),
    ),
    (
        "0010_smart_transcoding_settings.sql",
        include_str!("../migrations/0010_smart_transcoding_settings.sql"),
    ),
    (
        "0011_tag_assignment_metadata.sql",
        include_str!("../migrations/0011_tag_assignment_metadata.sql"),
    ),
    (
        "0012_transcoding_settings.sql",
        include_str!("../migrations/0012_transcoding_settings.sql"),
    ),
];

const META_KEY_PREFIX: &str = "migration:";

pub(crate) async fn apply_all(conn: &Connection) -> StorageResult<u32> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_meta (\
            key   TEXT PRIMARY KEY,\
            value TEXT NOT NULL\
        )",
        (),
    )
    .await
    .map_err(map_turso)?;

    let mut applied = 0u32;
    for (name, sql) in MIGRATIONS {
        if is_applied(conn, name).await? {
            continue;
        }
        let tx = turso::transaction::Transaction::new_unchecked(
            conn,
            turso::transaction::TransactionBehavior::Immediate,
        )
        .await
        .map_err(map_turso)?;
        let result = async {
            conn.execute_batch(*sql).await.map_err(map_turso)?;
            record_applied(conn, name).await
        }
        .await;
        match result {
            Ok(()) => tx.commit().await.map_err(map_turso)?,
            Err(error) => {
                let _ = tx.rollback().await;
                return Err(error);
            }
        }
        applied += 1;
    }
    Ok(applied)
}

pub(crate) async fn schema_version(conn: &Connection) -> StorageResult<u32> {
    let mut count = 0u32;
    for (name, _) in MIGRATIONS {
        if is_applied(conn, name).await? {
            count += 1;
        }
    }
    Ok(count)
}

async fn is_applied(conn: &Connection, name: &str) -> StorageResult<bool> {
    let key = format!("{META_KEY_PREFIX}{name}");
    let mut rows = conn
        .query("SELECT 1 FROM schema_meta WHERE key = ?1", vec![key])
        .await
        .map_err(map_turso)?;
    Ok(rows.next().await.map_err(map_turso)?.is_some())
}

async fn record_applied(conn: &Connection, name: &str) -> StorageResult<()> {
    let key = format!("{META_KEY_PREFIX}{name}");
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES (?1, ?2)",
        vec![
            turso::Value::Text(key),
            turso::Value::Text("ok".to_string()),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use noflex_storage::{
        Storage,
        domain::{ImageSource, ImageTarget},
    };

    use crate::TursoStore;

    /// 0004 keys images by source and backfills it from the path: rows
    /// the episode-thumbnail sweep wrote become `generated`, the rest
    /// `local`.
    #[tokio::test]
    async fn image_sources_backfill_from_path() {
        let store = TursoStore::open_temp().await.expect("store");
        let work = noflex_core::WorkId::new();
        {
            let (_schema, conn) = store.writer().await.expect("writer");
            conn.execute(
                "CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
                (),
            )
            .await
            .expect("meta");
            for (name, sql) in super::MIGRATIONS
                .iter()
                .take_while(|(name, _)| *name != "0004_image_sources.sql")
            {
                conn.execute_batch(*sql).await.expect("old migration");
                super::record_applied(&conn, name).await.expect("record");
            }
            for (kind, path) in [
                ("primary", "/media/Movie/poster.jpg"),
                ("thumb", "/cache/image/episode-thumbs/x.jpg"),
            ] {
                conn.execute(
                    "INSERT INTO images (target_kind, target_id, kind, path) \
                     VALUES ('work', ?1, ?2, ?3)",
                    vec![
                        turso::Value::Blob(crate::convert::uuid_to_blob(work.into_uuid())),
                        turso::Value::Text(kind.into()),
                        turso::Value::Text(path.into()),
                    ],
                )
                .await
                .expect("old image row");
            }
        }
        store.migrate().await.expect("migrate");

        let sources: Vec<_> = store
            .list_images(ImageTarget::Work(work), ImageSource::ALL)
            .await
            .expect("list")
            .into_iter()
            .map(|i| (i.kind.as_str(), i.source))
            .collect();
        assert_eq!(
            sources,
            vec![
                ("primary", ImageSource::Local),
                ("thumb", ImageSource::Generated)
            ]
        );
    }
}
