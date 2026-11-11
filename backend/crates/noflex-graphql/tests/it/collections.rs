// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{LibraryId, SessionId, TagId, TagTarget, UserId, WorkId, WorkKind};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{
    RequestUser,
    domain::{Edition, Library, LibraryConfig, LibraryKind, Tag, UserRestrictions, Work},
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
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        jwt: Some(jwt.clone()),
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
    let token = login(schema, "admin", "hunter2").await;
    principal_from_token(jwt, &token)
}

async fn login(schema: &noflex_graphql::NoflexSchema, username: &str, password: &str) -> String {
    let q = format!(
        r#"mutation {{ login(username: "{username}", password: "{password}") {{ token }} }}"#
    );
    let resp = schema.execute(q.as_str()).await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    resp.data.into_json().unwrap()["login"]["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn principal_from_token(jwt: &JwtConfig, token: &str) -> Principal {
    let claims = verify_jwt(jwt, token).expect("verify");
    Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub uuid")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(claims.sid.parse().expect("sid uuid"))),
    }
}

fn authed_request(query: impl Into<String>, principal: Principal) -> Request {
    Request::new(query).data(principal).data(RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    })
}

async fn create_user(
    schema: &noflex_graphql::NoflexSchema,
    admin: Principal,
    username: &str,
) -> Principal {
    let q = format!(
        r#"mutation {{ createUser(username: "{username}", password: "pw1", role: USER) {{ id }} }}"#
    );
    let resp = schema
        .execute(Request::new(q).data(admin).data(as_request_user(admin)))
        .await;
    assert!(resp.errors.is_empty(), "create user: {:?}", resp.errors);
    let token = login(schema, username, "pw1").await;
    let jwt = JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config");
    principal_from_token(&jwt, &token)
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
        metadata_status: noflex_core::MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    }
}

