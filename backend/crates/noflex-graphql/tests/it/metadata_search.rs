// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use noflex_graphql::{SchemaContext, build_schema};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
use noflex_scanner::{FilesystemScanner, ScanRegistry};
use noflex_storage::store::Storage;
use noflex_storage_turso::TursoStore;

fn as_admin(query: &str) -> async_graphql::Request {
    async_graphql::Request::new(query).data(noflex_storage::RequestUser {
        user_id: noflex_core::UserId::new(),
        is_admin: true,
    })
}

fn empty_priority_table() -> Arc<tokio::sync::RwLock<noflex_plugin_host::PriorityTable>> {
    Arc::new(tokio::sync::RwLock::new(
        noflex_plugin_host::PriorityTable::default(),
    ))
}

#[tokio::test]
async fn search_metadata_returns_empty_list_when_no_plugins_loaded() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine.clone()).expect("registry"));
    let scanner = Arc::new(FilesystemScanner::new(Arc::new(
        TursoStore::open(":memory:").await.expect("scan store"),
    )));
    let scan_registry = Arc::new(ScanRegistry::new(scanner));
    let metadata = Arc::new(noflex_graphql::MetadataSearch::new(registry));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        scanner: Some(scan_registry),
        metadata: Some(metadata),
        priority_table: Some(empty_priority_table()),
        ..Default::default()
    });

    let query = r#"{ searchMetadata(query: "The Matrix", kind: MOVIE, year: 1999) { title year confidence provider } }"#;
    let resp = schema.execute(as_admin(query)).await;
    assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["searchMetadata"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn search_metadata_rejects_empty_query() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine.clone()).expect("registry"));
    let scanner = Arc::new(FilesystemScanner::new(Arc::new(
        TursoStore::open(":memory:").await.expect("scan store"),
    )));
    let scan_registry = Arc::new(ScanRegistry::new(scanner));
    let metadata = Arc::new(noflex_graphql::MetadataSearch::new(registry));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        scanner: Some(scan_registry),
        metadata: Some(metadata),
        priority_table: Some(empty_priority_table()),
        ..Default::default()
    });

    let query = r#"{ searchMetadata(query: "   ", kind: MOVIE) { title } }"#;
    let resp = schema.execute(as_admin(query)).await;
    assert!(
        !resp.errors.is_empty(),
        "empty query must be rejected: {:?}",
        resp.data.into_json()
    );
    let msg = resp.errors[0].message.clone();
    assert!(msg.contains("query is required"), "msg: {msg}");
}
