// SPDX-License-Identifier: AGPL-3.0-only
//
// Integration tests for the host-driven embedding worker
// (`embeddings::spawn_embedding_worker`): the startup sweep backfills
// pre-existing works, `WorkAdded` events feed the debounced batch path,
// and a registry without an embedder leaves both the worker and the
// storage untouched.
//
// The fixture-embedder guest (deterministic, text-sensitive dim-8
// vectors, namespace "fixture-embed") is built on demand; the tests
// self-skip if the toolchain can't produce it.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use noflex_core::{DomainEvent, DomainEventBus, LibraryId, MetadataStatus, WorkId, WorkKind};
use noflex_plugin_host::{
    EmbedInput, EngineConfig, PluginRegistry, RemoteEmbedder, build_engine, embed_inputs,
    embeddings::EmbeddingWorkerConfig, spawn_embedding_worker,
};
use noflex_storage::{
    PageCursor, Storage,
    domain::{Library, LibraryKind, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

const NAMESPACE: &str = "fixture-embed";
const TIMEOUT: Duration = Duration::from_secs(15);

/// Ignores how busy the test machine is.
fn unthrottled() -> EmbeddingWorkerConfig {
    EmbeddingWorkerConfig {
        busy_percent: f32::INFINITY,
        ..Default::default()
    }
}

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

async fn setup(load_embedder: bool) -> Option<Harness> {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TursoStore::open(dir.path().join("embeddings.db"))
        .await
        .expect("open store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_storage(storage.clone());
    if load_embedder {
        let wasm = crate::fixtures::wasm("embedder")?;
        registry
            .load_from(&wasm)
            .unwrap_or_else(|e| panic!("load fixture-embedder: {e}"));
    }
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
        name: "Movies".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/movies".to_string()],
        ..Default::default()
    }
}

fn work(library_id: LibraryId, title: &str, year: Option<i32>, overview: Option<&str>) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
        year,
        release_date: None,
        overview: overview.map(str::to_string),
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    }
}

async fn missing_count(storage: &Arc<dyn Storage>) -> usize {
    storage
        .works_missing_embedding(NAMESPACE, 100, None::<PageCursor>)
        .await
        .expect("list missing")
        .items
        .len()
}

#[tokio::test]
async fn startup_sweep_backfills_existing_works() {
    let Some(h) = setup(true).await else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");
    let alien = work(
        lib.id,
        "Alien",
        Some(1979),
        Some("In space no one can hear you scream."),
    );
    let works = [
        alien.clone(),
        work(lib.id, "Blade Runner", Some(1982), None),
        work(lib.id, "The Thing", None, None),
    ];
    for w in &works {
        h.storage.upsert_work(w).await.expect("work");
    }

    let _worker =
        spawn_embedding_worker(h.registry.clone(), h.storage.clone(), &h.bus, unthrottled());

    poll_until("startup sweep embedded all three works", || async {
        (missing_count(&h.storage).await == 0).then_some(())
    })
    .await;

    // The stored vector is findable: embed the work's shaped text
    // through the fixture directly and ask the vector index for it.
    let text = format!(
        "{} ({}). {}",
        alien.title,
        alien.year.expect("year"),
        alien.overview.as_deref().expect("overview")
    );
    let query = embed_inputs(&h.registry, vec![EmbedInput::Query(text)])
        .await
        .expect("dispatch succeeds")
        .expect("embedder is loaded")
        .vectors
        .pop()
        .expect("one vector");
    let hits = h
        .storage
        .search_vector(NAMESPACE, &query, &[], None, 10)
        .await
        .expect("vector search");
    assert!(
        hits.iter().any(|hit| hit.work == alien.id),
        "vector search finds the work its text was embedded from: {hits:?}"
    );
}

#[tokio::test]
async fn work_added_event_feeds_the_batch_path() {
    let Some(h) = setup(true).await else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");

    let _worker =
        spawn_embedding_worker(h.registry.clone(), h.storage.clone(), &h.bus, unthrottled());

    let w = work(lib.id, "Arrival", Some(2016), None);
    h.storage.upsert_work(&w).await.expect("work");
    h.bus.emit(DomainEvent::WorkAdded {
        work: w.id,
        library: lib.id,
        title: w.title.clone(),
    });

    poll_until("WorkAdded work embedded via the event path", || async {
        (missing_count(&h.storage).await == 0).then_some(())
    })
    .await;
}

