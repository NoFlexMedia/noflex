// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{
    ExternalId, ExternalIdKind, LibraryId, MetadataStatus, SessionId, UserId, WorkId, WorkKind,
};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
use noflex_storage::{
    RequestUser,
    domain::{Library, LibraryConfig, LibraryKind, Work},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

fn as_request_user(principal: Principal) -> RequestUser {
    RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    }
}

const TEST_JWT_SECRET: &[u8] = b"test-secret-32-bytes-please-use-it";

async fn authed_schema() -> (
    noflex_graphql::NoflexSchema,
    Arc<JwtConfig>,
    Arc<TursoStore>,
) {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine).expect("registry"));
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        jwt: Some(jwt.clone()),
        plugins: Some(registry),
        priority_table: Some(Arc::new(tokio::sync::RwLock::new(
            noflex_plugin_host::PriorityTable::default(),
        ))),
        ..Default::default()
    });
    (schema, jwt, store)
}

async fn setup_admin(schema: &noflex_graphql::NoflexSchema, jwt: &JwtConfig) -> Principal {
    let resp = schema
        .execute(
            r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
    let resp = schema
        .execute(r#"mutation { login(username: "admin", password: "hunter2") { token } }"#)
        .await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let token = resp.data.into_json().unwrap()["login"]["token"]
        .as_str()
        .unwrap()
        .to_string();
    let claims = verify_jwt(jwt, &token).expect("verify");
    Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub uuid")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(claims.sid.parse().expect("sid uuid"))),
    }
}

async fn seed_library(store: &Arc<TursoStore>, name: &str) -> Library {
    let lib = Library {
        id: LibraryId::new(),
        name: name.to_string(),
        kind: LibraryKind::Movies,
        paths: vec![format!("/srv/{name}")],
        config: LibraryConfig::default(),
    };
    store.upsert_library(&lib).await.expect("library");
    lib
}

fn seed_work(library: LibraryId, title: &str) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
        year: None,
        release_date: None,
        overview: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        added_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn update_work_fields_edits_locks_and_marks_user_edited() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let lib = seed_library(&store, "Films").await;
    let work = seed_work(lib.id, "Alien");
    store.upsert_work(&work).await.expect("work");

    let edit = format!(
        r#"mutation {{ updateWorkFields(workId: "{}", fields: {{ title: "Alien: The Director's Cut", year: 1979 }}) {{ title year metadataStatus lockedFields }} }}"#,
        work.id
    );
    let resp = schema
        .execute(
            Request::new(edit.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "edit: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap()["updateWorkFields"].clone();
    assert_eq!(data["title"], "Alien: The Director's Cut");
    assert_eq!(data["year"], 1979);
    assert_eq!(data["metadataStatus"], "USER_EDITED");
    assert_eq!(data["lockedFields"], serde_json::json!(["TITLE", "YEAR"]));

    // Locks and status persist in storage, not just the response.
    let stored = store.get_work(work.id).await.expect("get").expect("work");
    assert_eq!(stored.metadata_status, MetadataStatus::UserEdited);
    assert_eq!(
        stored.locked_fields,
        vec![noflex_core::WorkField::Title, noflex_core::WorkField::Year]
    );

    let empty = format!(
        r#"mutation {{ updateWorkFields(workId: "{}", fields: {{}}) {{ id }} }}"#,
        work.id
    );
    let resp = schema
        .execute(
            Request::new(empty.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "no fields provided"),
        "empty input: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn set_work_field_lock_round_trips_sorted_and_deduped() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let lib = seed_library(&store, "Films").await;
    let work = seed_work(lib.id, "Alien");
    store.upsert_work(&work).await.expect("work");

    let lock = |field: &str, locked: bool| {
        format!(
            r#"mutation {{ setWorkFieldLock(workId: "{}", field: {field}, locked: {locked}) {{ lockedFields metadataStatus }} }}"#,
            work.id
        )
    };

    // Lock GENRES then YEAR: the response stays sorted (YEAR < GENRES).
    let resp = schema
        .execute(
            Request::new(lock("GENRES", true).as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "lock genres: {:?}", resp.errors);
    let resp = schema
        .execute(
            Request::new(lock("YEAR", true).as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "lock year: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap()["setWorkFieldLock"].clone();
    assert_eq!(data["lockedFields"], serde_json::json!(["YEAR", "GENRES"]));
    // Locking says nothing about provenance — status is untouched.
    assert_eq!(data["metadataStatus"], "UNCONFIRMED");

    // Re-locking an already-locked field stays a single entry.
    let resp = schema
        .execute(
            Request::new(lock("GENRES", true).as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    let data = resp.data.into_json().unwrap()["setWorkFieldLock"].clone();
    assert_eq!(data["lockedFields"], serde_json::json!(["YEAR", "GENRES"]));

    // Unlock removes just that field.
    let resp = schema
        .execute(
            Request::new(lock("YEAR", false).as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "unlock year: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap()["setWorkFieldLock"].clone();
    assert_eq!(data["lockedFields"], serde_json::json!(["GENRES"]));

    let stored = store.get_work(work.id).await.expect("get").expect("work");
    assert_eq!(stored.locked_fields, vec![noflex_core::WorkField::Genres]);
}

#[tokio::test]
async fn refresh_metadata_guards() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let lib = seed_library(&store, "Films").await;
    let mut work = seed_work(lib.id, "Alien");
    work.external_ids = vec![ExternalId {
        kind: ExternalIdKind::Tmdb,
        value: "348".into(),
    }];
    store.upsert_work(&work).await.expect("work");

    // No external ids → identify first.
    let bare = seed_work(lib.id, "Unidentified");
    store.upsert_work(&bare).await.expect("bare work");
    let refresh = format!(
        r#"mutation {{ refreshMetadata(workId: "{}") {{ id }} }}"#,
        bare.id
    );
    let resp = schema
        .execute(
            Request::new(refresh.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "work has no external ids; identify it first"),
        "no external ids: {:?}",
        resp.errors
    );

    // User-edited work without force → pass force.
    let mut edited = work.clone();
    edited.metadata_status = MetadataStatus::UserEdited;
    store.upsert_work(&edited).await.expect("edited work");
    let resp = schema
        .execute(
            Request::new(
                format!(
                    r#"mutation {{ refreshMetadata(workId: "{}") {{ id }} }}"#,
                    work.id
                )
                .as_str(),
            )
            .data(admin)
            .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "work has user edits; pass force: true to refresh anyway"),
        "user edits: {:?}",
        resp.errors
    );

    // force + an external id, but no provider is loaded.
    let resp = schema
        .execute(
            Request::new(
                format!(
                    r#"mutation {{ refreshMetadata(workId: "{}", force: true) {{ id }} }}"#,
                    work.id
                )
                .as_str(),
            )
            .data(admin)
            .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "no metadata provider could fetch this work"),
        "no provider: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn identify_work_rejects_unknown_provider() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let lib = seed_library(&store, "Films").await;
    let work = seed_work(lib.id, "Alien");
    store.upsert_work(&work).await.expect("work");

    let identify = format!(
        r#"mutation {{ identifyWork(workId: "{}", provider: "ghost", externalId: {{ kind: TMDB, value: "348" }}) {{ id }} }}"#,
        work.id
    );
    let resp = schema
        .execute(
            Request::new(identify.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "provider not loaded: ghost"),
        "unknown provider: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn metadata_mutations_require_authentication() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema, &jwt).await;
    let lib = seed_library(&store, "Films").await;
    let work = seed_work(lib.id, "Alien");
    store.upsert_work(&work).await.expect("work");

    for mutation in [
        format!(
            r#"mutation {{ refreshMetadata(workId: "{}") {{ id }} }}"#,
            work.id
        ),
        format!(
            r#"mutation {{ identifyWork(workId: "{}", provider: "tmdb", externalId: {{ kind: TMDB, value: "348" }}) {{ id }} }}"#,
            work.id
        ),
        format!(
            r#"mutation {{ updateWorkFields(workId: "{}", fields: {{ title: "X" }}) {{ id }} }}"#,
            work.id
        ),
        format!(
            r#"mutation {{ setWorkFieldLock(workId: "{}", field: TITLE, locked: true) {{ id }} }}"#,
            work.id
        ),
    ] {
        let resp = schema.execute(mutation.as_str()).await;
        assert!(
            resp.errors
                .iter()
                .any(|e| e.message == "authentication required"),
            "anonymous must be rejected: {mutation}: {:?}",
            resp.errors
        );
    }
}