fn seed_edition(work: WorkId, name: &str) -> Edition {
    Edition {
        id: noflex_core::EditionId::new(),
        work_id: work,
        name: name.to_string(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    }
}

#[tokio::test]
async fn collection_mutations_require_admin() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let bob = create_user(&schema, admin, "bob").await;
    let lib = seed_library(&store, "Films").await;

    let create = format!(
        r#"mutation {{ createCollection(libraryId: "{}", title: "Marvel") {{ id }} }}"#,
        lib.id
    );

    // Anonymous: no Principal at all.
    let resp = schema.execute(create.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous: {:?}",
        resp.errors
    );

    // Authenticated but not an admin.
    let resp = schema
        .execute(
            Request::new(create.as_str())
                .data(bob)
                .data(as_request_user(bob)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "administrator required"),
        "non-admin: {:?}",
        resp.errors
    );

    // The admin gets through.
    let resp = schema
        .execute(
            Request::new(create.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "admin: {:?}", resp.errors);
}

#[tokio::test]
async fn collection_lifecycle_with_ordering() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let lib = seed_library(&store, "Films").await;
    let alien = seed_work(lib.id, "Alien");
    let blade = seed_work(lib.id, "Blade Runner");
    store.upsert_work(&alien).await.expect("alien");
    store.upsert_work(&blade).await.expect("blade");

    // sortTitle defaults to title.
    let create = format!(
        r#"mutation {{ createCollection(libraryId: "{}", title: "Sci-Fi Classics") {{ id title sortTitle }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(create.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "create: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["createCollection"]["sortTitle"], "Sci-Fi Classics");
    let collection_id = data["createCollection"]["id"].as_str().unwrap().to_string();

    // Anonymous collection reads are rejected; authenticated reads succeed.
    let list = format!(
        r#"{{ collections(libraryId: "{}") {{ nodes {{ title }} pageInfo {{ hasNextPage endCursor }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(list.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous list: {:?}",
        resp.errors
    );
    let resp = schema.execute(authed_request(list.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "list: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["collections"]["nodes"].as_array().unwrap().len(), 1);

    // Membership replace preserves the given order.
    let set = format!(
        r#"mutation {{ setCollectionItems(id: "{collection_id}", workIds: ["{}", "{}"]) {{ id }} }}"#,
        alien.id, blade.id
    );
    let resp = schema.execute(authed_request(set.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "set: {:?}", resp.errors);

    let items = format!(
        r#"{{ collection(id: "{collection_id}") {{ items {{ nodes {{ id }} pageInfo {{ hasNextPage endCursor }} }} }} }}"#
    );
    let resp = schema.execute(authed_request(items.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "items: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let ids: Vec<&str> = data["collection"]["items"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [alien.id.to_string(), blade.id.to_string()]);

    let first_page = format!(
        r#"{{ collection(id: "{collection_id}") {{ items(first: 1) {{ nodes {{ id }} pageInfo {{ hasNextPage endCursor }} }} }} }}"#
    );
    let resp = schema.execute(authed_request(first_page, admin)).await;
    assert!(resp.errors.is_empty(), "first page: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert!(
        data["collection"]["items"]["pageInfo"]["hasNextPage"]
            .as_bool()
            .unwrap()
    );
    let cursor = data["collection"]["items"]["pageInfo"]["endCursor"]
        .as_str()
        .unwrap();
    let final_page = format!(
        r#"{{ collection(id: "{collection_id}") {{ items(first: 1, after: "{cursor}") {{ nodes {{ id }} pageInfo {{ hasNextPage endCursor }} }} }} }}"#
    );
    let resp = schema.execute(authed_request(final_page, admin)).await;
    assert!(resp.errors.is_empty(), "final page: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert!(
        !data["collection"]["items"]["pageInfo"]["hasNextPage"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        data["collection"]["items"]["nodes"][0]["id"],
        blade.id.to_string()
    );

    let oversized = format!(
        r#"{{ collection(id: "{collection_id}") {{ items(first: 100000) {{ nodes {{ id }} pageInfo {{ hasNextPage }} }} }} }}"#
    );
    let resp = schema.execute(authed_request(oversized, admin)).await;
    assert!(
        resp.errors.is_empty(),
        "oversized page is clamped before budgeting: {:?}",
        resp.errors
    );

    let invalid = format!(
        r#"{{ collection(id: "{collection_id}") {{ items(first: 1, after: "bad") {{ pageInfo {{ hasNextPage }} }} }} }}"#
    );
    let resp = schema.execute(authed_request(invalid, admin)).await;
    assert!(
        resp.errors
            .iter()
            .any(|error| error.message.contains("invalid collection item cursor")),
        "invalid cursor: {:?}",
        resp.errors
    );

    // Reorder via a second set.
    let set = format!(
        r#"mutation {{ setCollectionItems(id: "{collection_id}", workIds: ["{}", "{}"]) {{ id }} }}"#,
        blade.id, alien.id
    );
    let resp = schema.execute(authed_request(set.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "reorder: {:?}", resp.errors);
    let resp = schema.execute(authed_request(items.as_str(), admin)).await;
    let data = resp.data.into_json().unwrap();
    let ids: Vec<&str> = data["collection"]["items"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [blade.id.to_string(), alien.id.to_string()]);

    // Update renames; delete reports existence truthfully.
    let update = format!(
        r#"mutation {{ updateCollection(id: "{collection_id}", title: "Renamed") {{ title }} }}"#
    );
    let resp = schema.execute(authed_request(update.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "update: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["updateCollection"]["title"],
        "Renamed"
    );

    let delete = format!(r#"mutation {{ deleteCollection(id: "{collection_id}") {{ id }} }}"#);
    for expected in [true, false] {
        let resp = schema.execute(authed_request(delete.as_str(), admin)).await;
        assert!(resp.errors.is_empty(), "delete: {:?}", resp.errors);
        assert_eq!(
            resp.data.into_json().unwrap()["deleteCollection"].is_object(),
            expected
        );
    }

    let single = format!(r#"{{ collection(id: "{collection_id}") {{ id }} }}"#);
    let resp = schema.execute(authed_request(single.as_str(), admin)).await;
    assert!(resp.data.into_json().unwrap()["collection"].is_null());
}

#[tokio::test]
async fn set_collection_items_rejects_works_from_other_libraries() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let films = seed_library(&store, "Films").await;
    let shows = seed_library(&store, "Shows").await;
    let alien = seed_work(films.id, "Alien");
    let sopranos = seed_work(shows.id, "The Sopranos");
    store.upsert_work(&alien).await.expect("alien");
    store.upsert_work(&sopranos).await.expect("sopranos");

    let create = format!(
        r#"mutation {{ createCollection(libraryId: "{}", title: "Films Only") {{ id }} }}"#,
        films.id
    );
    let resp = schema
        .execute(
            Request::new(create.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    let collection_id = resp.data.into_json().unwrap()["createCollection"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // A work from another library is rejected, naming the id.
    let set = format!(
        r#"mutation {{ setCollectionItems(id: "{collection_id}", workIds: ["{}"]) {{ id }} }}"#,
        sopranos.id
    );
    let resp = schema
        .execute(
            Request::new(set.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message.contains(sopranos.id.to_string().as_str())),
        "cross-library: {:?}",
        resp.errors
    );

    // An unknown id is rejected, naming the id.
    let ghost = WorkId::new();
    let set = format!(
        r#"mutation {{ setCollectionItems(id: "{collection_id}", workIds: ["{ghost}"]) {{ id }} }}"#
    );
    let resp = schema
        .execute(
            Request::new(set.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message.contains(&ghost.to_string())),
        "unknown work: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn plugin_collections_are_edited_only_after_detaching() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let films = seed_library(&store, "Films").await;
    let alien = seed_work(films.id, "Alien");
    store.upsert_work(&alien).await.expect("alien");
    let synced = store
        .sync_plugin_collection(&noflex_storage::domain::PluginCollectionSync {
            plugin_id: "trakt".into(),
            key: "watchlist".into(),
            title: "Watchlist".into(),
            overview: None,
            works: vec![alien.id],
        })
        .await
        .expect("sync");
    let id = synced[0].id;
    let run = |query: String| {
        let schema = schema.clone();
        async move {
            schema
                .execute(Request::new(query).data(admin).data(as_request_user(admin)))
                .await
        }
    };
    let code = |resp: &async_graphql::Response| {
        resp.errors
            .first()
            .and_then(|e| e.extensions.as_ref())
            .and_then(|ext| ext.get("code").cloned())
    };

    let resp = run(format!(
        r#"{{ collection(id: "{id}") {{ title pluginId }} }}"#
    ))
    .await;
    assert_eq!(
        resp.data.into_json().unwrap()["collection"]["pluginId"],
        "trakt"
    );
    for edit in [
        format!(r#"mutation {{ setCollectionItems(id: "{id}", workIds: []) {{ id }} }}"#),
        format!(r#"mutation {{ updateCollection(id: "{id}", title: "Mine") {{ id }} }}"#),
    ] {
        let resp = run(edit).await;
        assert_eq!(
            code(&resp),
            Some(async_graphql::Value::from("PLUGIN_OWNED_COLLECTION")),
            "{:?}",
            resp.errors
        );
    }

    let resp = run(format!(
        r#"mutation {{ detachCollection(id: "{id}") {{ id pluginId }} }}"#
    ))
    .await;
    assert!(resp.errors.is_empty(), "detach: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["detachCollection"]["pluginId"].is_null());
    let resp = run(format!(
        r#"mutation {{ updateCollection(id: "{id}", title: "Mine") {{ title }} }}"#
    ))
    .await;
    assert!(
        resp.errors.is_empty(),
        "edit after detach: {:?}",
        resp.errors
    );

    let resp = run(
        r#"mutation { removePluginData(pluginId: "trakt") { tagAssignments collections } }"#.into(),
    )
    .await;
    assert!(resp.errors.is_empty(), "remove: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["removePluginData"]["collections"],
        0,
        "a detached collection is the administrator's"
    );
    assert!(store.get_collection(id).await.expect("get").is_some());
}

#[tokio::test]
async fn collection_items_hide_forbidden_works() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let bob = create_user(&schema, admin, "restricted-bob").await;
    let lib = seed_library(&store, "Rated Films").await;
    store
        .grant_library_access(bob.user_id, lib.id)
        .await
        .expect("library grant");

    let allowed = seed_work(lib.id, "Allowed");
    let blocked = seed_work(lib.id, "Blocked");
    store.upsert_work(&allowed).await.expect("allowed work");
    store.upsert_work(&blocked).await.expect("blocked work");
    let allowed_rating = Tag {
        id: TagId::new(),
        namespace: "parental".to_string(),
        parent_id: None,
        slug: "pg".to_string(),
        display: "PG".to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    let blocked_rating = Tag {
        id: TagId::new(),
        namespace: "parental".to_string(),
        parent_id: None,
        slug: "r".to_string(),
        display: "R".to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    store
        .upsert_tag(&allowed_rating)
        .await
        .expect("allowed tag");
    store
        .upsert_tag(&blocked_rating)
        .await
        .expect("blocked tag");
    store
        .assign_tag(allowed_rating.id, TagTarget::Work(allowed.id), 1.0, "test")
        .await
        .expect("allowed rating");
    store
        .assign_tag(blocked_rating.id, TagTarget::Work(blocked.id), 1.0, "test")
        .await
        .expect("blocked rating");
    store
        .set_user_restrictions(&UserRestrictions {
            user_id: bob.user_id,
            allowed_parental_rating_ids: vec![allowed_rating.id],
            allow_unrated: false,
            max_stream_bitrate_bps: None,
            max_concurrent_transcodes: None,
        })
        .await
        .expect("restrictions");

    let create = format!(
        r#"mutation {{ createCollection(libraryId: "{}", title: "Ratings") {{ id }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(create, admin)).await;
    let collection_id = resp.data.into_json().unwrap()["createCollection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let set = format!(
        r#"mutation {{ setCollectionItems(id: "{collection_id}", workIds: ["{}", "{}"]) {{ id }} }}"#,
        allowed.id, blocked.id
    );
    let resp = schema.execute(authed_request(set, admin)).await;
    assert!(resp.errors.is_empty(), "set: {:?}", resp.errors);

    let query = format!(
        r#"{{ collection(id: "{collection_id}") {{ items {{ nodes {{ id }} pageInfo {{ hasNextPage endCursor }} }} }} }}"#
    );
    let resp = schema.execute(authed_request(query, bob)).await;
    assert!(resp.errors.is_empty(), "items: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let items = data["collection"]["items"]["nodes"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], allowed.id.to_string());
}

#[tokio::test]
async fn playlists_are_isolated_per_user() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let bob = create_user(&schema, admin, "bob").await;

    let resp = schema
        .execute(
            Request::new(r#"mutation { createPlaylist(title: "Admin's Mix") { id title } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "create: {:?}", resp.errors);
    let playlist_id = resp.data.into_json().unwrap()["createPlaylist"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Bob sees no playlists, and the admin's playlist is null to him.
    let resp = schema
        .execute(
            Request::new(r#"{ playlists { nodes { id } pageInfo { hasNextPage endCursor } } }"#)
                .data(bob)
                .data(as_request_user(bob)),
        )
        .await;
    assert!(resp.errors.is_empty(), "bob list: {:?}", resp.errors);
    assert!(
        resp.data.into_json().unwrap()["playlists"]["nodes"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let single = format!(r#"{{ playlist(id: "{playlist_id}") {{ id }} }}"#);
    let resp = schema
        .execute(
            Request::new(single.as_str())
                .data(bob)
                .data(as_request_user(bob)),
        )
        .await;
    assert!(resp.errors.is_empty(), "bob get: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["playlist"].is_null());

    // Bob cannot mutate it either; the error doesn't confirm it exists.
    for mutation in [
        format!(
            r#"mutation {{ updatePlaylist(id: "{playlist_id}", title: "Hijacked") {{ id }} }}"#
        ),
        format!(r#"mutation {{ deletePlaylist(id: "{playlist_id}") {{ id }} }}"#),
        format!(r#"mutation {{ setPlaylistItems(id: "{playlist_id}", items: []) {{ id }} }}"#),
    ] {
        let resp = schema
            .execute(Request::new(mutation).data(bob).data(as_request_user(bob)))
            .await;
        assert!(
            resp.errors
                .iter()
                .any(|e| e.message == "playlist not found"),
            "bob mutation: {:?}",
            resp.errors
        );
    }

    // The admin's own view is unaffected.
    let resp = schema
        .execute(
            Request::new(r#"{ playlists { nodes { title } } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["playlists"]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(data["playlists"]["nodes"][0]["title"], "Admin's Mix");
}

#[tokio::test]
async fn playlist_mixed_items_round_trip() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let lib = seed_library(&store, "Films").await;
    let alien = seed_work(lib.id, "Alien");
    let blade = seed_work(lib.id, "Blade Runner");
    store.upsert_work(&alien).await.expect("alien");
    store.upsert_work(&blade).await.expect("blade");
    let directors_cut = seed_edition(blade.id, "Director's Cut");
    store.upsert_edition(&directors_cut).await.expect("edition");

    let resp = schema
        .execute(authed_request(
            r#"mutation { createPlaylist(title: "Mixed") { id } }"#,
            admin,
        ))
        .await;
    let playlist_id = resp.data.into_json().unwrap()["createPlaylist"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Work and edition targets interleave in the given order.
    let set = format!(
        r#"mutation {{ setPlaylistItems(id: "{playlist_id}", items: [
            {{ kind: WORK, id: "{}" }},
            {{ kind: EDITION, id: "{}" }}
        ]) {{ id }} }}"#,
        alien.id, directors_cut.id
    );
    let resp = schema.execute(authed_request(set.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "set: {:?}", resp.errors);

    let items = format!(
        r#"{{ playlist(id: "{playlist_id}") {{ items {{ nodes {{ id kind work {{ title }} edition {{ name }} }} pageInfo {{ hasNextPage endCursor }} }} }} }}"#
    );
    let resp = schema.execute(authed_request(items.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "items: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let entries = data["playlist"]["items"]["nodes"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["id"], alien.id.to_string());
    assert_eq!(entries[0]["kind"], "WORK");
    assert_eq!(entries[0]["work"]["title"], "Alien");
    assert!(entries[0]["edition"].is_null());
    assert_eq!(entries[1]["id"], directors_cut.id.to_string());
    assert_eq!(entries[1]["kind"], "EDITION");
    assert_eq!(entries[1]["edition"]["name"], "Director's Cut");
    assert!(entries[1]["work"].is_null());

    // addPlaylistItems appends after the existing entries.
    let add = format!(
        r#"mutation {{ addPlaylistItems(id: "{playlist_id}", items: [
            {{ kind: WORK, id: "{}" }}
        ]) {{ id }} }}"#,
        blade.id
    );
    let resp = schema.execute(authed_request(add.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "add: {:?}", resp.errors);
    let resp = schema.execute(authed_request(items.as_str(), admin)).await;
    let data = resp.data.into_json().unwrap();
    let entries = data["playlist"]["items"]["nodes"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[2]["kind"], "WORK");
    assert_eq!(entries[2]["work"]["title"], "Blade Runner");

    // An unknown target id fails the whole mutation, naming the id.
    let ghost = WorkId::new();
    let set = format!(
        r#"mutation {{ setPlaylistItems(id: "{playlist_id}", items: [
            {{ kind: WORK, id: "{ghost}" }}
        ]) {{ id }} }}"#
    );
    let resp = schema.execute(authed_request(set.as_str(), admin)).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message.contains(&ghost.to_string())),
        "unknown target: {:?}",
        resp.errors
    );

    // Owner lifecycle: rename, then delete reports existence.
    let update = format!(
        r#"mutation {{ updatePlaylist(id: "{playlist_id}", title: "Renamed Mix") {{ title }} }}"#
    );
    let resp = schema.execute(authed_request(update.as_str(), admin)).await;
    assert!(resp.errors.is_empty(), "update: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["updatePlaylist"]["title"],
        "Renamed Mix"
    );

    let delete = format!(r#"mutation {{ deletePlaylist(id: "{playlist_id}") {{ id }} }}"#);
    for expected in [true, false] {
        let resp = schema.execute(authed_request(delete.as_str(), admin)).await;
        assert!(resp.errors.is_empty(), "delete: {:?}", resp.errors);
        assert_eq!(
            resp.data.into_json().unwrap()["deletePlaylist"].is_object(),
            expected
        );
    }
}

#[tokio::test]
async fn playlist_queries_reject_anonymous() {
    let (schema, jwt, _store) = authed_schema().await;
    setup_admin(&schema, &jwt).await;

    for query in [
        r#"{ playlists { nodes { id } } }"#,
        r#"mutation { createPlaylist(title: "Nope") { id } }"#,
    ] {
        let resp = schema.execute(query).await;
        assert!(
            resp.errors
                .iter()
                .any(|e| e.message == "authentication required"),
            "anonymous must be rejected: {query}: {:?}",
            resp.errors
        );
    }
}
