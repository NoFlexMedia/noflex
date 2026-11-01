// SPDX-License-Identifier: AGPL-3.0-only
//
// Integration tests for host-scheduled background tasks (WIT 0.3.0). The
// fixture guest (`tests/fixtures/tasks/`) runs a "sweep" task that never
// finishes — every invocation returns `more(cursor+1)` — so the host-side
// cursor advances promptly without waiting on the manifest interval. The
// suite proves: scheduled firing + health reporting, cursor survival
// across instance recycling, cursor restore across unload/reload, clean
// drain on unload, and FIFO draining of `host-tasks.enqueue` payloads.
//
// Storage is a real Turso store in a tempdir so the persistence paths
// (cursor checkpoints, the dynamic queue) are exercised for real.
// Assertions poll with generous timeouts instead of fixed sleeps: the
// scheduler's cadence is host-timing-dependent.
//
// The fixture component is built on demand (cargo, wasm32-wasip2) when
// the artifact is missing; the tests self-skip if the toolchain can't
// produce it (e.g. a host without the target installed).

use std::{
    future::Future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use noflex_plugin_host::{EngineConfig, PluginId, PluginRegistry, build_engine, plugin_types};
use noflex_storage::Storage;
use noflex_storage_turso::TursoStore;

const PLUGIN_ID: &str = "fixture-tasks";
const TASK: &str = "sweep";
/// Scheduler timing (50ms interval, 500ms queue poll, 100ms recycle
/// backoff) leaves generous headroom here even on a loaded CI box.
const TIMEOUT: Duration = Duration::from_secs(10);

struct Fixture {
    registry: PluginRegistry,
    storage: Arc<dyn Storage>,
    /// Keeps the Turso file alive for the test's duration.
    _dir: tempfile::TempDir,
}

async fn setup() -> Option<(Fixture, PathBuf)> {
    let wasm = crate::fixtures::wasm("tasks")?;
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TursoStore::open(dir.path().join("tasks.db"))
        .await
        .expect("open store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_storage(Arc::clone(&storage));
    Some((
        Fixture {
            registry,
            storage,
            _dir: dir,
        },
        wasm,
    ))
}

/// Re-run `f` until it yields, failing the test after [`TIMEOUT`].
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

fn query(title: &str) -> plugin_types::SearchQuery {
    plugin_types::SearchQuery {
        title: title.to_string(),
        year: None,
        kind: plugin_types::WorkKind::Movie,
        hints: vec![],
    }
}

/// Current persisted cursor for the fixture's sweep task, parsed as the
/// monotonically increasing counter the guest writes.
async fn stored_cursor(storage: &Arc<dyn Storage>) -> Option<u32> {
    storage
        .plugin_task_state(PLUGIN_ID, TASK)
        .await
        .expect("read task state")
        .and_then(|s| s.cursor)
        .map(|c| c.parse::<u32>().expect("cursor is numeric"))
}

#[tokio::test]
async fn scheduled_task_fires_and_reports_health() {
    let Some((fx, wasm)) = setup().await else {
        eprintln!("skipping: could not build the fixture-tasks plugin");
        return;
    };
    fx.registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    let task = poll_until("sweep total_runs >= 3", || async {
        fx.registry
            .snapshot()
            .into_iter()
            .find(|p| p.id == PLUGIN_ID)
            .and_then(|p| p.tasks.into_iter().find(|t| t.name == TASK))
            .filter(|t| t.total_runs >= 3)
    })
    .await;
    assert!(task.last_run_at.is_some(), "a run finished");
    assert_eq!(
        task.consecutive_failures, 0,
        "the fixture never errors on scheduled runs"
    );

    let c1 = poll_until("persisted cursor", || async {
        stored_cursor(&fx.storage).await
    })
    .await;
    let c2 = poll_until("cursor advancing", || async {
        stored_cursor(&fx.storage).await.filter(|c| *c > c1)
    })
    .await;
    assert!(
        c1 >= 1 && c2 > c1,
        "cursor is numeric and strictly increasing (c1={c1}, c2={c2})"
    );
}

#[tokio::test]
async fn cursor_survives_instance_recycle() {
    let Some((fx, wasm)) = setup().await else {
        eprintln!("skipping: could not build the fixture-tasks plugin");
        return;
    };
    fx.registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let id = PluginId(PLUGIN_ID.to_string());

    poll_until("cursor >= 3 before trap", || async {
        stored_cursor(&fx.storage).await.filter(|c| *c >= 3)
    })
    .await;

    let plugin = fx.registry.get_plugin(&id).expect("fixture registered");
    let outcome = plugin
        .call_run_task(TASK.to_string(), None, Some("boom".to_string()))
        .await;
    assert!(
        outcome.is_err(),
        "boom payload traps the guest: dispatch must fail, got {outcome:?}"
    );

    poll_until("instance recycle", || async {
        fx.registry
            .snapshot()
            .into_iter()
            .find(|p| p.id == PLUGIN_ID)
            .filter(|p| p.instance_restarts >= 1)
    })
    .await;

    // The recycle zeroed the guest's counters; the host-side cursor must
    // resume where it was, not drop back to the guest's fresh 1.
    let c1 = poll_until("cursor after recycle", || async {
        stored_cursor(&fx.storage).await
    })
    .await;
    let c2 = poll_until("cursor advancing after recycle", || async {
        stored_cursor(&fx.storage).await.filter(|c| *c > c1)
    })
    .await;
    assert!(
        c1 > 1 && c2 > c1,
        "cursor survived recycling and kept advancing (c1={c1}, c2={c2})"
    );
}

#[tokio::test]
async fn cursor_restored_after_unload_reload() {
    let Some((fx, wasm)) = setup().await else {
        eprintln!("skipping: could not build the fixture-tasks plugin");
        return;
    };
    fx.registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let id = PluginId(PLUGIN_ID.to_string());

    let before = poll_until("cursor >= 5", || async {
        stored_cursor(&fx.storage).await.filter(|c| *c >= 5)
    })
    .await;

    assert!(fx.registry.unload(&id), "plugin was loaded");
    fx.registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("reload fixture plugin: {e}"));

    // The fresh scheduler restores the persisted cursor at loop start, so
    // the counter resumes above the pre-unload sample instead of
    // restarting from 1.
    let after = poll_until("cursor beyond pre-unload sample", || async {
        stored_cursor(&fx.storage).await.filter(|c| *c > before)
    })
    .await;
    assert!(
        after > before,
        "cursor restored across unload/reload (before={before}, after={after})"
    );
}

#[tokio::test]
async fn clean_drain_on_unload() {
    let Some((fx, wasm)) = setup().await else {
        eprintln!("skipping: could not build the fixture-tasks plugin");
        return;
    };
    fx.registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let id = PluginId(PLUGIN_ID.to_string());

    poll_until("sweep ran at least once", || async {
        fx.registry
            .snapshot()
            .into_iter()
            .find(|p| p.id == PLUGIN_ID)
            .and_then(|p| p.tasks.into_iter().find(|t| t.name == TASK))
            .filter(|t| t.total_runs >= 1)
    })
    .await;

    let plugin = fx.registry.get_plugin(&id).expect("fixture registered");
    let weak = Arc::downgrade(&plugin);
    assert!(fx.registry.unload(&id), "plugin was loaded");
    assert!(fx.registry.snapshot().is_empty(), "registry is empty");
    assert!(fx.registry.get_plugin(&id).is_none(), "plugin is gone");

    // Dropping the last handle closes the instance channel; the instance
    // task drains in-flight calls and the LoadedPlugin is dropped.
    drop(plugin);
    poll_until("instance fully torn down", || {
        let gone = weak.upgrade().is_none();
        async move { gone.then_some(()) }
    })
    .await;
}

#[tokio::test]
async fn guest_enqueue_rejects_unknown_tasks_and_oversize_payloads() {
    let Some((fx, wasm)) = setup().await else {
        eprintln!("skipping: could not build the fixture-tasks plugin");
        return;
    };
    fx.registry.load_from(&wasm).expect("load fixture plugin");
    let plugin = fx
        .registry
        .get_plugin(&PluginId(PLUGIN_ID.to_string()))
        .expect("fixture registered");

    for task in ["self-enqueue-unknown", "self-enqueue-oversize"] {
        let result = plugin
            .call_run_task(task.to_string(), None, None)
            .await
            .expect("dispatch succeeds");
        assert!(
            matches!(result, Err(plugin_types::PluginError::BadInput(_))),
            "{task}: {result:?}"
        );
    }
}

#[tokio::test]
async fn dynamic_enqueue_drains_fifo() {
    let Some((fx, wasm)) = setup().await else {
        eprintln!("skipping: could not build the fixture-tasks plugin");
        return;
    };
    fx.registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let id = PluginId(PLUGIN_ID.to_string());
    let plugin = fx.registry.get_plugin(&id).expect("fixture registered");

    // The guest calls host-tasks.enqueue("sweep", "dynamic") from inside
    // this run-task call; the scheduler then dequeues the payload and
    // fires sweep with it, bumping the guest's dynamic counter.
    plugin
        .call_run_task("self-enqueue".to_string(), None, None)
        .await
        .expect("dispatch succeeds")
        .expect("self-enqueue succeeds");

    poll_until("guest observed the dynamic payload", || async {
        let matches = plugin
            .call_search(query("probe"))
            .await
            .expect("dispatch succeeds")
            .expect("fixture search succeeds");
        let title = &matches[0].title;
        let dynamic: u32 = title
            .split_whitespace()
            .find_map(|part| part.strip_prefix("dynamic="))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("fixture title malformed: {title}"));
        (dynamic >= 1).then_some(dynamic)
    })
    .await;
}
