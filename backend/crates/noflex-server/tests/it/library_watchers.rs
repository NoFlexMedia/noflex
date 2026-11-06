// SPDX-License-Identifier: AGPL-3.0-only
//
// Library watchers follow library mutations: created libraries start
// watching, disabling or deleting stops it — no restart needed.

use std::sync::Arc;

use noflex_auth::{JwtConfig, Principal};
use noflex_core::{DomainEventBus, LibraryId, UserId};
use noflex_graphql::{SchemaContext, admin_status::WatcherState, build_schema};
use noflex_plugin_host::LifecycleBus;
use noflex_scanner::FilesystemScanner;
use noflex_server::{
    library_watchers::LibraryWatchers,
    metrics::Metrics,
    system_status::{LogBuffer, Recorder},
};
use noflex_storage::{RequestUser, Storage};
use noflex_storage_turso::TursoStore;

#[tokio::test]
async fn watcher_follows_library_mutations() {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store = Arc::new(store);
    let storage: Arc<dyn Storage> = store.clone();
    let recorder = Recorder::spawn(
        &LifecycleBus::default(),
        &DomainEventBus::default(),
        storage.clone(),
        Arc::new(LogBuffer::default()),
        Metrics::new(),
    );
    let watchers = Arc::new(LibraryWatchers::new(
        storage.clone(),
        Arc::new(FilesystemScanner::new(storage.clone())),
        recorder.clone(),
    ));
    let schema = build_schema(SchemaContext {
        storage: Some(store),
        jwt: Some(Arc::new(
            JwtConfig::dev(b"library-watchers-test-secret-32b!").expect("jwt"),
        )),
        library_watch: Some(watchers),
        ..Default::default()
    });
    let admin = UserId::new();
    let run = |query: String| {
        let schema = schema.clone();
        async move {
            let response = schema
                .execute(
                    async_graphql::Request::new(query)
                        .data(Principal {
                            user_id: admin,
                            is_admin: true,
                            session_id: None,
                        })
                        .data(RequestUser {
                            user_id: admin,
                            is_admin: true,
                        }),
                )
                .await;
            assert!(response.errors.is_empty(), "{:?}", response.errors);
            response.data.into_json().expect("json")
        }
    };
    let state = |id: LibraryId| recorder.library(id).2.state;

    let root = tempfile::tempdir().expect("root");
    let created = run(format!(
        r#"mutation {{ createLibrary(name: "Films", kind: MOVIES, paths: ["{}"]) {{ id }} }}"#,
        root.path().display()
    ))
    .await;
    let id: LibraryId = created["createLibrary"]["id"]
        .as_str()
        .expect("id")
        .parse::<uuid::Uuid>()
        .map(LibraryId::from_uuid)
        .expect("uuid");
    assert_eq!(state(id), WatcherState::Active, "new library is watched");

    run(format!(
        r#"mutation {{ updateLibraryConfig(id: "{id}", watchEnabled: false) {{ id }} }}"#
    ))
    .await;
    assert_eq!(state(id), WatcherState::NotStarted, "disabled stops it");

    run(format!(
        r#"mutation {{ updateLibraryConfig(id: "{id}", watchEnabled: true) {{ id }} }}"#
    ))
    .await;
    assert_eq!(state(id), WatcherState::Active, "re-enabled restarts it");

    run(format!(
        r#"mutation {{ deleteLibrary(id: "{id}") {{ id }} }}"#
    ))
    .await;
    assert_eq!(state(id), WatcherState::NotStarted, "deleted stops it");
}
