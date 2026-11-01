// SPDX-License-Identifier: AGPL-3.0-only
//
// Tests for `PluginRegistry::enqueue_task`, the host-side entry point
// for user-initiated "sync now" flows. The happy path drains through
// the fixture-tasks guest's sweep task (a `dynamic` payload bumps the
// guest's dynamic counter, observable via its `metadata.search` echo);
// the rejection paths are pure host-side validation: unknown plugin,
// missing `background` capability, and a task name the manifest never
// registered (the scheduler drains per registered name, so anything
// else would sit in the queue forever).
//
// The fixture components are built on demand (cargo, wasm32-wasip2)
// when the artifacts are missing; the tests self-skip if the toolchain
// can't produce them.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use noflex_plugin_host::{
    EngineConfig, HostError, PluginId, PluginRegistry, build_engine, plugin_types,
};
use noflex_storage::Storage;
use noflex_storage_turso::TursoStore;

const TASKS_ID: &str = "fixture-tasks";
const CONCURRENCY_ID: &str = "concurrency-fixture";
const TIMEOUT: Duration = Duration::from_secs(10);

async fn poll_until<T, F, Fut>(what: &str, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(
            start.elapsed() < TIMEOUT,
            "timed out after {TIMEOUT:?} waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn setup() -> Option<(PluginRegistry, tempfile::TempDir)> {
    let tasks_wasm = crate::fixtures::wasm("tasks")?;
    let concurrency_wasm = crate::fixtures::wasm("concurrency")?;
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TursoStore::open(dir.path().join("enqueue.db"))
        .await
        .expect("open store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_storage(storage);
    registry
        .load_from(&tasks_wasm)
        .unwrap_or_else(|e| panic!("load fixture-tasks: {e}"));
    registry
        .load_from(&concurrency_wasm)
        .unwrap_or_else(|e| panic!("load concurrency fixture: {e}"));
    Some((registry, dir))
}

#[tokio::test]
async fn enqueue_task_drains_on_the_registered_task() {
    let Some((registry, _dir)) = setup().await else {
        eprintln!("skipping: could not build the fixture plugins");
        return;
    };
    registry
        .enqueue_task(TASKS_ID, "sweep", Some("dynamic".to_string()))
        .await
        .expect("enqueue accepted");

    let plugin = registry
        .get_plugin(&PluginId(TASKS_ID.to_string()))
        .expect("fixture registered");
    poll_until("guest observed the enqueued payload", || async {
        let matches = plugin
            .call_search(plugin_types::SearchQuery {
                title: "probe".to_string(),
                year: None,
                kind: plugin_types::WorkKind::Movie,
                hints: vec![],
            })
            .await
            .expect("dispatch succeeds")
            .expect("fixture search succeeds");
        let dynamic: u32 = matches[0]
            .title
            .split_whitespace()
            .find_map(|part| part.strip_prefix("dynamic="))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("fixture title malformed: {}", matches[0].title));
        (dynamic >= 1).then_some(dynamic)
    })
    .await;
}

#[tokio::test]
async fn enqueue_task_rejects_unregistered_task_name() {
    let Some((registry, _dir)) = setup().await else {
        eprintln!("skipping: could not build the fixture plugins");
        return;
    };
    let err = registry
        .enqueue_task(TASKS_ID, "nightly", None)
        .await
        .expect_err("unregistered task name is rejected");
    match err {
        HostError::UnknownTask {
            id,
            task,
            registered,
        } => {
            assert_eq!(id, TASKS_ID);
            assert_eq!(task, "nightly");
            assert!(
                registered.contains("sweep"),
                "error names the registered tasks: {registered}"
            );
        }
        other => panic!("expected UnknownTask, got {other}"),
    }
}

#[tokio::test]
async fn enqueue_task_rejects_oversize_payload() {
    let Some((registry, _dir)) = setup().await else {
        eprintln!("skipping: could not build the fixture plugins");
        return;
    };
    let err = registry
        .enqueue_task(TASKS_ID, "sweep", Some("x".repeat(1024 * 1024 + 1)))
        .await
        .expect_err("oversize payload is rejected");
    assert!(matches!(err, HostError::TaskPayloadTooLarge { .. }));
}

#[tokio::test]
async fn enqueue_task_rejects_missing_background_capability() {
    let Some((registry, _dir)) = setup().await else {
        eprintln!("skipping: could not build the fixture plugins");
        return;
    };
    let err = registry
        .enqueue_task(CONCURRENCY_ID, "sweep", None)
        .await
        .expect_err("plugin without the background capability is rejected");
    assert!(
        matches!(
            err,
            HostError::MissingCapability {
                capability: "background",
                ..
            }
        ),
        "expected MissingCapability, got {err}"
    );
}

#[tokio::test]
async fn enqueue_task_rejects_unknown_plugin() {
    let Some((registry, _dir)) = setup().await else {
        eprintln!("skipping: could not build the fixture plugins");
        return;
    };
    let err = registry
        .enqueue_task("ghost", "sweep", None)
        .await
        .expect_err("unknown plugin is rejected");
    assert!(
        matches!(err, HostError::Unknown { .. }),
        "expected Unknown, got {err}"
    );
}
