// SPDX-License-Identifier: AGPL-3.0-only
//
// Schema snapshot gate. Builds the schema the way the production
// binary does (every SchemaContext piece wired) and compares the
// printed SDL against a checked-in snapshot, so client-facing surface
// changes can't silently drift the client contract.
//
// Regenerate after an intentional schema change:
//   NOFLEX_UPDATE_SNAPSHOTS=1 cargo test -p noflex-graphql --test it schema_snapshot
// and review the snapshot diff in the same commit.

use std::sync::Arc;

use noflex_auth::JwtConfig;
use noflex_graphql::{ExportEventBroadcaster, MetadataSearch, SchemaContext, build_schema};
use noflex_plugin_host::{EngineConfig, LifecycleBus, PluginRegistry, PriorityTable, build_engine};
use noflex_scanner::{FilesystemScanner, ScanRegistry};
use noflex_storage::store::Storage;
use noflex_storage_turso::TursoStore;

const SNAPSHOT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/snapshots/schema.graphql"
);

async fn full_schema() -> noflex_graphql::NoflexSchema {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn noflex_storage::Storage> = Arc::new(store);

    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine.clone()).expect("registry"));
    let scanner = Arc::new(FilesystemScanner::new(storage.clone()));
    let scan_registry = Arc::new(ScanRegistry::new(scanner));
    let metadata = Arc::new(MetadataSearch::new(registry));

    build_schema(SchemaContext {
        storage: Some(storage),
        scanner: Some(scan_registry),
        metadata: Some(metadata),
        priority_table: Some(Arc::new(tokio::sync::RwLock::new(PriorityTable::default()))),
        jwt: Some(Arc::new(
            JwtConfig::dev(b"snapshot-secret-32-bytes-do-not-care").expect("JWT config"),
        )),
        export_events: Some(Arc::new(ExportEventBroadcaster::default())),
        lifecycle_bus: Some(LifecycleBus::default()),
        ..Default::default()
    })
}

#[tokio::test]
async fn schema_sdl_matches_snapshot() {
    let sdl = format!("{}\n", full_schema().await.sdl());

    if std::env::var_os("NOFLEX_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(SNAPSHOT_PATH, &sdl).expect("write snapshot");
        return;
    }

    let expected = std::fs::read_to_string(SNAPSHOT_PATH).unwrap_or_else(|e| {
        panic!(
            "read snapshot {SNAPSHOT_PATH}: {e}. Run with NOFLEX_UPDATE_SNAPSHOTS=1 to create it"
        )
    });
    if sdl != expected {
        let mismatch = sdl
            .lines()
            .zip(expected.lines())
            .enumerate()
            .find(|(_, (got, want))| got != want)
            .map(|(i, (got, want))| {
                format!(
                    "first diff at line {}:\n  got:  {got}\n  want: {want}",
                    i + 1
                )
            })
            .unwrap_or_else(|| "snapshots differ in length".to_string());
        panic!(
            "schema SDL drifted from {SNAPSHOT_PATH}\n{mismatch}\n\
             If this change is intentional, regenerate with NOFLEX_UPDATE_SNAPSHOTS=1 \
             and commit the snapshot diff."
        );
    }
}

/// Houdini's Infinite paging drops every existing edge whose node has the same
/// `id` as a new one. It reads that field by name, whatever keys the type is
/// configured with, and a missing one matches, so a client paging nodes without
/// it keeps only the newest page.
#[tokio::test]
async fn connection_nodes_have_an_id() {
    let sdl = full_schema().await.sdl();
    let types = sdl
        .split("\ntype ")
        .skip(1)
        .filter_map(|block| {
            let (name, body) = block.split_once(' ')?;
            Some((name, body.split_once("\n}")?.0))
        })
        .collect::<std::collections::HashMap<_, _>>();

    let mut nodes = 0;
    let mut missing = Vec::new();
    for (name, body) in &types {
        if !name.ends_with("Edge") {
            continue;
        }
        let node = body
            .lines()
            .find_map(|line| line.trim().strip_prefix("node: "))
            .unwrap_or_else(|| panic!("{name} has no node"))
            .trim_end_matches('!');
        nodes += 1;
        let has_id = types[node]
            .lines()
            .any(|line| line.trim().starts_with("id: "));
        if !has_id {
            missing.push(node);
        }
    }
    assert!(nodes > 0, "found no connections to check");
    assert!(
        missing.is_empty(),
        "connection nodes without an id: {missing:?}"
    );
}
