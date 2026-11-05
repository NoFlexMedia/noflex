// SPDX-License-Identifier: AGPL-3.0-only
//
// E6/E7 acceptance coverage for catalog visibility, per-user restrictions,
// playback authorization, and watch-room membership/control boundaries.

use std::{collections::HashSet, sync::Arc, time::Duration};

use async_graphql::{Request, Response};
use futures::StreamExt;
use noflex_auth::Principal;
use noflex_core::{
    EditionId, ImageKind, LibraryId, MediaFileId, TagId, TagTarget, UserId, WatchRoomId, WorkId,
    WorkKind,
};
use noflex_graphql::{NoflexSchema, SchemaContext, WatchRoomBus, build_schema};
use noflex_storage::{
    RequestUser, Storage,
    domain::{
        Edition, Image, ImageTarget, Library, LibraryConfig, LibraryKind, MediaFile, Tag, User,
        UserRestrictions, WatchRoomState, Work,
    },
};
use noflex_storage_turso::TursoStore;
use serde_json::Value;
use time::OffsetDateTime;

mod fixture {
    use super::*;

    pub fn principal(user: &User) -> Principal {
        Principal {
            user_id: user.id,
            is_admin: user.is_admin,
            session_id: None,
        }
    }

    pub fn request(query: impl Into<String>, principal: Principal) -> Request {
        Request::new(query).data(principal).data(RequestUser {
            user_id: principal.user_id,
            is_admin: principal.is_admin,
        })
    }

    pub fn user(name: &str, is_admin: bool) -> User {
        User {
            id: UserId::new(),
            name: name.to_string(),
            password_hash: "argon2id$placeholder".to_string(),
            is_admin,
            role: if is_admin {
                noflex_core::UserRole::Admin
            } else {
                noflex_core::UserRole::User
            },
            created_at: OffsetDateTime::now_utc(),
        }
    }

    pub fn library(name: &str) -> Library {
        Library {
            id: LibraryId::new(),
            name: name.to_string(),
            kind: LibraryKind::Movies,
            paths: vec![format!("/srv/{}", name.to_lowercase())],
            config: LibraryConfig::default(),
        }
    }

