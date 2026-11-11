// SPDX-License-Identifier: AGPL-3.0-only
//
// Headless client-contract smoke suite. Walks the flow a real client
// takes today — discover server state, run the setup wizard, use the
// issued token, then exercise the library surface — so login, browsing,
// and streaming changes land on a regression net instead of bare schema.
// Extend this file as those surfaces ship.

use std::sync::Arc;

use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{LibraryId, SessionId, UserId, WorkId, WorkKind};
use noflex_graphql::{
    PlaybackRouteKind, PlaybackSession, PlaybackSessionService, SchemaContext, build_schema,
};
use noflex_storage::{
    RequestUser,
    domain::{Library, LibraryConfig, LibraryKind, User, Work},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

const TEST_JWT_SECRET: &[u8] = b"contract-secret-32-bytes-for-tests";

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

/// Run the setup wizard and return the `(user_id, Principal)` pair the
/// HTTP layer would hand resolvers for that user's subsequent requests.
async fn run_setup_wizard(schema: &noflex_graphql::NoflexSchema) -> (String, Principal) {
    let resp = schema
        .execute(r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId accessToken } }"#)
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let user_id = data["completeSetupWizard"]["userId"]
        .as_str()
        .expect("userId")
        .to_string();
    let principal = Principal {
        user_id: UserId::from_uuid(user_id.parse().expect("user id is a uuid")),
        is_admin: true,
        session_id: None,
    };
    (user_id, principal)
}

fn request_user(principal: Principal) -> RequestUser {
    RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    }
}

