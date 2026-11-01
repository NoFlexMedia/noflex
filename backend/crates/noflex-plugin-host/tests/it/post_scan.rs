// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end test for the post-scan hook wiring: a `ScanCompleted`
// domain event fans its work-id lists out to every loaded plugin
// exporting `post-scan` (and only those), alongside the notifier
// fan-out. The fixture guest records each call as a one-line encoding
// in instance-global state; the host reads the log back through the
// fixture's `report` background task.
//
// The fixture components are built on demand (cargo, wasm32-wasip2)
// when the artifacts are missing; the test self-skips if the toolchain
// can't produce them.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use noflex_core::{
    LibraryId, WorkId,
    events::{DomainEvent, DomainEventBus},
};
use noflex_plugin_host::{
    EngineConfig, HookKind, LoadedPlugin, PluginId, PluginRegistry, TaskOutcome, build_engine,
    dispatcher,
};

const POST_SCAN_ID: &str = "fixture-post-scan";
const NOTIFIER_ID: &str = "fixture-notifier";
const TIMEOUT: Duration = Duration::from_secs(10);

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

/// The fixture's received-call log, read back through its `report`
/// background task (returned as the `more` cursor).
async fn received(plugin: &Arc<LoadedPlugin>) -> Vec<String> {
    let outcome = plugin
        .call_run_task("report".to_string(), None, None)
        .await
        .expect("report dispatch succeeds")
        .expect("report succeeds");
    let TaskOutcome::More(log) = outcome else {
        panic!("report always answers `more`");
    };
    log.lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn scan_completed_reaches_post_scan_exporters_with_work_ids() {
    let (Some(post_scan_wasm), Some(notifier_wasm)) = (
        crate::fixtures::wasm("post-scan"),
        crate::fixtures::wasm("notifier"),
    ) else {
        eprintln!("skipping: could not build the fixture plugins");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&post_scan_wasm)
        .unwrap_or_else(|e| panic!("load post-scan fixture: {e}"));
    registry
        .load_from(&notifier_wasm)
        .unwrap_or_else(|e| panic!("load notifier fixture: {e}"));

    // The notifier-only fixture must never see a post-scan call: the
    // fan-out selects on the exported hook, not on presence.
    assert_eq!(
        registry.exporters_of(HookKind::PostScan),
        vec![PluginId(POST_SCAN_ID.to_string())],
        "only the post-scan fixture is fanned out to"
    );

    let bus = DomainEventBus::new(16);
    let registry = Arc::new(registry);
    let dispatcher = dispatcher::spawn(&bus, Arc::clone(&registry));

    let library = LibraryId::new();
    let added = vec![WorkId::new(), WorkId::new()];
    let updated = vec![WorkId::new()];
    bus.emit(DomainEvent::ScanCompleted {
        library,
        library_name: "Movies".into(),
        added: 2,
        updated: 1,
        removed: 0,
        added_works: added.clone(),
        updated_works: updated.clone(),
        removed_works: vec![],
    });

    // The guest's encoding: scan-complete:<library>:<added csv>|<updated csv>|<removed csv>
    let expected = format!(
        "scan-complete:{}:{}|{}|",
        library,
        added
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
        updated[0],
    );
    let post_scan = registry
        .get_plugin(&PluginId(POST_SCAN_ID.to_string()))
        .expect("post-scan fixture registered");
    poll_until("post-scan call delivered with ids intact", || async {
        received(&post_scan).await.contains(&expected).then_some(())
    })
    .await;

    // The notifier fan-out still happens on the same event.
    let notifier = registry
        .get_plugin(&PluginId(NOTIFIER_ID.to_string()))
        .expect("notifier fixture registered");
    poll_until("notifier still receives scan-completed", || async {
        received(&notifier)
            .await
            .contains(&"scan-completed:Movies:2:1:0".to_string())
            .then_some(())
    })
    .await;
    dispatcher.abort();
}
