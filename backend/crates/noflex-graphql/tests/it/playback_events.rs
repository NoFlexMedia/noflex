// SPDX-License-Identifier: AGPL-3.0-only
//
// Domain-event emission from the playback mutations. The bus is
// optional schema data; these tests wire one in and assert the
// events land on a subscribed receiver (and that Playing/Paused
// heartbeats stay silent).

use std::{sync::Arc, time::Duration};

use async_graphql::Request;
use noflex_auth::Principal;
use noflex_core::{DomainEvent, DomainEventBus, LibraryId, UserId, WorkId, WorkKind};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_storage::{
    RequestUser,
    domain::{Library, LibraryConfig, LibraryKind, Work},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

const TEST_JWT_SECRET: &[u8] = b"test-secret-32-bytes-please-use-it";

async fn authed_schema() -> (NoflexSchema, Arc<TursoStore>, DomainEventBus) {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let bus = DomainEventBus::default();
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        jwt: Some(Arc::new(
            noflex_auth::JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"),
        )),
        event_bus: Some(bus.clone()),
        ..Default::default()
    });
    (schema, store, bus)
}

/// Run the setup wizard and return the `Principal` the HTTP layer
/// would attach to that admin's subsequent requests.
async fn setup_admin(schema: &NoflexSchema) -> Principal {
    let resp = schema
        .execute(
            r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
    let user_id = resp.data.into_json().unwrap()["completeSetupWizard"]["userId"]
        .as_str()
        .expect("userId")
        .to_string();
    Principal {
        user_id: UserId::from_uuid(user_id.parse().expect("user id is a uuid")),
        is_admin: true,
        session_id: None,
    }
}

fn authed_request(query: impl Into<String>, principal: Principal) -> Request {
    Request::new(query).data(principal).data(RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    })
}

fn seed_library() -> Library {
    Library {
        id: LibraryId::new(),
        name: "Films".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/films".to_string()],
        config: LibraryConfig::default(),
    }
}

fn seed_work(library: LibraryId, title: &str) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library,
        title: title.to_string(),
        sort_title: title.to_string(),
        year: Some(1979),
        release_date: None,
        overview: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: noflex_core::MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        added_at: now,
        updated_at: now,
    }
}

fn seed_edition(work: WorkId) -> noflex_storage::domain::Edition {
    noflex_storage::domain::Edition {
        id: noflex_core::EditionId::new(),
        work_id: work,
        name: "Default".to_string(),
        runtime_ms: Some(7_020_000),
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    }
}

