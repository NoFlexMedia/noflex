// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{
    EmbeddingTarget, ExternalId, ExternalIdKind, LibraryId, MetadataStatus, SessionId, UserId,
    WorkId, WorkKind,
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
        metadata_provider: None,
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
    store
        .upsert_embedding(EmbeddingTarget::Work(work.id), "ns", "model", &[1.0, 0.0])
        .await
        .expect("vector");

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

    // The vector described the old text, so it goes back in the queue.
    let queued = store
        .works_missing_embedding("ns", 10, None)
        .await
        .expect("missing");
    assert!(queued.items.iter().any(|item| item.id == work.id));

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
async fn attributions_credit_metadata_then_art_in_use() {
    use noflex_core::ImageKind;
    use noflex_storage::domain::{
        ArtworkPreference, Image, ImageSource, ImageTarget, ProviderAttribution,
    };

    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let mut lib = seed_library(&store, "Films").await;
    for (provider, logo) in [("meta", Some("d1")), ("art", None), ("other", None)] {
        store
            .upsert_provider_attribution(
                &ProviderAttribution {
                    provider: provider.into(),
                    name: provider.to_uppercase(),
                    text: format!("Data from {provider}."),
                    url: None,
                    logo_mime: logo.map(|_| "image/png".to_string()),
                    logo_digest: logo.map(str::to_string),
                },
                logo.map(|_| &b"\x89PNG"[..]),
            )
            .await
            .expect("attribution");
    }
    let mut work = seed_work(lib.id, "Alien");
    work.metadata_provider = Some("meta".into());
    store.upsert_work(&work).await.expect("work");
    let image = |kind, source, provider: Option<&str>| Image {
        target: ImageTarget::Work(work.id),
        kind,
        source,
        path: "/art.png".into(),
        width: None,
        height: None,
        language: None,
        provider: provider.map(str::to_string),
    };
    for row in [
        image(ImageKind::Primary, ImageSource::Provider, Some("art")),
        image(ImageKind::Logo, ImageSource::Provider, Some("meta")),
        image(ImageKind::Backdrop, ImageSource::Provider, Some("gone")),
    ] {
        store.upsert_image(&row).await.expect("image");
    }

    let credits = |schema: noflex_graphql::NoflexSchema| {
        let query = format!(
            r#"{{ work(id: "{}") {{ attributions {{ provider name logoUrl logoMime }} }} }}"#,
            work.id
        );
        async move {
            let resp = schema
                .execute(Request::new(query).data(admin).data(as_request_user(admin)))
                .await;
            assert!(resp.errors.is_empty(), "{:?}", resp.errors);
            resp.data.into_json().expect("json")["work"]["attributions"].clone()
        }
    };
    assert_eq!(
        credits(schema.clone()).await,
        serde_json::json!([
            { "provider": "meta", "name": "META", "logoUrl": "/attributions/meta/logo?v=d1", "logoMime": "image/png" },
            { "provider": "art", "name": "ART", "logoUrl": null, "logoMime": null },
        ]),
        "metadata first, each provider once, and one with no stored credit left out"
    );

    // A local poster in use takes the art provider's credit with it.
    lib.config.artwork_preference = ArtworkPreference::PreferLocal;
    store.upsert_library(&lib).await.expect("library");
    store
        .upsert_image(&image(ImageKind::Primary, ImageSource::Local, None))
        .await
        .expect("image");
    let names: Vec<String> = credits(schema.clone())
        .await
        .as_array()
        .expect("list")
        .iter()
        .map(|credit| credit["provider"].as_str().expect("id").to_string())
        .collect();
    assert_eq!(names, ["meta"]);
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