fn authed_request(query: impl Into<String>, principal: Principal) -> async_graphql::Request {
    async_graphql::Request::new(query)
        .data(principal)
        .data(request_user(principal))
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

fn seed_work(library: LibraryId, kind: WorkKind, title: &str, year: Option<i32>) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind,
        parent_id: None,
        library_id: library,
        title: title.to_string(),
        sort_title: title.to_string(),
        year,
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

#[tokio::test]
async fn client_flow_setup_wizard_then_library_browse() {
    let (schema, jwt, _store) = authed_schema().await;

    // Fresh server reports unconfigured.
    let resp = schema.execute(r#"{ serverInfo { isConfigured } }"#).await;
    assert!(resp.errors.is_empty(), "serverInfo: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["serverInfo"]["isConfigured"],
        false
    );

    // Setup wizard mints the admin + a usable token.
    let resp = schema
        .execute(r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId accessToken refreshToken } }"#)
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let token = data["completeSetupWizard"]["accessToken"]
        .as_str()
        .expect("token");
    let claims = verify_jwt(&jwt, token).expect("wizard token verifies");
    assert_eq!(
        claims.sub,
        data["completeSetupWizard"]["userId"].as_str().unwrap()
    );
    assert!(claims.is_admin);
    let principal = Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub is a uuid")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(
            claims.sid.parse().expect("sid is a uuid"),
        )),
    };

    // Refresh round-trip: the wizard's refresh token exchanges for a
    // fresh pair on the same session.
    let refresh_token = data["completeSetupWizard"]["refreshToken"]
        .as_str()
        .expect("refresh token");
    let refresh = format!(
        r#"mutation {{ refreshSession(refreshToken: "{refresh_token}") {{ token refreshToken }} }}"#
    );
    let resp = schema.execute(refresh.as_str()).await;
    assert!(resp.errors.is_empty(), "refresh: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let new_claims = verify_jwt(
        &jwt,
        data["refreshSession"]["token"].as_str().expect("new token"),
    )
    .expect("refreshed token verifies");
    assert_eq!(new_claims.sub, claims.sub);
    assert_eq!(new_claims.sid, claims.sid);

    // Configured now.
    let resp = schema.execute(r#"{ serverInfo { isConfigured } }"#).await;
    assert_eq!(
        resp.data.into_json().unwrap()["serverInfo"]["isConfigured"],
        true
    );

    // Library lifecycle: create, browse, fetch by id, delete.
    let resp = schema
        .execute(authed_request(
            r#"mutation { createLibrary(name: "Films", kind: MOVIES, paths: ["/srv/films"]) { id name kind } }"#,
            principal,
        ))
        .await;
    assert!(resp.errors.is_empty(), "createLibrary: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let lib_id = data["createLibrary"]["id"].as_str().unwrap().to_string();

    let resp = schema
        .execute(authed_request(
            r#"{ libraries { id name kind } }"#,
            principal,
        ))
        .await;
    assert!(resp.errors.is_empty(), "libraries: {:?}", resp.errors);
    let libraries = resp.data.into_json().unwrap();
    let libraries = libraries["libraries"].as_array().unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0]["name"], "Films");

    let by_id = format!(r#"{{ library(id: "{lib_id}") {{ name kind }} }}"#);
    let resp = schema
        .execute(authed_request(by_id.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "library by id: {:?}", resp.errors);
    assert_eq!(resp.data.into_json().unwrap()["library"]["kind"], "MOVIES");

    // Catalog queries a browsing client hits on an empty library must
    // answer (empty), not error.
    for query in [
        r#"{ activity { nodes { kind } } }"#,
        r#"{ recentlyAdded { nodes { id } } }"#,
    ] {
        let resp = schema.execute(authed_request(query, principal)).await;
        assert!(resp.errors.is_empty(), "{query}: {:?}", resp.errors);
    }

    let del = format!(r#"mutation {{ deleteLibrary(id: "{lib_id}") {{ id }} }}"#);
    let resp = schema
        .execute(authed_request(del.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "deleteLibrary: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["deleteLibrary"].is_object());
}

#[tokio::test]
async fn admin_creates_user_who_can_log_in() {
    let (schema, _jwt, _store) = authed_schema().await;
    let (_user_id, admin) = run_setup_wizard(&schema).await;

    let create = r#"mutation { createUser(username: "bob", password: "pw1", role: USER) { id name role isAdmin } }"#;
    let resp = schema
        .execute(
            async_graphql::Request::new(create)
                .data(admin)
                .data(request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "createUser: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["createUser"]["name"], "bob");
    assert_eq!(data["createUser"]["isAdmin"], false);

    let resp = schema
        .execute(
            r#"mutation { login(username: "bob", password: "pw1") { token user { name role } } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "login as bob: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["login"]["user"]["name"], "bob");
    assert_eq!(data["login"]["user"]["role"], "USER");
}

#[tokio::test]
async fn images_query_exposes_serving_urls() {
    let (schema, _jwt, store) = authed_schema().await;
    let (_user_id, principal) = run_setup_wizard(&schema).await;

    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");
    let work = seed_work(lib.id, WorkKind::Movie, "Alien", Some(1979));
    store.upsert_work(&work).await.expect("work");

    let image = noflex_storage::domain::Image {
        target: noflex_storage::domain::ImageTarget::Work(work.id),
        kind: noflex_core::ImageKind::Primary,
        source: noflex_storage::domain::ImageSource::Local,
        path: "/cache/alien-primary.jpg".to_string(),
        width: Some(1920),
        height: Some(1080),
        language: None,
        provider: None,
    };
    store.upsert_image(&image).await.expect("image");

    let query = format!(
        r#"{{ images(targetKind: WORK, targetId: "{}") {{ kind url width }} }}"#,
        work.id
    );
    let resp = schema
        .execute(authed_request(query.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "images: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let items = data["images"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "PRIMARY");
    assert_eq!(items[0]["width"], 1920);
    let url = items[0]["url"].as_str().unwrap();
    assert!(
        url.starts_with(&format!("/images/work/{}/primary?source=local&v=", work.id)),
        "{url}"
    );
}

#[tokio::test]
async fn playback_reporting_flow() {
    let (schema, _jwt, store) = authed_schema().await;
    let (_user_id, principal) = run_setup_wizard(&schema).await;

    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");
    let work = seed_work(lib.id, WorkKind::Movie, "Alien", Some(1979));
    store.upsert_work(&work).await.expect("work");

    let target = format!(r#"{{ kind: WORK, id: "{}" }}"#, work.id);

    // Without a Principal the mutation must refuse the write.
    let report = format!(
        r#"mutation {{ reportProgress(target: {target}, positionMs: 42000, state: PLAYING) {{ positionMs }} }}"#
    );
    let resp = schema.execute(report.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "unauthenticated reportProgress: {:?}",
        resp.errors
    );

    // Authed progress write lands on continueWatching.
    let resp = schema
        .execute(authed_request(report.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "reportProgress: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["reportProgress"]["positionMs"],
        42000
    );

    let resume = r#"{ continueWatching { nodes { id title } } }"#;
    let resp = schema.execute(authed_request(resume, principal)).await;
    assert!(
        resp.errors.is_empty(),
        "continueWatching: {:?}",
        resp.errors
    );
    let data = resp.data.into_json().unwrap();
    let items = data["continueWatching"]["nodes"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], work.id.to_string());

    // markPlayed flips `played` and drops the row off continueWatching.
    let mark = format!(r#"mutation {{ markPlayed(target: {target}) {{ played }} }}"#);
    let resp = schema
        .execute(authed_request(mark.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "markPlayed: {:?}", resp.errors);
    assert_eq!(resp.data.into_json().unwrap()["markPlayed"]["played"], true);

    let resp = schema.execute(authed_request(resume, principal)).await;
    assert!(
        resp.data.into_json().unwrap()["continueWatching"]["nodes"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // markUnplayed clears the flag again.
    let unmark = format!(r#"mutation {{ markUnplayed(target: {target}) {{ played }} }}"#);
    let resp = schema
        .execute(authed_request(unmark.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "markUnplayed: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["markUnplayed"]["played"],
        false
    );

    // setFavorite round-trips through the userData query.
    let fav =
        format!(r#"mutation {{ setFavorite(target: {target}, favorite: true) {{ favorite }} }}"#);
    let resp = schema
        .execute(authed_request(fav.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "setFavorite: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["setFavorite"]["favorite"],
        true
    );

    let user_data = format!(r#"{{ userData(target: {target}) {{ played favorite positionMs }} }}"#);
    let resp = schema
        .execute(authed_request(user_data.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "userData: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["userData"]["favorite"], true);
    assert_eq!(data["userData"]["played"], false);
    assert_eq!(data["userData"]["positionMs"], 42000);
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
async fn browse_catalog_queries() {
    let (schema, _jwt, store) = authed_schema().await;
    let (_user_id, principal) = run_setup_wizard(&schema).await;
    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");

    // Stagger added_at so DateAddedDesc pagination is deterministic:
    // alien (newest), bladerunner, sopranos (oldest).
    let mut alien = seed_work(lib.id, WorkKind::Movie, "Alien", Some(1979));
    let mut bladerunner = seed_work(lib.id, WorkKind::Movie, "Blade Runner", Some(1982));
    let mut sopranos = seed_work(lib.id, WorkKind::Series, "The Sopranos", Some(1999));
    let now = OffsetDateTime::now_utc();
    alien.added_at = now;
    bladerunner.added_at = now - time::Duration::hours(1);
    sopranos.added_at = now - time::Duration::hours(2);
    for w in [&alien, &bladerunner, &sopranos] {
        store.upsert_work(w).await.expect("work");
    }
    let edition = seed_edition(alien.id);
    store.upsert_edition(&edition).await.expect("edition");
    let media = noflex_storage::domain::MediaFile {
        id: noflex_core::MediaFileId::new(),
        edition_id: edition.id,
        path: "/srv/films/alien.mkv".to_string(),
        part_index: 0,
        size_bytes: 4_500_000_000,
        mtime: now,
        container: Some("matroska".to_string()),
        probe: serde_json::json!({"format": "matroska"}).into(),
        subtitle_tracks: Vec::new(),
    };
    store.upsert_media_file(&media).await.expect("media");

    // Kind filter: only the two movies.
    let q = format!(
        r#"{{ works(libraryId: "{}", filter: {{ kinds: [MOVIE] }}) {{ nodes {{ title }} pageInfo {{ hasNextPage endCursor }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "works kinds: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let titles: Vec<&str> = data["works"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Alien", "Blade Runner"]);

    // Year filter on top of the kind filter.
    let q = format!(
        r#"{{ works(libraryId: "{}", filter: {{ kinds: [MOVIE], yearMin: 1980 }}) {{ nodes {{ title }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "works year: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let titles: Vec<&str> = data["works"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Blade Runner"]);

    // Pagination: limit 2 over all three works, then follow nextCursor.
    let q = format!(
        r#"{{ works(libraryId: "{}", first: 2) {{ nodes {{ title }} pageInfo {{ hasNextPage endCursor }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "works page 1: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let page1: Vec<&str> = data["works"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(page1, ["Alien", "Blade Runner"]);
    assert_eq!(data["works"]["pageInfo"]["hasNextPage"], true);
    let cursor = data["works"]["pageInfo"]["endCursor"]
        .as_str()
        .expect("page 1 has an end cursor");

    let q = format!(
        r#"{{ works(libraryId: "{}", first: 2, after: "{cursor}") {{ nodes {{ title }} pageInfo {{ hasNextPage endCursor }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "works page 2: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let page2: Vec<&str> = data["works"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(page2, ["The Sopranos"]);
    assert_eq!(data["works"]["pageInfo"]["hasNextPage"], false);

    // Single-work lookup; unknown id is null, not an error.
    let q = format!(r#"{{ work(id: "{}") {{ title year }} }}"#, alien.id);
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "work: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["work"]["title"], "Alien");
    assert_eq!(data["work"]["year"], 1979);

    let q = format!(r#"{{ work(id: "{}") {{ title }} }}"#, WorkId::new());
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "work missing: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["work"].is_null());

    // Editions and their media files.
    let q = format!(
        r#"{{ editions(workId: "{}") {{ id name runtimeMs }} }}"#,
        alien.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "editions: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let editions = data["editions"].as_array().unwrap();
    assert_eq!(editions.len(), 1);
    assert_eq!(editions[0]["id"], edition.id.to_string());
    assert_eq!(editions[0]["runtimeMs"], 7_020_000_i64);

    let q = format!(
        r#"{{ mediaFiles(editionId: "{}") {{ id path sizeBytes }} }}"#,
        edition.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "mediaFiles: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let files = data["mediaFiles"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "/srv/films/alien.mkv");
}

#[tokio::test]
async fn browse_filters_watched_rating_and_title_cursor() {
    let (schema, _jwt, store) = authed_schema().await;
    let (user_id, principal) = run_setup_wizard(&schema).await;

    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");

    let alien = seed_work(lib.id, WorkKind::Movie, "alien", Some(1979));
    let bladerunner = seed_work(lib.id, WorkKind::Movie, "Blade Runner", Some(1982));
    let starman = seed_work(lib.id, WorkKind::Movie, "Starman", Some(1984));
    for (w, value) in [(&alien, 8.5), (&bladerunner, 8.1), (&starman, 6.9)] {
        store.upsert_work(w).await.expect("work");
        let imdb = noflex_core::providers::Rating {
            source: "imdb".into(),
            value,
            scale: 10.0,
            votes: None,
        };
        store
            .replace_work_ratings(w.id, "omdb", &[imdb])
            .await
            .expect("rating");
    }

    // markPlayed makes `alien` the only watched work for this user.
    let target = format!(r#"{{ kind: WORK, id: "{}" }}"#, alien.id);
    let mark = format!(r#"mutation {{ markPlayed(target: {target}) {{ played }} }}"#);
    let resp = schema
        .execute(authed_request(mark.as_str(), principal))
        .await;
    assert!(resp.errors.is_empty(), "markPlayed: {:?}", resp.errors);

    let titles_of = |data: serde_json::Value| {
        let mut titles: Vec<String> = data["works"]["nodes"]
            .as_array()
            .expect("items")
            .iter()
            .map(|w| w["title"].as_str().expect("title").to_string())
            .collect();
        titles.sort();
        titles
    };

    // watched defaults to true when the flag is omitted.
    let q = format!(
        r#"{{ works(libraryId: "{}", filter: {{ watched: {{ userId: "{user_id}" }} }}) {{ nodes {{ title }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "watched: {:?}", resp.errors);
    assert_eq!(titles_of(resp.data.into_json().unwrap()), ["alien"]);

    // watched: false returns everything not completed.
    let q = format!(
        r#"{{ works(libraryId: "{}", filter: {{ watched: {{ userId: "{user_id}", watched: false }} }}) {{ nodes {{ title }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "unwatched: {:?}", resp.errors);
    assert_eq!(
        titles_of(resp.data.into_json().unwrap()),
        ["Blade Runner", "Starman"]
    );

    // ratingMin keeps only works at or above the bound.
    let q = format!(
        r#"{{ works(libraryId: "{}", filter: {{ ratingMin: 8.0 }}) {{ nodes {{ title }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "ratingMin: {:?}", resp.errors);
    assert_eq!(
        titles_of(resp.data.into_json().unwrap()),
        ["Blade Runner", "alien"]
    );

    // Title-sorted cursor round-trip: two pages, distinct and ordered.
    let q = format!(
        r#"{{ works(libraryId: "{}", sort: TITLE_ASC, first: 2) {{ nodes {{ title }} pageInfo {{ hasNextPage endCursor }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "title page 1: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let page1: Vec<&str> = data["works"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(page1, ["alien", "Blade Runner"]);
    assert_eq!(data["works"]["pageInfo"]["hasNextPage"], true);
    let cursor = data["works"]["pageInfo"]["endCursor"]
        .as_str()
        .expect("page 1 has an end cursor");

    let q = format!(
        r#"{{ works(libraryId: "{}", sort: TITLE_ASC, first: 2, after: "{cursor}") {{ nodes {{ title }} pageInfo {{ hasNextPage endCursor }} }} }}"#,
        lib.id
    );
    let resp = schema.execute(authed_request(q.as_str(), principal)).await;
    assert!(resp.errors.is_empty(), "title page 2: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let page2: Vec<&str> = data["works"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap())
        .collect();
    assert_eq!(page2, ["Starman"]);
    assert_eq!(data["works"]["pageInfo"]["hasNextPage"], false);
}

#[tokio::test]
async fn browse_filters_cannot_read_another_users_state() {
    let (schema, _jwt, store) = authed_schema().await;
    let (_admin_id, admin) = run_setup_wizard(&schema).await;
    let viewer = User {
        id: UserId::new(),
        username: "viewer".to_string(),
        display_name: None,
        password_hash: "unused".to_string(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    };
    store.upsert_user(&viewer).await.expect("viewer");
    let library = seed_library();
    store.upsert_library(&library).await.expect("library");
    store
        .grant_library_access(viewer.id, library.id)
        .await
        .expect("grant");

    let query = format!(
        r#"{{ works(libraryId: "{}", filter: {{ favoriteFor: "{}", watched: {{ userId: "{}" }} }}) {{ nodes {{ id }} }} }}"#,
        library.id, admin.user_id, admin.user_id
    );
    let viewer_principal = Principal {
        user_id: viewer.id,
        is_admin: false,
        session_id: None,
    };
    let response = schema
        .execute(authed_request(query, viewer_principal))
        .await;
    assert!(
        response
            .errors
            .iter()
            .any(|error| error.message == "not found"),
        "cross-user filters: {:?}",
        response.errors
    );
}

#[tokio::test]
async fn viewer_reflects_logged_in_user() {
    let (schema, jwt, store) = authed_schema().await;
    run_setup_wizard(&schema).await;

    // Log in fresh and build the Principal the HTTP layer would
    // attach after verifying the login token.
    let resp = schema
        .execute(
            r#"mutation { login(username: "admin", password: "hunter2") { token user { id } } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let login = resp.data.into_json().unwrap();
    let claims = verify_jwt(&jwt, login["login"]["token"].as_str().expect("token"))
        .expect("login token verifies");
    let principal = Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub is a uuid")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(
            claims.sid.parse().expect("sid is a uuid"),
        )),
    };

    // Anonymous viewers are refused.
    let resp = schema.execute(r#"{ viewer { id } }"#).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous viewer: {:?}",
        resp.errors
    );

    // A granted library shows up on the viewer's self-serve grant list.
    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");
    store
        .grant_library_access(principal.user_id, lib.id)
        .await
        .expect("grant");

    let resp = schema
        .execute(
            async_graphql::Request::new(r#"{ viewer { id name role libraryAccess } }"#)
                .data(principal)
                .data(request_user(principal)),
        )
        .await;
    assert!(resp.errors.is_empty(), "viewer: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let viewer = &data["viewer"];
    assert_eq!(viewer["id"], login["login"]["user"]["id"]);
    assert_eq!(viewer["name"], "admin");
    assert_eq!(viewer["role"], "ADMIN");
    let access = viewer["libraryAccess"].as_array().unwrap();
    assert_eq!(access.len(), 1);
    assert_eq!(access[0], lib.id.to_string());
}

#[tokio::test]
async fn device_code_link_flow() {
    let (schema, jwt, _store) = authed_schema().await;
    let (_user_id, principal) = run_setup_wizard(&schema).await;

    // An unauthenticated device asks for a code to show on screen.
    let resp = schema
        .execute(r#"mutation { createDeviceCode(client: "noflex-tv", device: "Living Room TV") { code expiresAt } }"#)
        .await;
    assert!(
        resp.errors.is_empty(),
        "createDeviceCode: {:?}",
        resp.errors
    );
    let data = resp.data.into_json().unwrap();
    let code = data["createDeviceCode"]["code"]
        .as_str()
        .expect("code")
        .to_string();
    assert_eq!(code.len(), 6);
    assert!(data["createDeviceCode"]["expiresAt"].as_str().is_some());

    let poll = format!(
        r#"mutation {{ pollDeviceCode(code: "{code}") {{ token refreshToken user {{ name }} }} }}"#
    );

    // Pending: poll answers null, not an error.
    let resp = schema.execute(poll.as_str()).await;
    assert!(resp.errors.is_empty(), "poll pending: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["pollDeviceCode"].is_null());

    // Authorizing needs a signed-in user.
    let authorize = format!(r#"mutation {{ authorizeDeviceCode(code: "{code}") }}"#);
    let resp = schema.execute(authorize.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous authorize: {:?}",
        resp.errors
    );
    let resp = schema
        .execute(
            async_graphql::Request::new(authorize.as_str())
                .data(principal)
                .data(request_user(principal)),
        )
        .await;
    assert!(resp.errors.is_empty(), "authorize: {:?}", resp.errors);
    assert_eq!(resp.data.into_json().unwrap()["authorizeDeviceCode"], true);

    // The first poll after authorization trades the code for a session.
    let resp = schema.execute(poll.as_str()).await;
    assert!(resp.errors.is_empty(), "poll authorized: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let token = data["pollDeviceCode"]["token"].as_str().expect("token");
    let claims = verify_jwt(&jwt, token).expect("code token verifies");
    assert_eq!(claims.sub, principal.user_id.to_string());
    assert_eq!(data["pollDeviceCode"]["user"]["name"], "admin");

    // The code is consumed: a second poll is null again.
    let resp = schema.execute(poll.as_str()).await;
    assert!(resp.errors.is_empty(), "poll consumed: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["pollDeviceCode"].is_null());

    // Unknown codes fail soft on both halves of the flow.
    let resp = schema
        .execute(r#"mutation { pollDeviceCode(code: "ZZZZZZ") { token } }"#)
        .await;
    assert!(resp.errors.is_empty(), "poll unknown: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["pollDeviceCode"].is_null());
    let resp = schema
        .execute(
            async_graphql::Request::new(r#"mutation { authorizeDeviceCode(code: "ZZZZZZ") }"#)
                .data(principal)
                .data(request_user(principal)),
        )
        .await;
    assert!(
        resp.errors.is_empty(),
        "authorize unknown: {:?}",
        resp.errors
    );
    assert_eq!(resp.data.into_json().unwrap()["authorizeDeviceCode"], false);
}

/// A child's TV has nobody signed in to approve it from, so an administrator
/// approves its code as them. Nobody else can sign a device in as another user.
#[tokio::test]
async fn an_administrator_approves_a_code_for_someone_else() {
    let (schema, jwt, store) = authed_schema().await;
    let (_admin_id, admin) = run_setup_wizard(&schema).await;
    let create =
        r#"mutation { createUser(username: "kid", password: "hunter22", role: USER) { id } }"#;
    let resp = schema.execute(authed_request(create, admin)).await;
    assert!(resp.errors.is_empty(), "createUser: {:?}", resp.errors);
    let kid_id = resp.data.into_json().unwrap()["createUser"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    let kid = Principal {
        user_id: UserId::from_uuid(kid_id.parse().expect("uuid")),
        is_admin: false,
        session_id: None,
    };

    let new_code = || async {
        let resp = schema
            .execute(r#"mutation { createDeviceCode(client: "noflex-tv", device: "Playroom TV") { code } }"#)
            .await;
        resp.data.into_json().unwrap()["createDeviceCode"]["code"]
            .as_str()
            .expect("code")
            .to_string()
    };
    let approve_for = |code: &str, user: &str| {
        format!(r#"mutation {{ authorizeDeviceCode(code: "{code}", userId: "{user}") }}"#)
    };

    let code = new_code().await;
    let resp = schema
        .execute(authed_request(
            approve_for(&code, &admin.user_id.to_string()),
            kid,
        ))
        .await;
    assert_eq!(resp.errors[0].message, "administrator required");

    let resp = schema
        .execute(authed_request(approve_for(&code, &kid_id), admin))
        .await;
    assert!(resp.errors.is_empty(), "approve: {:?}", resp.errors);
    let resp = schema
        .execute(format!(
            r#"mutation {{ pollDeviceCode(code: "{code}") {{ token }} }}"#
        ))
        .await;
    let data = resp.data.into_json().unwrap();
    let claims = verify_jwt(
        &jwt,
        data["pollDeviceCode"]["token"].as_str().expect("token"),
    )
    .expect("token verifies");
    assert_eq!(
        claims.sub, kid_id,
        "the device is the child's, not the admin's"
    );

    let activity = store
        .list_activity(
            &[noflex_storage::domain::ActivityKind::DeviceApprovedFor],
            None,
            10,
            None,
        )
        .await
        .expect("activity");
    assert_eq!(activity.items.len(), 1);
    let entry = &activity.items[0];
    assert_eq!(
        entry.actor.as_deref(),
        Some(admin.user_id.to_string().as_str())
    );
    assert_eq!(entry.subject.as_deref(), Some(kid_id.as_str()));
}

#[tokio::test]
async fn api_key_create_list_revoke() {
    let (schema, _jwt, _store) = authed_schema().await;
    let (_user_id, admin) = run_setup_wizard(&schema).await;

    let create = r#"mutation { createApiKey(name: "cli") { key apiKey { id name revokedAt } } }"#;
    let resp = schema.execute(create).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous createApiKey: {:?}",
        resp.errors
    );

    let resp = schema
        .execute(
            async_graphql::Request::new(create)
                .data(admin)
                .data(request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "createApiKey: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let key = data["createApiKey"]["key"].as_str().expect("key");
    assert!(key.starts_with("nfk_"), "key prefix: {key}");
    assert_eq!(key.len(), 4 + 64);
    let key_id = data["createApiKey"]["apiKey"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    assert_eq!(data["createApiKey"]["apiKey"]["name"], "cli");
    assert!(data["createApiKey"]["apiKey"]["revokedAt"].is_null());

    // The self-scoped listing shows the row (and never the plaintext).
    let resp = schema
        .execute(
            async_graphql::Request::new(r#"{ apiKeys { id name revokedAt } }"#)
                .data(admin)
                .data(request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "apiKeys: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let keys = data["apiKeys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["id"], key_id);

    // A non-admin gets a uniform "not found" for someone else's key.
    let resp = schema
        .execute(
            async_graphql::Request::new(
                r#"mutation { createUser(username: "bob", password: "pw1", role: USER) { id } }"#,
            )
            .data(admin)
            .data(request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "createUser: {:?}", resp.errors);
    let bob_id = resp.data.into_json().unwrap()["createUser"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = Principal {
        user_id: UserId::from_uuid(bob_id.parse().expect("bob id")),
        is_admin: false,
        session_id: None,
    };
    let revoke = format!(r#"mutation {{ revokeApiKey(id: "{key_id}") {{ revokedAt }} }}"#);
    let resp = schema
        .execute(
            async_graphql::Request::new(revoke.as_str())
                .data(bob)
                .data(request_user(bob)),
        )
        .await;
    assert!(
        resp.errors.iter().any(|e| e.message == "api key not found"),
        "bob revoke: {:?}",
        resp.errors
    );

    // The owner revokes; a replay reports null.
    let resp = schema
        .execute(
            async_graphql::Request::new(revoke.as_str())
                .data(admin)
                .data(request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "revoke: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["revokeApiKey"]["revokedAt"].is_string());
    let resp = schema
        .execute(
            async_graphql::Request::new(revoke.as_str())
                .data(admin)
                .data(request_user(admin)),
        )
        .await;
    assert!(resp.data.into_json().unwrap()["revokeApiKey"].is_null());

    // The revoked row stays listed, now stamped.
    let resp = schema
        .execute(
            async_graphql::Request::new(r#"{ apiKeys { id revokedAt } }"#)
                .data(admin)
                .data(request_user(admin)),
        )
        .await;
    let data = resp.data.into_json().unwrap();
    let keys = data["apiKeys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys[0]["revokedAt"].is_string());
}

struct StubPlaybackSessions;

fn stub_plan() -> noflex_transcoder::TranscodePlan {
    noflex_transcoder::TranscodePlan {
        segment_container: noflex_transcoder::HlsSegmentContainer::Fmp4,
        video: noflex_transcoder::VideoPlan::None,
        audio: noflex_transcoder::AudioPlan::None,
    }
}

#[async_trait::async_trait]
impl PlaybackSessionService for StubPlaybackSessions {
    async fn start(
        &self,
        _user: noflex_core::UserId,
        _login: Option<noflex_core::SessionId>,
        _request: noflex_graphql::PlaybackStart,
    ) -> anyhow::Result<PlaybackSession> {
        Ok(PlaybackSession {
            session_id: async_graphql::ID("job-1".to_string()),
            progress_token: None,
            route: PlaybackRouteKind::Transcode,
            url: "/stream/job/job-1/master.m3u8".to_string(),
            file_url: None,
            job_id: Some(async_graphql::ID("job-1".to_string())),
            start_position_ms: 0,
            decision: noflex_transcoder::PlaybackDecisionInfo::live(
                &stub_plan(),
                noflex_transcoder::Encoder::Software264,
                None,
            ),
            subtitles: Vec::new(),
            qualities: Vec::new(),
            adaptive: false,
        })
    }

    async fn stop(&self, _user: noflex_core::UserId, session_id: &str) -> anyhow::Result<()> {
        if session_id == "job-1" {
            Ok(())
        } else {
            Err(anyhow::anyhow!("unknown job"))
        }
    }

    async fn active_streams(&self) -> anyhow::Result<Vec<noflex_graphql::ActiveStreamRecord>> {
        Ok(Vec::new())
    }
}

const CAPS_INPUT: &str = r#"{ directPlay: [{ container: "mp4", videoCodecs: [H264], audioCodecs: [AAC] }], hls: [{ segmentContainer: FMP4, videoCodecs: [H264], audioCodecs: [AAC] }], videoCodecs: [{ codec: H264, profiles: [H264_HIGH], maxLevel: 51, maxBitDepth: 8, videoRanges: [SDR] }], audioCodecs: [{ codec: AAC, maxChannels: 6 }], subtitleCodecs: [WEB_VTT] }"#;

#[tokio::test]
async fn playback_session_start_stop() {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(store),
        jwt: Some(jwt),
        playback_sessions: Some(Arc::new(StubPlaybackSessions)),
        ..Default::default()
    });
    let (_user_id, principal) = run_setup_wizard(&schema).await;

    let media_id = noflex_core::MediaFileId::new();
    let start = format!(
        r#"mutation {{ startPlaybackSession(mediaFileId: "{media_id}", capabilities: {CAPS_INPUT}, burnSubtitle: 2) {{ sessionId route url jobId decision {{ route plan reasons {{ code description }} }} }} }}"#
    );

    // Anonymous refused on both mutations.
    let resp = schema.execute(start.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous start: {:?}",
        resp.errors
    );

    let resp = schema
        .execute(
            async_graphql::Request::new(start.as_str())
                .data(principal)
                .data(request_user(principal)),
        )
        .await;
    assert!(resp.errors.is_empty(), "start: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let session = &data["startPlaybackSession"];
    assert_eq!(session["route"], "TRANSCODE");
    assert_eq!(session["url"], "/stream/job/job-1/master.m3u8");
    assert_eq!(session["jobId"], "job-1");
    assert_eq!(session["sessionId"], "job-1");
    assert_eq!(session["decision"]["plan"], "REMUX");
    assert_eq!(session["decision"]["reasons"][0]["code"], "LIVE_SOURCE");

    let stop = r#"mutation { stopPlaybackSession(sessionId: "job-1") }"#;
    let resp = schema.execute(stop).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous stop: {:?}",
        resp.errors
    );
    let resp = schema
        .execute(
            async_graphql::Request::new(stop)
                .data(principal)
                .data(request_user(principal)),
        )
        .await;
    assert!(resp.errors.is_empty(), "stop: {:?}", resp.errors);
    assert_eq!(resp.data.into_json().unwrap()["stopPlaybackSession"], true);

    // Service failures surface as GraphQL errors.
    let resp = schema
        .execute(
            async_graphql::Request::new(r#"mutation { stopPlaybackSession(sessionId: "nope") }"#)
                .data(principal)
                .data(request_user(principal)),
        )
        .await;
    assert!(!resp.errors.is_empty(), "stop unknown job must error");
}

#[tokio::test]
async fn playback_session_unavailable_without_service() {
    let (schema, _jwt, _store) = authed_schema().await;
    let (_user_id, principal) = run_setup_wizard(&schema).await;
    let media_id = noflex_core::MediaFileId::new();
    let start = format!(
        r#"mutation {{ startPlaybackSession(mediaFileId: "{media_id}", capabilities: {CAPS_INPUT}) {{ route }} }}"#
    );
    let resp = schema
        .execute(
            async_graphql::Request::new(start.as_str())
                .data(principal)
                .data(request_user(principal)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "playback sessions unavailable"),
        "no service: {:?}",
        resp.errors
    );
}