#[tokio::test]
async fn playback_mutations_emit_domain_events() {
    let (schema, store, bus) = authed_schema().await;
    let principal = setup_admin(&schema).await;

    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");
    let work = seed_work(lib.id, "Alien");
    store.upsert_work(&work).await.expect("work");

    let mut rx = bus.subscribe();
    let target = format!(r#"{{ kind: WORK, id: "{}" }}"#, work.id);

    let mark = format!(r#"mutation {{ markPlayed(target: {target}) {{ played }} }}"#);
    let resp = schema
        .execute(authed_request(mark.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "markPlayed: {:?}", resp.errors);
    match rx.recv().await.expect("event") {
        DomainEvent::WorkPlayed {
            user,
            work: w,
            title,
        } => {
            assert_eq!(user, principal.user_id);
            assert_eq!(w, work.id);
            assert_eq!(title, "Alien");
        }
        other => panic!("expected WorkPlayed, got {other:?}"),
    }

    let fav =
        format!(r#"mutation {{ setFavorite(target: {target}, favorite: true) {{ favorite }} }}"#);
    let resp = schema
        .execute(authed_request(fav.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "setFavorite: {:?}", resp.errors);
    match rx.recv().await.expect("event") {
        DomainEvent::Favorite {
            user,
            work: w,
            title,
            favorite,
        } => {
            assert_eq!(user, principal.user_id);
            assert_eq!(w, work.id);
            assert_eq!(title, "Alien");
            assert!(favorite);
        }
        other => panic!("expected Favorite, got {other:?}"),
    }

    // A Playing heartbeat must NOT emit.
    let heartbeat = format!(
        r#"mutation {{ reportProgress(target: {target}, positionMs: 42000, state: PLAYING) {{ positionMs }} }}"#
    );
    let resp = schema
        .execute(authed_request(heartbeat.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "reportProgress: {:?}", resp.errors);
    let silent = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(silent.is_err(), "Playing heartbeat emitted an event");

    // The terminal stop emits ProgressAtStop with the position.
    let stop = format!(
        r#"mutation {{ reportProgress(target: {target}, positionMs: 42000, state: STOPPED) {{ positionMs }} }}"#
    );
    let resp = schema
        .execute(authed_request(stop.as_str(), principal))
        .await;
    assert!(
        resp.errors.is_empty(),
        "reportProgress stop: {:?}",
        resp.errors
    );
    match rx.recv().await.expect("event") {
        DomainEvent::ProgressAtStop {
            user,
            work: w,
            title,
            position_ms,
        } => {
            assert_eq!(user, principal.user_id);
            assert_eq!(w, work.id);
            assert_eq!(title, "Alien");
            assert_eq!(position_ms, 42000);
        }
        other => panic!("expected ProgressAtStop, got {other:?}"),
    }
}

#[tokio::test]
async fn progress_past_the_completion_point_finishes_the_work_once() {
    let (schema, store, bus) = authed_schema().await;
    let principal = setup_admin(&schema).await;

    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");
    let work = seed_work(lib.id, "Alien");
    store.upsert_work(&work).await.expect("work");
    store
        .upsert_edition(&seed_edition(work.id))
        .await
        .expect("edition");

    let mut rx = bus.subscribe();
    let report = async |position: i64| {
        let query = format!(
            r#"mutation {{ reportProgress(target: {{ kind: WORK, id: "{}" }}, positionMs: {position}, state: PLAYING) {{ played }} }}"#,
            work.id
        );
        let resp = schema
            .execute(authed_request(query.as_str(), principal))
            .await;
        assert!(resp.errors.is_empty(), "reportProgress: {:?}", resp.errors);
        resp.data.into_json().unwrap()["reportProgress"]["played"]
            .as_bool()
            .expect("played")
    };

    // 90% of the seeded 7,020,000 ms runtime is 6,318,000.
    assert!(!report(6_000_000).await);
    let silent = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(silent.is_err(), "no completion before the threshold");

    assert!(report(6_400_000).await);
    assert!(matches!(
        rx.recv().await.expect("event"),
        DomainEvent::WorkPlayed { .. }
    ));

    assert!(report(6_500_000).await);
    let silent = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(silent.is_err(), "completion is announced once");
}

#[tokio::test]
async fn edition_target_resolves_to_work_event() {
    let (schema, store, bus) = authed_schema().await;
    let principal = setup_admin(&schema).await;

    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");
    let work = seed_work(lib.id, "Alien");
    store.upsert_work(&work).await.expect("work");
    let edition = seed_edition(work.id);
    store.upsert_edition(&edition).await.expect("edition");

    let mut rx = bus.subscribe();
    let target = format!(r#"{{ kind: EDITION, id: "{}" }}"#, edition.id);
    let mark = format!(r#"mutation {{ markPlayed(target: {target}) {{ played }} }}"#);
    let resp = schema
        .execute(authed_request(mark.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "markPlayed: {:?}", resp.errors);
    match rx.recv().await.expect("event") {
        DomainEvent::WorkPlayed {
            user,
            work: w,
            title,
        } => {
            assert_eq!(user, principal.user_id);
            assert_eq!(w, work.id, "edition target must resolve to its work");
            assert_eq!(title, "Alien");
        }
        other => panic!("expected WorkPlayed, got {other:?}"),
    }
}

#[tokio::test]
async fn unresolved_target_returns_not_found_without_event() {
    let (schema, _store, bus) = authed_schema().await;
    let principal = setup_admin(&schema).await;

    let mut rx = bus.subscribe();
    let bogus = uuid::Uuid::now_v7();
    let mark = format!(
        r#"mutation {{ markPlayed(target: {{ kind: WORK, id: "{bogus}" }}) {{ played }} }}"#
    );
    let resp = schema
        .execute(authed_request(mark.as_str(), principal))
        .await;
    assert_eq!(resp.errors.len(), 1, "markPlayed errors: {:?}", resp.errors);
    assert_eq!(resp.errors[0].message, "target not found");
    let silent = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(silent.is_err(), "unresolved target must not emit");
}