    pub fn work(library_id: LibraryId, title: &str) -> Work {
        let now = OffsetDateTime::now_utc();
        Work {
            id: WorkId::new(),
            kind: WorkKind::Movie,
            parent_id: None,
            library_id,
            title: title.to_string(),
            sort_title: title.to_lowercase(),
            year: None,
            release_date: None,
            overview: Some(format!("{title} overview")),
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

    pub fn edition(work_id: WorkId) -> Edition {
        Edition {
            id: EditionId::new(),
            work_id,
            name: "Default".to_string(),
            runtime_ms: Some(7_020_000),
            intro_start_ms: None,
            intro_end_ms: None,
            credits_start_ms: None,
            chapters: vec![],
        }
    }

    pub fn media(edition_id: EditionId, path: &str) -> MediaFile {
        MediaFile {
            id: MediaFileId::new(),
            edition_id,
            path: path.to_string(),
            part_index: 0,
            size_bytes: 1024,
            mtime: OffsetDateTime::now_utc(),
            container: Some("matroska".to_string()),
            probe: serde_json::json!({}).into(),
            subtitle_tracks: Vec::new(),
        }
    }

    pub fn image(target: ImageTarget, kind: ImageKind, path: &str) -> Image {
        Image {
            target,
            kind,
            source: noflex_storage::domain::ImageSource::Local,
            path: path.to_string(),
            width: Some(1920),
            height: Some(1080),
            language: Some("en".to_string()),
        }
    }

    pub fn parental_tag(slug: &str, display: &str) -> Tag {
        Tag {
            id: TagId::new(),
            namespace: "parental".to_string(),
            parent_id: None,
            slug: slug.to_string(),
            display: display.to_string(),
            metadata: async_graphql::Json(serde_json::json!({})),
        }
    }

    pub struct Catalog {
        pub schema: NoflexSchema,
        pub store: Arc<TursoStore>,
        pub admin: Principal,
        pub user: Principal,
        pub library: Library,
        pub work: Work,
        pub edition: Edition,
        pub media: MediaFile,
    }

    pub async fn catalog() -> Catalog {
        let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
        store.migrate().await.expect("migrate");
        let schema = build_schema(SchemaContext::new(store.clone()));

        let admin = user("catalog-admin", true);
        let normal = user("catalog-user", false);
        store.upsert_user(&admin).await.expect("admin");
        store.upsert_user(&normal).await.expect("normal user");

        let library = library("Catalog");
        store.upsert_library(&library).await.expect("library");
        let work = work(library.id, "Catalog Movie");
        store.upsert_work(&work).await.expect("work");
        let edition = edition(work.id);
        store.upsert_edition(&edition).await.expect("edition");
        let media = media(edition.id, "/srv/catalog/movie.mkv");
        store.upsert_media_file(&media).await.expect("media");
        store
            .upsert_image(&image(
                ImageTarget::Work(work.id),
                ImageKind::Primary,
                "/cache/catalog-primary.jpg",
            ))
            .await
            .expect("work image");
        store
            .upsert_image(&image(
                ImageTarget::Edition(edition.id),
                ImageKind::Backdrop,
                "/cache/catalog-backdrop.jpg",
            ))
            .await
            .expect("edition image");

        Catalog {
            schema,
            store,
            admin: principal(&admin),
            user: principal(&normal),
            library,
            work,
            edition,
            media,
        }
    }

    pub struct Restrictions {
        pub schema: NoflexSchema,
        pub store: Arc<TursoStore>,
        pub admin: Principal,
        pub user: Principal,
        pub library: Library,
        pub allowed_rating: Tag,
        pub allowed: Vec<Work>,
        pub blocked: Vec<Work>,
        pub unrated: Work,
    }

    pub async fn restrictions() -> Restrictions {
        let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
        store.migrate().await.expect("migrate");
        let schema = build_schema(SchemaContext::new(store.clone()));

        let admin = user("restriction-admin", true);
        let normal = user("restriction-user", false);
        store.upsert_user(&admin).await.expect("admin");
        store.upsert_user(&normal).await.expect("normal user");

        let library = library("Restrictions");
        store.upsert_library(&library).await.expect("library");
        store
            .grant_library_access(normal.id, library.id)
            .await
            .expect("library grant");

        let allowed_rating = parental_tag("pg", "PG");
        let blocked_rating = parental_tag("r", "R");
        store
            .upsert_tag(&allowed_rating)
            .await
            .expect("allowed rating");
        store
            .upsert_tag(&blocked_rating)
            .await
            .expect("blocked rating");

        let mut allowed = Vec::new();
        let mut blocked = Vec::new();
        for (title, rating, destination) in [
            ("Catalog A", allowed_rating.id, true),
            ("Catalog B", blocked_rating.id, false),
            ("Catalog C", allowed_rating.id, true),
            ("Catalog D", blocked_rating.id, false),
            ("Catalog E", allowed_rating.id, true),
        ] {
            let work = work(library.id, title);
            store.upsert_work(&work).await.expect("work");
            store
                .assign_tag(rating, TagTarget::Work(work.id), 1.0, "acceptance-test")
                .await
                .expect("rating assignment");
            if destination {
                allowed.push(work);
            } else {
                blocked.push(work);
            }
        }
        let unrated = work(library.id, "Catalog F");
        store.upsert_work(&unrated).await.expect("unrated work");
        store
            .set_user_restrictions(&UserRestrictions {
                user_id: normal.id,
                allowed_parental_rating_ids: vec![allowed_rating.id],
                allow_unrated: false,
                max_stream_bitrate_bps: None,
                max_concurrent_transcodes: None,
            })
            .await
            .expect("restriction policy");

        Restrictions {
            schema,
            store,
            admin: principal(&admin),
            user: principal(&normal),
            library,
            allowed_rating,
            allowed,
            blocked,
            unrated,
        }
    }

    pub struct Watch {
        pub schema: NoflexSchema,
        pub store: Arc<TursoStore>,
        pub host: Principal,
        pub member: Principal,
        pub outsider: Principal,
        pub library: Library,
        pub media: MediaFile,
    }

    pub async fn watch() -> Watch {
        let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
        store.migrate().await.expect("migrate");
        let bus = Arc::new(WatchRoomBus::default());
        let schema = build_schema(SchemaContext {
            storage: Some(store.clone()),
            watch_room_bus: Some(bus),
            ..Default::default()
        });

        let host = user("watch-host", false);
        let member = user("watch-member", false);
        let outsider = user("watch-outsider", false);
        for user in [&host, &member, &outsider] {
            store.upsert_user(user).await.expect("watch user");
        }
        let library = library("Watch");
        store.upsert_library(&library).await.expect("watch library");
        let work = work(library.id, "Watch Movie");
        store.upsert_work(&work).await.expect("watch work");
        let edition = edition(work.id);
        store.upsert_edition(&edition).await.expect("watch edition");
        let media = media(edition.id, "/srv/watch/movie.mkv");
        store.upsert_media_file(&media).await.expect("watch media");

        Watch {
            schema,
            store,
            host: principal(&host),
            member: principal(&member),
            outsider: principal(&outsider),
            library,
            media,
        }
    }
}

fn json(response: Response, label: &str) -> Value {
    assert!(response.errors.is_empty(), "{label}: {:?}", response.errors);
    response.data.into_json().expect("GraphQL JSON")
}

fn assert_error(response: &Response, label: &str, expected: &str) {
    assert!(
        !response.errors.is_empty(),
        "{label} unexpectedly succeeded"
    );
    assert!(
        response
            .errors
            .iter()
            .any(|error| error.message.contains(expected)),
        "{label} errors {:?} do not contain {expected:?}",
        response.errors
    );
}

async fn expect_subscription_error(
    schema: &NoflexSchema,
    room_id: &str,
    principal: Principal,
    label: &str,
) {
    let query = format!(
        r#"subscription {{ watchRoomEvents(roomId: "{room_id}") {{ room {{ id }} action sequence }} }}"#
    );
    let mut stream = schema.execute_stream(fixture::request(query, principal));
    let response = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("subscription error response timed out")
        .expect("subscription ended without an error response");
    assert_error(&response, label, "watch room not found");
}

async fn assert_catalog_absent(fixture: &fixture::Catalog, principal: Principal) {
    let unknown_library = LibraryId::new();
    let unknown_work = WorkId::new();
    let unknown_edition = EditionId::new();

    let data = json(
        fixture
            .schema
            .execute(fixture::request("{ libraries { id } }", principal))
            .await,
        "libraries absent",
    );
    assert!(data["libraries"].as_array().expect("libraries").is_empty());

    for (library_id, label) in [
        (fixture.library.id.to_string(), "forbidden library"),
        (unknown_library.to_string(), "unknown library"),
    ] {
        let query = format!(r#"{{ library(id: "{library_id}") {{ id }} }}"#);
        let data = json(
            fixture
                .schema
                .execute(fixture::request(query, principal))
                .await,
            label,
        );
        assert!(data["library"].is_null(), "{label} leaked an object");
    }

    let query = format!(
        r#"{{ works(libraryId: "{}", first: 10) {{ nodes {{ id }} }} }}"#,
        fixture.library.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "works absent",
    );
    assert!(
        data["works"]["nodes"]
            .as_array()
            .expect("works items")
            .is_empty()
    );

    for (id, label) in [
        (fixture.work.id.to_string(), "forbidden work"),
        (unknown_work.to_string(), "unknown work"),
    ] {
        let query = format!(r#"{{ work(id: "{id}") {{ id }} }}"#);
        let data = json(
            fixture
                .schema
                .execute(fixture::request(query, principal))
                .await,
            label,
        );
        assert!(data["work"].is_null(), "{label} leaked an object");
    }

    for (work_id, label) in [
        (fixture.work.id.to_string(), "forbidden editions"),
        (unknown_work.to_string(), "unknown editions"),
    ] {
        let query = format!(r#"{{ editions(workId: "{work_id}") {{ id }} }}"#);
        let data = json(
            fixture
                .schema
                .execute(fixture::request(query, principal))
                .await,
            label,
        );
        assert!(data["editions"].as_array().expect("editions").is_empty());
    }

    for (edition_id, label) in [
        (fixture.edition.id.to_string(), "forbidden media files"),
        (unknown_edition.to_string(), "unknown media files"),
    ] {
        let query = format!(r#"{{ mediaFiles(editionId: "{edition_id}") {{ id }} }}"#);
        let data = json(
            fixture
                .schema
                .execute(fixture::request(query, principal))
                .await,
            label,
        );
        assert!(
            data["mediaFiles"]
                .as_array()
                .expect("media files")
                .is_empty()
        );
    }

    for (kind, id, label) in [
        ("WORK", fixture.work.id.to_string(), "forbidden work images"),
        (
            "EDITION",
            fixture.edition.id.to_string(),
            "forbidden edition images",
        ),
        ("WORK", unknown_work.to_string(), "unknown work images"),
        (
            "EDITION",
            unknown_edition.to_string(),
            "unknown edition images",
        ),
    ] {
        let query = format!(r#"{{ images(targetKind: {kind}, targetId: "{id}") {{ kind }} }}"#);
        let data = json(
            fixture
                .schema
                .execute(fixture::request(query, principal))
                .await,
            label,
        );
        assert!(data["images"].as_array().expect("images").is_empty());
    }

    let data = json(
        fixture
            .schema
            .execute(fixture::request(
                r#"{ search(text: "Catalog Movie", mode: LEXICAL) { nodes { workId } } }"#,
                principal,
            ))
            .await,
        "search absent",
    );
    assert!(
        data["search"]["nodes"]
            .as_array()
            .expect("search")
            .is_empty()
    );
}

async fn assert_catalog_present(fixture: &fixture::Catalog, principal: Principal) {
    let data = json(
        fixture
            .schema
            .execute(fixture::request("{ libraries { id } }", principal))
            .await,
        "libraries present",
    );
    assert_eq!(data["libraries"].as_array().expect("libraries").len(), 1);

    let query = format!(r#"{{ library(id: "{}") {{ id }} }}"#, fixture.library.id);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "library present",
    );
    assert_eq!(data["library"]["id"], fixture.library.id.to_string());

    let query = format!(
        r#"{{ works(libraryId: "{}") {{ nodes {{ id }} }} }}"#,
        fixture.library.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "works present",
    );
    assert_eq!(data["works"]["nodes"][0]["id"], fixture.work.id.to_string());

    let query = format!(r#"{{ work(id: "{}") {{ id title }} }}"#, fixture.work.id);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "work present",
    );
    assert_eq!(data["work"]["id"], fixture.work.id.to_string());

    let query = format!(r#"{{ editions(workId: "{}") {{ id }} }}"#, fixture.work.id);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "editions present",
    );
    assert_eq!(data["editions"][0]["id"], fixture.edition.id.to_string());

    let query = format!(
        r#"{{ mediaFiles(editionId: "{}") {{ id }} }}"#,
        fixture.edition.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "media files present",
    );
    assert_eq!(data["mediaFiles"][0]["id"], fixture.media.id.to_string());

    let query = format!(
        r#"{{ images(targetKind: WORK, targetId: "{}") {{ kind url }} }}"#,
        fixture.work.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "work images present",
    );
    assert_eq!(data["images"].as_array().expect("work images").len(), 1);

    let query = format!(
        r#"{{ images(targetKind: EDITION, targetId: "{}") {{ kind }} }}"#,
        fixture.edition.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, principal))
            .await,
        "edition images present",
    );
    assert_eq!(data["images"].as_array().expect("edition images").len(), 1);

    let data = json(
        fixture
            .schema
            .execute(fixture::request(
                r#"{ search(text: "Catalog Movie", mode: LEXICAL) { nodes { workId } } }"#,
                principal,
            ))
            .await,
        "search present",
    );
    assert_eq!(
        data["search"]["nodes"][0]["workId"],
        fixture.work.id.to_string()
    );
}

#[tokio::test]
async fn e6_catalog_requires_auth_and_honors_grants_for_every_target() {
    let fixture = fixture::catalog().await;

    let anonymous_queries = [
        "{ libraries { id } }".to_string(),
        format!(r#"{{ library(id: "{}") {{ id }} }}"#, fixture.library.id),
        format!(
            r#"{{ works(libraryId: "{}") {{ nodes {{ id }} }} }}"#,
            fixture.library.id
        ),
        format!(r#"{{ work(id: "{}") {{ id }} }}"#, fixture.work.id),
        format!(r#"{{ editions(workId: "{}") {{ id }} }}"#, fixture.work.id),
        format!(
            r#"{{ mediaFiles(editionId: "{}") {{ id }} }}"#,
            fixture.edition.id
        ),
        format!(
            r#"{{ images(targetKind: WORK, targetId: "{}") {{ kind }} }}"#,
            fixture.work.id
        ),
        r#"{ search(text: "Catalog Movie", mode: LEXICAL) { nodes { workId } } }"#.to_string(),
        "{ recentlyAdded { nodes { id } } }".to_string(),
        "{ tags(namespaces: []) { nodes { tag { id } } } }".to_string(),
    ];
    for query in anonymous_queries {
        let response = fixture.schema.execute(Request::new(query.clone())).await;
        assert_error(&response, "anonymous catalog", "authentication required");
    }

    // A normal user has no catalog surface before a library grant.
    assert_catalog_absent(&fixture, fixture.user).await;

    fixture
        .store
        .grant_library_access(fixture.user.user_id, fixture.library.id)
        .await
        .expect("grant library access");
    assert_catalog_present(&fixture, fixture.user).await;

    // Revocation removes every object and search path, not just the list.
    fixture
        .store
        .revoke_library_access(fixture.user.user_id, fixture.library.id)
        .await
        .expect("revoke library access");
    assert_catalog_absent(&fixture, fixture.user).await;

    for target in [
        format!(r#"{{ kind: WORK, id: "{}" }}"#, fixture.work.id),
        format!(r#"{{ kind: EDITION, id: "{}" }}"#, fixture.edition.id),
    ] {
        let query = format!(r#"{{ userData(target: {target}) {{ played }} }}"#);
        let response = fixture
            .schema
            .execute(fixture::request(query, fixture.user))
            .await;
        assert_error(&response, "revoked userData", "target not found");

        let mutation = format!(
            r#"mutation {{ reportProgress(target: {target}, positionMs: 1000, state: PLAYING) {{ positionMs }} }}"#
        );
        let response = fixture
            .schema
            .execute(fixture::request(mutation, fixture.user))
            .await;
        assert_error(&response, "revoked reportProgress", "target not found");
    }

    // Administrators bypass both library grants and per-user restrictions.
    assert_catalog_present(&fixture, fixture.admin).await;
}

#[tokio::test]
async fn e6_parental_policy_and_playback_authorization_are_uniform() {
    let fixture = fixture::restrictions().await;

    let query = format!(r#"{{ work(id: "{}") {{ id }} }}"#, fixture.allowed[0].id);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, fixture.user))
            .await,
        "allowed parental work",
    );
    assert_eq!(data["work"]["id"], fixture.allowed[0].id.to_string());

    for (work, label) in fixture
        .blocked
        .iter()
        .map(|work| (work, "disallowed parental work"))
        .chain(std::iter::once((&fixture.unrated, "unrated denied work")))
    {
        let query = format!(r#"{{ work(id: "{}") {{ id }} }}"#, work.id);
        let data = json(
            fixture
                .schema
                .execute(fixture::request(query, fixture.user))
                .await,
            label,
        );
        assert!(data["work"].is_null(), "{label} was visible");
    }

    let query = format!(
        r#"{{ works(libraryId: "{}", sort: TITLE_ASC) {{ nodes {{ id }} }} }}"#,
        fixture.library.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, fixture.user))
            .await,
        "restricted works",
    );
    let restricted_ids: HashSet<String> = data["works"]["nodes"]
        .as_array()
        .expect("restricted items")
        .iter()
        .map(|work| work["id"].as_str().expect("work id").to_string())
        .collect();
    let expected_allowed: HashSet<String> = fixture
        .allowed
        .iter()
        .map(|work| work.id.to_string())
        .collect();
    assert_eq!(restricted_ids, expected_allowed);

    let data = json(
        fixture
            .schema
            .execute(fixture::request(
                r#"{ search(text: "Catalog", mode: LEXICAL) { nodes { workId } } }"#,
                fixture.user,
            ))
            .await,
        "restricted search",
    );
    let search_ids: HashSet<String> = data["search"]["nodes"]
        .as_array()
        .expect("restricted search")
        .iter()
        .map(|hit| hit["workId"].as_str().expect("search id").to_string())
        .collect();
    assert_eq!(search_ids, expected_allowed);

    // An explicit allow for unrated works changes only the unrated branch.
    fixture
        .store
        .set_user_restrictions(&UserRestrictions {
            user_id: fixture.user.user_id,
            allowed_parental_rating_ids: vec![fixture.allowed_rating.id],
            allow_unrated: true,
            max_stream_bitrate_bps: None,
            max_concurrent_transcodes: None,
        })
        .await
        .expect("allow unrated policy");
    let query = format!(r#"{{ work(id: "{}") {{ id }} }}"#, fixture.unrated.id);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, fixture.user))
            .await,
        "unrated allowed work",
    );
    assert_eq!(data["work"]["id"], fixture.unrated.id.to_string());

    // Admin visibility is independent of the normal user's policy and grant.
    let query = format!(
        r#"{{ works(libraryId: "{}", sort: TITLE_ASC) {{ nodes {{ id }} }} }}"#,
        fixture.library.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, fixture.admin))
            .await,
        "admin restricted works",
    );
    assert_eq!(
        data["works"]["nodes"]
            .as_array()
            .expect("admin items")
            .len(),
        6
    );

    let target_work = format!(r#"{{ kind: WORK, id: "{}" }}"#, fixture.blocked[0].id);
    let target_edition = format!(r#"{{ kind: EDITION, id: "{}" }}"#, EditionId::new());
    for target in [target_work, target_edition] {
        let query = format!(r#"{{ userData(target: {target}) {{ played }} }}"#);
        let response = fixture
            .schema
            .execute(fixture::request(query, fixture.user))
            .await;
        assert_error(&response, "forbidden userData", "target not found");

        let mutation = format!(
            r#"mutation {{ reportProgress(target: {target}, positionMs: 1000, state: PLAYING) {{ positionMs }} }}"#
        );
        let response = fixture
            .schema
            .execute(fixture::request(mutation, fixture.user))
            .await;
        assert_error(&response, "forbidden reportProgress", "target not found");
    }
}

#[tokio::test]
async fn e6_filtered_works_cursor_pages_each_allowed_record_once() {
    let fixture = fixture::restrictions().await;
    let mut cursor: Option<String> = None;
    let mut seen = Vec::new();

    loop {
        let cursor_arg = cursor
            .as_deref()
            .map(|cursor| format!(r#", after: "{cursor}""#))
            .unwrap_or_default();
        let query = format!(
            r#"{{ works(libraryId: "{}", sort: TITLE_ASC, first: 2{}) {{ nodes {{ id }} pageInfo {{ hasNextPage endCursor }} }} }}"#,
            fixture.library.id, cursor_arg
        );
        let data = json(
            fixture
                .schema
                .execute(fixture::request(query, fixture.user))
                .await,
            "filtered works page",
        );
        for work in data["works"]["nodes"].as_array().expect("page items") {
            seen.push(work["id"].as_str().expect("page work id").to_string());
        }
        let page_info = &data["works"]["pageInfo"];
        cursor = (page_info["hasNextPage"] == true).then(|| {
            page_info["endCursor"]
                .as_str()
                .expect("end cursor")
                .to_owned()
        });
        if cursor.is_none() {
            break;
        }
    }

    let expected: HashSet<String> = fixture
        .allowed
        .iter()
        .map(|work| work.id.to_string())
        .collect();
    let unique: HashSet<String> = seen.iter().cloned().collect();
    assert_eq!(seen.len(), expected.len(), "all allowed rows were paged");
    assert_eq!(unique.len(), seen.len(), "cursor repeated an allowed row");
    assert_eq!(unique, expected, "cursor exposed or skipped a row");
    assert!(
        fixture
            .blocked
            .iter()
            .all(|work| !unique.contains(&work.id.to_string()))
    );
    assert!(!unique.contains(&fixture.unrated.id.to_string()));
}

async fn create_room(fixture: &fixture::Watch, principal: Principal) -> String {
    let mutation = format!(
        r#"mutation {{ createWatchRoom(mediaFileId: "{}") {{ id hostUserId mediaFileId state sequence }} }}"#,
        fixture.media.id
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(mutation, principal))
            .await,
        "create watch room",
    );
    data["createWatchRoom"]["id"]
        .as_str()
        .expect("watch room id")
        .to_string()
}

#[tokio::test]
async fn e7_watch_room_access_control_sequence_and_terminal_lifecycle() {
    let fixture = fixture::watch().await;

    let create = format!(
        r#"mutation {{ createWatchRoom(mediaFileId: "{}") {{ id }} }}"#,
        fixture.media.id
    );
    let response = fixture
        .schema
        .execute(fixture::request(create, fixture.host))
        .await;
    assert_error(
        &response,
        "create without target access",
        "media file not found",
    );

    fixture
        .store
        .grant_library_access(fixture.host.user_id, fixture.library.id)
        .await
        .expect("host library grant");
    let room_id = create_room(&fixture, fixture.host).await;

    let join = format!(r#"mutation {{ joinWatchRoom(roomId: "{room_id}") {{ id }} }}"#);
    let response = fixture
        .schema
        .execute(fixture::request(join.clone(), fixture.member))
        .await;
    assert_error(
        &response,
        "join without target access",
        "watch room not found",
    );

    fixture
        .store
        .grant_library_access(fixture.member.user_id, fixture.library.id)
        .await
        .expect("member library grant");
    let data = json(
        fixture
            .schema
            .execute(fixture::request(join, fixture.member))
            .await,
        "join with target access",
    );
    assert_eq!(data["joinWatchRoom"]["id"], room_id);

    // Target access alone does not make an outsider a room member.
    fixture
        .store
        .grant_library_access(fixture.outsider.user_id, fixture.library.id)
        .await
        .expect("outsider library grant");
    let query = format!(r#"{{ watchRoom(id: "{room_id}") {{ id }} }}"#);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, fixture.outsider))
            .await,
        "nonmember watchRoom",
    );
    assert!(data["watchRoom"].is_null());

    let query = r#"{ watchRooms { id } }"#.to_string();
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, fixture.outsider))
            .await,
        "nonmember watchRooms",
    );
    assert!(
        data["watchRooms"]
            .as_array()
            .expect("watch rooms")
            .is_empty()
    );

    let control = format!(
        r#"mutation {{ controlWatchRoom(roomId: "{room_id}", action: PLAY, positionMs: 10) {{ id state positionMs sequence }} }}"#
    );
    let response = fixture
        .schema
        .execute(fixture::request(control.clone(), fixture.outsider))
        .await;
    assert_error(&response, "nonmember control", "watch room not found");
    expect_subscription_error(
        &fixture.schema,
        &room_id,
        fixture.outsider,
        "nonmember subscription",
    )
    .await;

    let response = fixture
        .schema
        .execute(fixture::request(control.clone(), fixture.member))
        .await;
    assert_error(&response, "member control", "watch room not found");

    let data = json(
        fixture
            .schema
            .execute(fixture::request(control, fixture.host))
            .await,
        "host control",
    );
    assert_eq!(data["controlWatchRoom"]["state"], "PLAYING");
    assert_eq!(data["controlWatchRoom"]["positionMs"], 10);
    assert_eq!(data["controlWatchRoom"]["sequence"], 1);

    let typed_room = WatchRoomId::from_uuid(room_id.parse().expect("room UUID"));
    let persisted = fixture
        .store
        .get_watch_room(typed_room)
        .await
        .expect("persisted room")
        .expect("room exists");
    assert_eq!(persisted.state, WatchRoomState::Playing);
    assert_eq!(persisted.sequence, 1);
    assert_eq!(persisted.position_ms, 10);

    let stop = format!(
        r#"mutation {{ controlWatchRoom(roomId: "{room_id}", action: STOP) {{ id state sequence }} }}"#
    );
    let data = json(
        fixture
            .schema
            .execute(fixture::request(stop, fixture.host))
            .await,
        "host stop",
    );
    assert_eq!(data["controlWatchRoom"]["state"], "ENDED");
    assert_eq!(data["controlWatchRoom"]["sequence"], 2);

    let persisted = fixture
        .store
        .get_watch_room(typed_room)
        .await
        .expect("persisted stopped room")
        .expect("stopped room exists");
    assert_eq!(persisted.state, WatchRoomState::Ended);
    assert_eq!(persisted.sequence, 2);

    let play_after_stop =
        format!(r#"mutation {{ controlWatchRoom(roomId: "{room_id}", action: PLAY) {{ id }} }}"#);
    let response = fixture
        .schema
        .execute(fixture::request(play_after_stop, fixture.host))
        .await;
    assert_error(&response, "control after STOP", "watch room not found");

    let join_after_stop = format!(r#"mutation {{ joinWatchRoom(roomId: "{room_id}") {{ id }} }}"#);
    let response = fixture
        .schema
        .execute(fixture::request(join_after_stop, fixture.member))
        .await;
    assert_error(&response, "join after STOP", "watch room not found");

    // A host leave is a terminal event and clears room membership while
    // retaining the ended snapshot in storage.
    let second_room = create_room(&fixture, fixture.host).await;
    let join_second = format!(r#"mutation {{ joinWatchRoom(roomId: "{second_room}") {{ id }} }}"#);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(join_second, fixture.member))
            .await,
        "join second room",
    );
    assert_eq!(data["joinWatchRoom"]["id"], second_room);

    let event_query = format!(
        r#"subscription {{ watchRoomEvents(roomId: "{second_room}") {{ room {{ id state sequence }} action sequence }} }}"#
    );
    let mut stream = fixture
        .schema
        .execute_stream(fixture::request(event_query, fixture.host));
    let leave = format!(r#"mutation {{ leaveWatchRoom(roomId: "{second_room}") }}"#);
    let schema = fixture.schema.clone();
    let host = fixture.host;
    let (leave_tx, leave_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let response = schema.execute(fixture::request(leave, host)).await;
        let _ = leave_tx.send(response);
    });
    let event_response = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("host leave event timed out")
        .expect("host leave subscription ended");
    let leave_response = leave_rx.await.expect("leave response");
    let leave_data = json(leave_response, "host leave");
    assert_eq!(leave_data["leaveWatchRoom"], true);
    let event = json(event_response, "host leave event");
    assert_eq!(event["watchRoomEvents"]["room"]["id"], second_room);
    assert_eq!(event["watchRoomEvents"]["room"]["state"], "ENDED");
    assert_eq!(event["watchRoomEvents"]["room"]["sequence"], 1);
    assert_eq!(event["watchRoomEvents"]["action"], "STOP");
    assert_eq!(event["watchRoomEvents"]["sequence"], 1);

    let second_typed = WatchRoomId::from_uuid(second_room.parse().expect("room UUID"));
    let ended = fixture
        .store
        .get_watch_room(second_typed)
        .await
        .expect("ended room")
        .expect("ended room snapshot");
    assert_eq!(ended.state, WatchRoomState::Ended);
    assert_eq!(ended.sequence, 1);
    assert!(
        fixture
            .store
            .list_watch_room_members(second_typed)
            .await
            .expect("ended room members")
            .is_empty()
    );

    let query = format!(r#"{{ watchRoom(id: "{second_room}") {{ id }} }}"#);
    let data = json(
        fixture
            .schema
            .execute(fixture::request(query, fixture.host))
            .await,
        "ended room query",
    );
    assert!(data["watchRoom"].is_null());
    expect_subscription_error(
        &fixture.schema,
        &second_room,
        fixture.host,
        "ended room subscription",
    )
    .await;
}

#[tokio::test]
async fn e7_watch_subscription_filters_other_rooms_and_reports_actions() {
    let fixture = fixture::watch().await;
    fixture
        .store
        .grant_library_access(fixture.host.user_id, fixture.library.id)
        .await
        .expect("host library grant");

    let room_a = create_room(&fixture, fixture.host).await;
    let room_b = create_room(&fixture, fixture.host).await;
    let query = format!(
        r#"subscription {{ watchRoomEvents(roomId: "{room_a}") {{ room {{ id state sequence }} action sequence }} }}"#
    );
    let mut stream = fixture
        .schema
        .execute_stream(fixture::request(query, fixture.host));

    let cross_room_control = format!(
        r#"mutation {{ controlWatchRoom(roomId: "{room_b}", action: PLAY, positionMs: 50) {{ id }} }}"#
    );
    let schema = fixture.schema.clone();
    let host = fixture.host;
    let (cross_tx, cross_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let response = schema
            .execute(fixture::request(cross_room_control, host))
            .await;
        let _ = cross_tx.send(response);
    });
    match tokio::time::timeout(Duration::from_millis(250), stream.next()).await {
        Err(_) => {}
        Ok(Some(response)) => panic!("cross-room event leaked: {:?}", response.data),
        Ok(None) => panic!("watch subscription ended while filtering rooms"),
    }
    let cross_response = cross_rx.await.expect("cross-room control response");
    assert!(
        cross_response.errors.is_empty(),
        "cross-room control: {:?}",
        cross_response.errors
    );

    let same_room_control = format!(
        r#"mutation {{ controlWatchRoom(roomId: "{room_a}", action: PAUSE, positionMs: 75) {{ id state sequence }} }}"#
    );
    let response = fixture
        .schema
        .execute(fixture::request(same_room_control, fixture.host))
        .await;
    let data = json(response, "same-room control");
    assert_eq!(data["controlWatchRoom"]["sequence"], 1);

    let event_response = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("same-room event timed out")
        .expect("same-room subscription ended");
    let event = json(event_response, "same-room event");
    assert_eq!(event["watchRoomEvents"]["room"]["id"], room_a);
    assert_eq!(event["watchRoomEvents"]["room"]["state"], "PAUSED");
    assert_eq!(event["watchRoomEvents"]["room"]["sequence"], 1);
    assert_eq!(event["watchRoomEvents"]["action"], "PAUSE");
    assert_eq!(event["watchRoomEvents"]["sequence"], 1);

    let typed_room = WatchRoomId::from_uuid(room_a.parse().expect("room UUID"));
    let persisted = fixture
        .store
        .get_watch_room(typed_room)
        .await
        .expect("persisted same-room control")
        .expect("room A");
    assert_eq!(persisted.state, WatchRoomState::Paused);
    assert_eq!(persisted.sequence, 1);
}
