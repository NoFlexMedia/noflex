// SPDX-License-Identifier: AGPL-3.0-only
//
// Library search surface: mode dispatch (lexical / vector / hybrid),
// the no-embedder degrade paths, and kinds/limit handling. Reads are
// use an authenticated admin request. No PluginRegistry is wired
// into the schema here — that is exactly the embedder-absent
// environment the degrade paths exist for.

use std::sync::Arc;

use noflex_core::{LibraryId, UserId, WorkId, WorkKind};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{
    RequestUser,
    domain::{Library, LibraryConfig, LibraryKind, User, Work},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

fn seed_work(library: LibraryId, kind: WorkKind, title: &str) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind,
        parent_id: None,
        library_id: library,
        title: title.to_string(),
        sort_title: title.to_string(),
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
        added_at: now,
        updated_at: now,
    }
}

struct Seeded {
    schema: noflex_graphql::NoflexSchema,
    request_user: RequestUser,
    dune: Work,
    dune_prophecy: Work,
}

async fn seeded() -> Seeded {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let schema = build_schema(SchemaContext::new(store.clone()));
    let user = User {
        id: UserId::new(),
        name: "search-admin".to_string(),
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: true,
        role: noflex_core::UserRole::Admin,
        created_at: OffsetDateTime::now_utc(),
    };
    store.upsert_user(&user).await.expect("user");

    let lib = Library {
        id: LibraryId::new(),
        name: "Films".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/films".to_string()],
        config: LibraryConfig::default(),
    };
    store.upsert_library(&lib).await.expect("library");
    let dune = seed_work(lib.id, WorkKind::Movie, "Dune");
    let dune_prophecy = seed_work(lib.id, WorkKind::Series, "Dune: Prophecy");
    let unrelated = seed_work(lib.id, WorkKind::Movie, "Alien");
    for w in [&dune, &dune_prophecy, &unrelated] {
        store.upsert_work(w).await.expect("work");
    }

    Seeded {
        schema,
        request_user: RequestUser {
            user_id: user.id,
            is_admin: true,
        },
        dune,
        dune_prophecy,
    }
}

impl Seeded {
    fn request(&self, query: impl Into<String>) -> async_graphql::Request {
        async_graphql::Request::new(query).data(self.request_user)
    }
}

fn hit_ids(data: &serde_json::Value) -> Vec<&str> {
    data["search"]["nodes"]
        .as_array()
        .expect("search hits")
        .iter()
        .map(|h| h["workId"].as_str().expect("work id"))
        .collect()
}

