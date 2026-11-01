// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end tests for the domain-event dispatcher: events published on
// the core `DomainEventBus` are mapped onto the notifier hook's wire
// shape and fanned out to every loaded plugin exporting `notifier`. The
// fixture guest (`tests/fixtures/notifier/`) records each received event
// as a one-line encoding in instance-global state; the host reads the
// log back through the fixture's `report` background task. The fixture
// panics on any field containing "boom", which the second test uses to
// prove a trapping notifier never kills the dispatch loop.
//
// The fixture component is built on demand (cargo, wasm32-wasip2) when
// the artifact is missing; the tests self-skip if the toolchain can't
// produce it (e.g. a host without the target installed).

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use noflex_core::{
    LibraryId, MediaFileId, UserId, WorkId,
    events::{DomainEvent, DomainEventBus},
};
use noflex_plugin_host::{
    EngineConfig, HookKind, LoadedPlugin, PluginId, PluginRegistry, TaskOutcome, build_engine,
    dispatcher,
};

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

/// The fixture's received-event log, read back through its `report`
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
async fn dispatcher_fans_out_all_event_kinds() {
    let Some(wasm) = crate::fixtures::wasm("notifier") else {
        eprintln!("skipping: could not build the fixture-notifier plugin");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load notifier fixture: {e}"));

    // A plugin without the notifier hook must never be called: the
    // concurrency fixture rides along as the non-notifier bystander.
    if let Some(concurrency) = crate::fixtures::wasm("concurrency") {
        registry
            .load_from(&concurrency)
            .unwrap_or_else(|e| panic!("load concurrency fixture: {e}"));
        assert_eq!(
            registry.exporters_of(HookKind::Notifier),
            vec![PluginId(NOTIFIER_ID.to_string())],
            "only the notifier fixture is fanned out to"
        );
    }

    let bus = DomainEventBus::new(16);
    let registry = Arc::new(registry);
    let dispatcher = dispatcher::spawn(&bus, Arc::clone(&registry));

    let user = UserId::new();
    let work = WorkId::new();
    let media_file = MediaFileId::new();
    let title = "Alien";
    let events = vec![
        DomainEvent::WorkAdded {
            work,
            library: LibraryId::new(),
            title: title.into(),
        },
        DomainEvent::WorkPlayed {
            user,
            work,
            title: title.into(),
        },
        DomainEvent::ProgressAtStop {
            user,
            work,
            title: title.into(),
            position_ms: 42_000,
        },
        DomainEvent::Favorite {
            user,
            work,
            title: title.into(),
            favorite: true,
        },
        DomainEvent::PlaybackStarted {
            user,
            work,
            title: title.into(),
            media_file,
        },
        DomainEvent::PlaybackStopped {
            user,
            work,
            title: title.into(),
            media_file,
        },
        DomainEvent::ScanStarted {
            library: LibraryId::new(),
            library_name: "Movies".into(),
        },
        DomainEvent::ScanCompleted {
            library: LibraryId::new(),
            library_name: "Movies".into(),
            added: 3,
            updated: 2,
            removed: 1,
            added_works: vec![],
            updated_works: vec![],
            removed_works: vec![],
        },
    ];
    // The expected fixture-side encodings, mirroring encode() in the
    // guest. WorkAdded's library id has no WIT counterpart and is dropped.
    let expected: Vec<String> = vec![
        format!("work-added:{work}:{title}"),
        format!("work-played:{user}:{work}:{title}"),
        format!("progress-at-stop:{user}:{work}:{title}:42000"),
        format!("favorite:{user}:{work}:{title}:true"),
        format!("playback-started:{user}:{work}:{title}:{media_file}"),
        format!("playback-stopped:{user}:{work}:{title}:{media_file}"),
        "scan-started:Movies".to_string(),
        "scan-completed:Movies:3:2:1".to_string(),
    ];
    for event in events {
        bus.emit(event);
    }

    let plugin = registry
        .get_plugin(&PluginId(NOTIFIER_ID.to_string()))
        .expect("notifier registered");
    let log = poll_until("all eight events delivered", || async {
        let log = received(&plugin).await;
        expected
            .iter()
            .all(|line| log.contains(line))
            .then_some(log)
    })
    .await;
    assert_eq!(
        log.len(),
        expected.len(),
        "exactly the emitted events arrived, in order: {log:?}"
    );
    dispatcher.abort();
}

#[tokio::test]
async fn dispatcher_survives_trapping_notifier() {
    let Some(wasm) = crate::fixtures::wasm("notifier") else {
        eprintln!("skipping: could not build the fixture-notifier plugin");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load notifier fixture: {e}"));

    let bus = DomainEventBus::new(16);
    let registry = Arc::new(registry);
    let dispatcher = dispatcher::spawn(&bus, Arc::clone(&registry));

    // "boom" panics the guest mid-`notify`: the call fails, the instance
    // recycles, and the dispatcher must keep delivering afterwards.
    bus.emit(DomainEvent::WorkAdded {
        work: WorkId::new(),
        library: LibraryId::new(),
        title: "boom".into(),
    });
    poll_until("trapping instance recycled", || async {
        registry
            .snapshot()
            .into_iter()
            .find(|p| p.id == NOTIFIER_ID)
            .filter(|p| p.instance_restarts >= 1)
    })
    .await;

    let work = WorkId::new();
    bus.emit(DomainEvent::WorkAdded {
        work,
        library: LibraryId::new(),
        title: "Aftermath".into(),
    });
    let plugin = registry
        .get_plugin(&PluginId(NOTIFIER_ID.to_string()))
        .expect("notifier registered");
    let expected = format!("work-added:{work}:Aftermath");
    poll_until("event delivered after the trap", || async {
        received(&plugin).await.contains(&expected).then_some(())
    })
    .await;
    dispatcher.abort();
}

#[tokio::test]
async fn dispatcher_honors_manifest_event_subscriptions() {
    let Some(wasm) = crate::fixtures::wasm("notifier-events") else {
        eprintln!("skipping: could not build the fixture-notifier-events plugin");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load notifier-events fixture: {e}"));

    let bus = DomainEventBus::new(16);
    let registry = Arc::new(registry);
    let dispatcher = dispatcher::spawn(&bus, Arc::clone(&registry));

    // The fixture's manifest declares `events = ["scan-started"]`:
    // scan-started is delivered, work-played is filtered host-side.
    bus.emit(DomainEvent::ScanStarted {
        library: LibraryId::new(),
        library_name: "Movies".into(),
    });
    bus.emit(DomainEvent::WorkPlayed {
        user: UserId::new(),
        work: WorkId::new(),
        title: "Alien".into(),
    });

    let plugin = registry
        .get_plugin(&PluginId("fixture-notifier-events".to_string()))
        .expect("notifier-events registered");
    let expected = "scan-started:Movies".to_string();
    poll_until("subscribed event delivered", || async {
        received(&plugin).await.contains(&expected).then_some(())
    })
    .await;
    // Give the filtered event every chance to (wrongly) arrive before
    // asserting its absence.
    tokio::time::sleep(Duration::from_millis(250)).await;
    let log = received(&plugin).await;
    assert!(log.contains(&expected));
    assert!(
        !log.iter().any(|l| l.starts_with("work-played")),
        "unsubscribed event must be filtered: {log:?}"
    );
    dispatcher.abort();
}