#[tokio::test]
async fn scan_completed_triggers_another_sweep() {
    let Some(h) = setup(true).await else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");

    let _worker =
        spawn_embedding_worker(h.registry.clone(), h.storage.clone(), &h.bus, unthrottled());

    // No event per work — the sweep must find it on its own.
    let w = work(lib.id, "Solaris", Some(1972), None);
    h.storage.upsert_work(&w).await.expect("work");
    h.bus.emit(DomainEvent::ScanCompleted {
        library: lib.id,
        library_name: lib.name.clone(),
        added: 1,
        updated: 0,
        removed: 0,
        added_works: vec![w.id],
        updated_works: vec![],
        removed_works: vec![],
    });

    poll_until("ScanCompleted sweep embedded the work", || async {
        (missing_count(&h.storage).await == 0).then_some(())
    })
    .await;
}

#[tokio::test]
async fn worker_without_embedder_idles_without_embeds() {
    let Some(h) = setup(false).await else {
        eprintln!("skipping: could not set up storage");
        return;
    };
    assert!(h.registry.embedder_claim().is_none());
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");
    let w = work(lib.id, "Stalker", Some(1979), None);
    h.storage.upsert_work(&w).await.expect("work");

    let worker =
        spawn_embedding_worker(h.registry.clone(), h.storage.clone(), &h.bus, unthrottled());
    h.bus.emit(DomainEvent::WorkAdded {
        work: w.id,
        library: lib.id,
        title: w.title.clone(),
    });

    // Long enough for the debounce window and the startup sweep to
    // have fired several times over.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!worker.is_finished(), "worker must not die");
    assert_eq!(
        missing_count(&h.storage).await,
        1,
        "no embedder, no embeddings"
    );
}

/// A stand-in embeddings API whose vectors differ by model, counting what it's
/// asked to embed.
async fn embeddings_api() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use axum::{Json, extract::State, routing::post};

    let embedded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = axum::Router::new()
        .route(
            "/v1/embeddings",
            post(
                |State(embedded): State<Arc<std::sync::atomic::AtomicUsize>>,
                 Json(body): Json<serde_json::Value>| async move {
                    let inputs = body["input"].as_array().expect("input").len();
                    embedded.fetch_add(inputs, std::sync::atomic::Ordering::SeqCst);
                    let value = if body["model"] == "first" { 1.0 } else { -1.0 };
                    let data = (0..inputs)
                        .map(|index| {
                            serde_json::json!({ "index": index, "embedding": [value, 0.0, 0.0] })
                        })
                        .collect::<Vec<_>>();
                    Json(serde_json::json!({ "data": data }))
                },
            ),
        )
        .with_state(embedded.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{address}/v1"), embedded)
}

fn endpoint(url: &str, model: &str) -> Arc<RemoteEmbedder> {
    Arc::new(
        RemoteEmbedder::new(noflex_storage::domain::EmbeddingEndpoint {
            url: url.to_string(),
            api_key: None,
            model: model.to_string(),
            dimensions: 3,
            request_dimensions: None,
            query_prefix: String::new(),
            document_prefix: String::new(),
        })
        .expect("client"),
    )
}

#[tokio::test]
async fn changing_the_model_reindexes_everything_and_prunes_the_old_vectors() {
    let h = setup(false).await.expect("needs no plugin");
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");
    for title in ["Alien", "Aliens", "Heat"] {
        h.storage
            .upsert_work(&work(lib.id, title, None, None))
            .await
            .expect("work");
    }
    let (url, embedded) = embeddings_api().await;
    let first = endpoint(&url, "first");
    let first_namespace = first.claim().namespace.clone();
    h.registry.set_remote_embedder(Some(first));
    let _worker =
        spawn_embedding_worker(h.registry.clone(), h.storage.clone(), &h.bus, unthrottled());

    let count = |namespace: String| {
        let storage = h.storage.clone();
        async move {
            storage
                .search_vector(&namespace, &[1.0, 0.0, 0.0], &[], None, 10)
                .await
                .expect("search")
                .len()
        }
    };
    poll_until("the first model indexed the library", || async {
        (count(first_namespace.clone()).await == 3).then_some(())
    })
    .await;
    let active = h.storage.active_embedding_model().await.expect("active");
    assert_eq!(active.expect("recorded").namespace, first_namespace);

    let second = endpoint(&url, "second");
    let second_namespace = second.claim().namespace.clone();
    assert_ne!(first_namespace, second_namespace);
    h.registry.set_remote_embedder(Some(second));

    poll_until("the second model indexed the library", || async {
        (count(second_namespace.clone()).await == 3).then_some(())
    })
    .await;
    poll_until("the first model's vectors were pruned", || async {
        (count(first_namespace.clone()).await == 0).then_some(())
    })
    .await;
    let active = h.storage.active_embedding_model().await.expect("active");
    assert_eq!(active.expect("recorded").model_id, "second");
    assert_eq!(
        embedded.load(std::sync::atomic::Ordering::SeqCst),
        6,
        "each work embedded once per model"
    );
}
