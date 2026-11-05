// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end tests for the intro-detection worker
// (`intro::spawn_intro_detector_worker`): the startup sweep pages
// editions that have never been scanned, drives the detector plugin's
// `detect` hook (whose inputs arrive through the real
// `host-library.lookup-edition-by-id` import), and persists the outcome
// — markers plus a found scan row on a detection, a not-found row on
// `None`. A `ScanCompleted` event triggers another sweep for editions
// added after startup.
//
// The fixture guest (`tests/fixtures/intro-detector/`) returns a
// deterministic detection derived from the edition's chapter positions
// when chapters exist, `None` otherwise. It is built on demand (cargo,
// wasm32-wasip2); the tests self-skip if the toolchain can't produce it.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use noflex_core::{
    EditionId, LibraryId, WorkId,
    events::{DomainEvent, DomainEventBus},
};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine, spawn_intro_detector_worker};
use noflex_storage::{
    PageCursor, Storage,
    domain::{Chapter, Edition, Library, LibraryKind, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

const PLUGIN_ID: &str = "fixture-intro-detector";
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

struct Harness {
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    bus: DomainEventBus,
    // Holds the tempfile alive for the store's lifetime.
    _dir: tempfile::TempDir,
}

async fn setup() -> Option<Harness> {
    let wasm = crate::fixtures::wasm("intro-detector")?;
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TursoStore::open(dir.path().join("intro.db"))
        .await
        .expect("open store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_storage(Arc::clone(&storage));
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load intro-detector fixture: {e}"));
    Some(Harness {
        registry: Arc::new(registry),
        storage,
        bus: DomainEventBus::default(),
        _dir: dir,
    })
}

fn library() -> Library {
    Library {
        id: LibraryId::new(),
        name: "Shows".to_string(),
        kind: LibraryKind::Shows,
        paths: vec!["/srv/shows".to_string()],
        ..Default::default()
    }
}

fn work(library_id: LibraryId, title: &str) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind: noflex_core::WorkKind::Episode,
        parent_id: None,
        library_id,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
        year: None,
        release_date: None,
        overview: None,
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: noflex_core::MetadataStatus::Unconfirmed,
        locked_fields: vec![],
        added_at: now,
        updated_at: now,
    }
}

fn edition(work_id: WorkId, runtime_ms: Option<i64>, chapters: Vec<Chapter>) -> Edition {
    Edition {
        id: EditionId::new(),
        work_id,
        name: "Default".to_string(),
        runtime_ms,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters,
    }
}

/// Editions still awaiting a scan row for the fixture plugin.
async fn missing(storage: &Arc<dyn Storage>) -> Vec<EditionId> {
    storage
        .editions_missing_intro_scan(PLUGIN_ID, 100, None::<PageCursor>)
        .await
        .expect("list missing")
        .items
        .iter()
        .map(|e| e.id)
        .collect()
}

#[tokio::test]
async fn startup_sweep_persists_markers_and_records_scans() {
    let Some(h) = setup().await else {
        eprintln!("skipping: could not build the fixture-intro-detector plugin");
        return;
    };
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");
    let episode = work(lib.id, "Pilot");
    h.storage.upsert_work(&episode).await.expect("work");
    let with_chapters = edition(
        episode.id,
        Some(1_560_000),
        vec![
            Chapter {
                position_ms: 0,
                title: Some("Opening".to_string()),
            },
            Chapter {
                position_ms: 90_000,
                title: Some("Episode".to_string()),
            },
            Chapter {
                position_ms: 1_500_000,
                title: Some("Credits".to_string()),
            },
        ],
    );
    let mut without_chapters = edition(episode.id, Some(1_560_000), vec![]);
    without_chapters.name = "No chapters".to_string();
    h.storage
        .upsert_edition(&with_chapters)
        .await
        .expect("edition");
    h.storage
        .upsert_edition(&without_chapters)
        .await
        .expect("edition");

    let _worker = spawn_intro_detector_worker(h.registry.clone(), h.storage.clone(), &h.bus);

    poll_until(
        "startup sweep recorded a scan for both editions",
        || async { missing(&h.storage).await.is_empty().then_some(()) },
    )
    .await;

    // Found path: the fixture's deterministic detection — first/last
    // chapter positions, runtime minus a minute — persisted verbatim.
    let detected = h
        .storage
        .get_edition(with_chapters.id)
        .await
        .expect("get edition")
        .expect("edition exists");
    assert_eq!(detected.intro_start_ms, Some(0));
    assert_eq!(detected.intro_end_ms, Some(1_500_000));
    assert_eq!(detected.credits_start_ms, Some(1_500_000));

    // None path: no markers written, but the scan row exists (the
    // edition left the missing list above), so the sweep never retries
    // it either.
    let skipped = h
        .storage
        .get_edition(without_chapters.id)
        .await
        .expect("get edition")
        .expect("edition exists");
    assert_eq!(skipped.intro_start_ms, None);
    assert_eq!(skipped.intro_end_ms, None);
    assert_eq!(skipped.credits_start_ms, None);
}

#[tokio::test]
async fn scan_completed_triggers_a_sweep_for_new_editions() {
    let Some(h) = setup().await else {
        eprintln!("skipping: could not build the fixture-intro-detector plugin");
        return;
    };
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");

    let _worker = spawn_intro_detector_worker(h.registry.clone(), h.storage.clone(), &h.bus);

    let episode = work(lib.id, "Finale");
    h.storage.upsert_work(&episode).await.expect("work");
    let ed = edition(
        episode.id,
        Some(1_560_000),
        vec![
            Chapter {
                position_ms: 5_000,
                title: Some("Cold Open".to_string()),
            },
            Chapter {
                position_ms: 1_400_000,
                title: Some("Credits".to_string()),
            },
        ],
    );
    h.storage.upsert_edition(&ed).await.expect("edition");
    h.bus.emit(DomainEvent::ScanCompleted {
        library: lib.id,
        library_name: lib.name.clone(),
        added: 1,
        updated: 0,
        removed: 0,
        added_works: vec![episode.id],
        updated_works: vec![],
        removed_works: vec![],
    });

    poll_until("ScanCompleted sweep detected the new edition", || async {
        missing(&h.storage).await.is_empty().then_some(())
    })
    .await;

    let detected = h
        .storage
        .get_edition(ed.id)
        .await
        .expect("get edition")
        .expect("edition exists");
    assert_eq!(detected.intro_start_ms, Some(5_000));
    assert_eq!(detected.intro_end_ms, Some(1_400_000));
    assert_eq!(detected.credits_start_ms, Some(1_500_000));
}