#[tokio::test]
async fn lexical_search_matches_and_filters() {
    let s = seeded().await;

    let resp = s
        .schema
        .execute(s.request(
            r#"{ search(text: "dune", mode: LEXICAL) { nodes { workId score matchedFields } } }"#,
        ))
        .await;
    assert!(resp.errors.is_empty(), "lexical: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let hits = data["search"]["nodes"].as_array().expect("hits");
    assert_eq!(hits.len(), 2, "both dune works, not the unrelated one");
    for hit in hits {
        assert!(
            !hit["matchedFields"].as_array().expect("fields").is_empty(),
            "matched fields reported"
        );
    }

    // kinds restricts to the requested work kinds.
    let resp = s
        .schema
        .execute(s.request(
            r#"{ search(text: "dune", mode: LEXICAL, kinds: [MOVIE]) { nodes { workId } } }"#,
        ))
        .await;
    assert!(resp.errors.is_empty(), "kinds: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(hit_ids(&data), [s.dune.id.to_string().as_str()]);

    // `first` is clamped to 1..=100, so `first: 0` still returns one hit.
    let resp = s
        .schema
        .execute(
            s.request(r#"{ search(text: "dune", mode: LEXICAL, first: 0) { nodes { workId } } }"#),
        )
        .await;
    assert!(resp.errors.is_empty(), "first: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(hit_ids(&data).len(), 1, "first: 0 clamps to 1");
}

#[tokio::test]
async fn hybrid_without_embedder_degrades_to_lexical() {
    let s = seeded().await;

    // Default mode is HYBRID; with no registry in the schema context
    // the resolver degrades to vectorless hybrid — the same hit set
    // lexical search returns.
    let resp = s
        .schema
        .execute(s.request(r#"{ search(text: "dune") { nodes { workId } } }"#))
        .await;
    assert!(resp.errors.is_empty(), "hybrid: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let mut hybrid_ids = hit_ids(&data);
    hybrid_ids.sort_unstable();

    let mut expected = vec![s.dune.id.to_string(), s.dune_prophecy.id.to_string()];
    expected.sort_unstable();
    assert_eq!(hybrid_ids, expected);
}

#[tokio::test]
async fn vector_mode_without_embedder_errors() {
    let s = seeded().await;

    let resp = s
        .schema
        .execute(s.request(r#"{ search(text: "dune", mode: VECTOR) { nodes { workId } } }"#))
        .await;
    assert_eq!(resp.errors.len(), 1, "vector without embedder errors");
    assert!(
        resp.errors[0]
            .message
            .contains("no embedder plugin is available"),
        "unexpected error: {}",
        resp.errors[0].message
    );
}

#[tokio::test]
async fn forbidden_search_hydration_stays_hidden() {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let user = User {
        id: UserId::new(),
        name: "restricted-search".to_string(),
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: OffsetDateTime::now_utc(),
    };
    store.upsert_user(&user).await.expect("user");
    let allowed = Library {
        id: LibraryId::new(),
        name: "Allowed".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/allowed-search".to_string()],
        config: LibraryConfig::default(),
    };
    let forbidden = Library {
        id: LibraryId::new(),
        name: "Forbidden".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/forbidden-search".to_string()],
        config: LibraryConfig::default(),
    };
    store.upsert_library(&allowed).await.expect("allowed");
    store.upsert_library(&forbidden).await.expect("forbidden");
    store
        .grant_library_access(user.id, allowed.id)
        .await
        .expect("grant");
    let visible = seed_work(allowed.id, WorkKind::Movie, "Shared title");
    let hidden = seed_work(forbidden.id, WorkKind::Movie, "Shared title");
    store.upsert_work(&visible).await.expect("visible work");
    store.upsert_work(&hidden).await.expect("hidden work");

    let response = build_schema(SchemaContext::new(store))
        .execute(
            async_graphql::Request::new(
                r#"{ search(text: "shared", mode: LEXICAL) { nodes { workId work { id title } } } }"#,
            )
            .data(RequestUser {
                user_id: user.id,
                is_admin: false,
            }),
        )
        .await;
    assert!(response.errors.is_empty(), "search: {:?}", response.errors);
    let data = response.data.into_json().expect("json");
    let hits = data["search"]["nodes"].as_array().expect("hits");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["workId"], visible.id.to_string());
    assert_eq!(hits[0]["work"]["id"], visible.id.to_string());
}

#[tokio::test]
async fn lexical_hit_hydrates_work_and_round_trips_id() {
    let s = seeded().await;

    // Hydrated `work { title }` matches the inserted row, and
    // `workId` round-trips back to the same id — so clients can
    // consume hits without a follow-up `getWork` roundtrip.
    let resp = s
        .schema
        .execute(s.request(
            r#"{ search(text: "dune", mode: LEXICAL, kinds: [MOVIE]) { nodes {
                workId
                work { id title kind }
            } } }"#,
        ))
        .await;
    assert!(resp.errors.is_empty(), "hydrate: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let hits = data["search"]["nodes"].as_array().expect("hits");
    assert_eq!(hits.len(), 1, "kinds narrows to the movie");
    let hit = &hits[0];
    assert_eq!(
        hit["workId"].as_str().expect("workId"),
        s.dune.id.to_string(),
        "workId round-trips"
    );
    assert_eq!(
        hit["work"]["title"].as_str().expect("hydrated title"),
        s.dune.title,
        "hydrated title matches the seeded row"
    );
    assert_eq!(
        hit["work"]["id"].as_str().expect("hydrated id"),
        s.dune.id.to_string(),
        "hydrated id matches workId"
    );
}
