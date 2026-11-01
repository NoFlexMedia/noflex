// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end tests for the scrobble dispatcher: playback-derived
// `DomainEvent`s are enriched from storage (kind, year, external ids,
// runtime) and fanned out to every loaded scrobbler plugin whose acting
// user holds linked secrets. The fixture guest (`tests/fixtures/scrobbler/`)
// records each received event as a one-line encoding in instance-global
// state (read back through its `report` background task, the same
// observation channel fixture-notifier uses) and rotates the acting
// user's `access_token` through `read-for-user`/`write-for-user`, so
// the tests assert both the enriched payload and the persisted secret
// write over the real wasm boundary.
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
    LibraryId, UserId, WorkId,
    events::{DomainEvent, DomainEventBus},
    model::{ExternalId, ExternalIdKind},
};
use noflex_plugin_host::{
    EngineConfig, LoadedPlugin, PluginId, PluginRegistry, TaskOutcome, build_engine,
    spawn_scrobble_dispatcher,
};
use noflex_storage::{
    Storage,
    domain::{Edition, Library, LibraryKind, User, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

const PLUGIN_ID: &str = "fixture-scrobbler";
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
    plugin: Arc<LoadedPlugin>,
    // Holds the tempfile alive for the store's lifetime.
    _dir: tempfile::TempDir,
}

async fn setup() -> Option<Harness> {
    let wasm = crate::fixtures::wasm("scrobbler")?;
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TursoStore::open(dir.path().join("scrobble.db"))
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
        .unwrap_or_else(|e| panic!("load scrobbler fixture: {e}"));
    let plugin = registry
        .get_plugin(&PluginId(PLUGIN_ID.to_string()))
        .expect("fixture registered");
    Some(Harness {
        registry: Arc::new(registry),
        storage,
        bus: DomainEventBus::default(),
        plugin,
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

fn work(library_id: LibraryId, title: &str, year: Option<i32>, ids: Vec<ExternalId>) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
        year,
        release_date: None,
        overview: None,
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: ids,
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: noflex_core::MetadataStatus::Unconfirmed,
        locked_fields: vec![],
        added_at: now,
        updated_at: now,
    }
}

fn edition(work_id: WorkId, runtime_ms: Option<i64>) -> Edition {
    Edition {
        id: noflex_core::EditionId::new(),
        work_id,
        name: "Default".to_string(),
        runtime_ms,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
        codec_video: None,
        codec_audio: None,
        width: None,
        height: None,
        bit_depth: None,
        hdr_format: None,
        audio_tracks: None,
        subtitle_tracks: None,
        video_codec: None,
        video_profile: None,
        video_level: None,
        pixel_format: None,
        color_primaries: None,
        transfer_characteristics: None,
        matrix_coefficients: None,
        dovi_profile: None,
        max_cll: None,
        max_fall: None,
        mastering_display_primary_r: None,
        mastering_display_primary_g: None,
        mastering_display_primary_b: None,
        mastering_display_white_point: None,
        mastering_display_min_luminance: None,
        mastering_display_max_luminance: None,
        frame_rate: None,
    }
}

async fn seed_user(storage: &Arc<dyn Storage>, name: &str) -> UserId {
    let user = UserId::new();
    storage
        .upsert_user(&User {
            id: user,
            name: name.to_string(),
            password_hash: String::new(),
            is_admin: false,
            role: noflex_core::UserRole::User,
            created_at: OffsetDateTime::now_utc(),
        })
        .await
        .expect("seed user");
    user
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
async fn scrobble_reaches_linked_user_enriched_and_rotates_token() {
    let Some(h) = setup().await else {
        eprintln!("skipping: could not build the fixture-scrobbler plugin");
        return;
    };
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");
    let alien = work(
        lib.id,
        "Alien",
        Some(1979),
        vec![
            ExternalId {
                kind: ExternalIdKind::Tmdb,
                value: "603".to_string(),
            },
            ExternalId {
                kind: ExternalIdKind::Imdb,
                value: "tt0078748".to_string(),
            },
        ],
    );
    h.storage.upsert_work(&alien).await.expect("work");
    let edition = edition(alien.id, Some(7_020_000));
    h.storage.upsert_edition(&edition).await.expect("edition");
    let user = seed_user(&h.storage, "watcher").await;
    h.storage
        .set_user_plugin_secret(user, PLUGIN_ID, "access_token", "seed-token")
        .await
        .expect("seed user secret");

    let _dispatcher = spawn_scrobble_dispatcher(h.registry.clone(), h.storage.clone(), &h.bus);
    h.bus.emit(DomainEvent::ProgressAtStop {
        user,
        work: alien.id,
        title: alien.title.clone(),
        position_ms: 3_000_000,
    });

    let line = poll_until("fixture received the enriched scrobble", || async {
        received(&h.plugin)
            .await
            .into_iter()
            .find(|l| l.contains(&alien.id.to_string()))
    })
    .await;

    assert!(
        line.contains(&format!("user={user}")),
        "acting user on the wire: {line}"
    );
    assert!(line.contains("kind=movie"), "work kind enriched: {line}");
    assert!(line.contains("year=1979"), "year enriched: {line}");
    assert!(
        line.contains("ids=tmdb=603,imdb=tt0078748"),
        "external ids enriched, in stored order: {line}"
    );
    assert!(
        line.contains("action=stopped"),
        "progress-at-stop maps to stopped: {line}"
    );
    assert!(line.contains("pos=3000000"), "position carried: {line}");
    assert!(
        line.contains("runtime=7020000"),
        "runtime enriched from the edition: {line}"
    );
    assert!(
        line.contains("read=ok:seed-token"),
        "guest read the linked user's secret: {line}"
    );
    assert!(
        line.contains("write=ok"),
        "guest write-for-user succeeded: {line}"
    );

    let stored = h
        .storage
        .user_plugin_secret(user, PLUGIN_ID, "access_token")
        .await
        .expect("read back secret")
        .expect("secret row exists");
    assert_eq!(
        stored.value, "seed-token-rotated",
        "write-for-user persisted the rotated token"
    );
}

#[tokio::test]
async fn unlinked_user_is_skipped() {
    let Some(h) = setup().await else {
        eprintln!("skipping: could not build the fixture-scrobbler plugin");
        return;
    };
    let lib = library();
    h.storage.upsert_library(&lib).await.expect("library");
    let alien = work(lib.id, "Alien", Some(1979), vec![]);
    h.storage.upsert_work(&alien).await.expect("work");
    let linked = seed_user(&h.storage, "linked").await;
    let unlinked = seed_user(&h.storage, "unlinked").await;
    h.storage
        .set_user_plugin_secret(linked, PLUGIN_ID, "access_token", "seed-token")
        .await
        .expect("seed user secret");

    let _dispatcher = spawn_scrobble_dispatcher(h.registry.clone(), h.storage.clone(), &h.bus);
    // The unlinked user's event goes first; the broadcast channel keeps
    // order, so once the linked user's later event lands the earlier one
    // has already been through the fan-out.
    h.bus.emit(DomainEvent::PlaybackStopped {
        user: unlinked,
        work: alien.id,
        title: alien.title.clone(),
        media_file: noflex_core::MediaFileId::new(),
    });
    h.bus.emit(DomainEvent::PlaybackStarted {
        user: linked,
        work: alien.id,
        title: alien.title.clone(),
        media_file: noflex_core::MediaFileId::new(),
    });

    poll_until("fixture received the linked user's scrobble", || async {
        received(&h.plugin)
            .await
            .into_iter()
            .find(|l| l.contains(&linked.to_string()))
    })
    .await;

    let log = received(&h.plugin).await;
    assert!(
        !log.iter().any(|l| l.contains(&unlinked.to_string())),
        "a user with no linked secrets is never delivered: {log:?}"
    );
}
