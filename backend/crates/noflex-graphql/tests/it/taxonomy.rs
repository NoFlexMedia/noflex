// SPDX-License-Identifier: AGPL-3.0-only
//
// Taxonomy surface: `tags` summaries with per-namespace filtering and
// work counts, `tag` detail with its filmography, and the `works`
// tag filter through an authenticated admin request.

use std::sync::Arc;

use noflex_core::{LibraryId, TagId, UserId, WorkId, WorkKind};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{
    RequestUser,
    domain::{Library, LibraryConfig, LibraryKind, Tag, User, Work},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

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

fn seed_tag(namespace: &str, slug: &str, display: &str) -> Tag {
    Tag {
        id: TagId::new(),
        namespace: namespace.to_string(),
        parent_id: None,
        slug: slug.to_string(),
        display: display.to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    }
}

struct Seeded {
    schema: noflex_graphql::NoflexSchema,
    request_user: RequestUser,
    lib: Library,
    alien: Work,
    blade: Work,
    scifi: Tag,
    ridley: Tag,
    /// Assigned to nothing — exercises the zero-count path.
    bleak: Tag,
}

async fn seeded() -> Seeded {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        ..Default::default()
    });
    let user = User {
        id: UserId::new(),
        username: "taxonomy-admin".to_string(),
        display_name: None,
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: true,
        role: noflex_core::UserRole::Admin,
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
    };
    store.upsert_user(&user).await.expect("user");

    let lib = seed_library();
    store.upsert_library(&lib).await.expect("library");
    let alien = seed_work(lib.id, "Alien");
    let blade = seed_work(lib.id, "Blade Runner");
    store.upsert_work(&alien).await.expect("alien");
    store.upsert_work(&blade).await.expect("blade");

    let scifi = seed_tag("genre", "sci-fi", "Sci-Fi");
    let ridley = seed_tag("person", "ridley-scott", "Ridley Scott");
    let bleak = seed_tag("mood", "bleak", "Bleak");
    for t in [&scifi, &ridley, &bleak] {
        store.upsert_tag(t).await.expect("tag");
    }
    let assign = |tag: TagId, work: WorkId| {
        store.assign_tag(tag, noflex_core::TagTarget::Work(work), 1.0, "test")
    };
    assign(scifi.id, alien.id).await.expect("scifi alien");
    assign(scifi.id, blade.id).await.expect("scifi blade");
    assign(ridley.id, alien.id).await.expect("ridley alien");

    Seeded {
        schema,
        request_user: RequestUser {
            user_id: user.id,
            is_admin: true,
        },
        lib,
        alien,
        blade,
        scifi,
        ridley,
        bleak,
    }
}

impl Seeded {
    fn request(&self, query: impl Into<String>) -> async_graphql::Request {
        async_graphql::Request::new(query).data(self.request_user)
    }
}

#[tokio::test]
async fn tags_lists_summaries_with_counts_per_namespace() {
    let s = seeded().await;

    let resp = s
        .schema
        .execute(s.request(
            r#"{ tags(namespaces: ["genre"]) { nodes { id tag { id slug namespace } workCount } pageInfo { hasNextPage endCursor } } }"#,
        ))
        .await;
    assert!(resp.errors.is_empty(), "tags: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let items = data["tags"]["nodes"].as_array().expect("items");
    assert_eq!(items.len(), 1, "other namespaces filtered out");
    assert_eq!(items[0]["tag"]["slug"], s.scifi.slug);
    assert_eq!(items[0]["id"], items[0]["tag"]["id"]);
    assert_eq!(items[0]["workCount"], 2);
    assert_eq!(data["tags"]["pageInfo"]["hasNextPage"], false);

    // Empty namespaces = every namespace, zero-count tags included,
    // ordered by display name.
    let resp = s
        .schema
        .execute(s.request(r#"{ tags(namespaces: []) { nodes { tag { slug } workCount } } }"#))
        .await;
    assert!(resp.errors.is_empty(), "tags all: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let items = data["tags"]["nodes"].as_array().expect("items");
    let slugs: Vec<&str> = items
        .iter()
        .map(|i| i["tag"]["slug"].as_str().expect("slug"))
        .collect();
    assert_eq!(slugs, ["bleak", "ridley-scott", "sci-fi"]);
    let count_of = |slug: &str| {
        items
            .iter()
            .find(|i| i["tag"]["slug"] == slug)
            .and_then(|i| i["workCount"].as_u64())
            .expect("count")
    };
    assert_eq!(count_of(&s.bleak.slug), 0);
    assert_eq!(count_of(&s.ridley.slug), 1);
    assert_eq!(count_of(&s.scifi.slug), 2);
}

#[tokio::test]
async fn tag_detail_includes_filmography() {
    let s = seeded().await;

    let q = format!(
        r#"{{ tag(id: "{}") {{ slug display works {{ nodes {{ id title }} pageInfo {{ hasNextPage endCursor }} }} }} }}"#,
        s.scifi.id
    );
    let resp = s.schema.execute(s.request(q.as_str())).await;
    assert!(resp.errors.is_empty(), "tag: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["tag"]["slug"], "sci-fi");
    let works = data["tag"]["works"]["nodes"].as_array().expect("works");
    let ids: Vec<&str> = works
        .iter()
        .map(|w| w["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids.len(), 2, "both sci-fi works");
    assert!(ids.contains(&s.alien.id.to_string().as_str()));
    assert!(ids.contains(&s.blade.id.to_string().as_str()));

    // An unknown id is null, not an error.
    let q = format!(r#"{{ tag(id: "{}") {{ slug }} }}"#, TagId::new());
    let resp = s.schema.execute(s.request(q.as_str())).await;
    assert!(resp.errors.is_empty(), "absent tag: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["tag"].is_null());
}

#[tokio::test]
async fn works_filter_by_tag() {
    let s = seeded().await;

    let q = format!(
        r#"{{ works(libraryId: "{}", filter: {{ tag: "{}" }}) {{ nodes {{ id }} }} }}"#,
        s.lib.id, s.scifi.id
    );
    let resp = s.schema.execute(s.request(q.as_str())).await;
    assert!(resp.errors.is_empty(), "works by tag: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let items = data["works"]["nodes"].as_array().expect("items");
    assert_eq!(items.len(), 2);

    // A tag nobody carries yields an empty page.
    let q = format!(
        r#"{{ works(libraryId: "{}", filter: {{ tag: "{}" }}) {{ nodes {{ id }} }} }}"#,
        s.lib.id, s.bleak.id
    );
    let resp = s.schema.execute(s.request(q.as_str())).await;
    assert!(
        resp.errors.is_empty(),
        "works by empty tag: {:?}",
        resp.errors
    );
    assert_eq!(
        resp.data.into_json().unwrap()["works"]["nodes"]
            .as_array()
            .expect("items")
            .len(),
        0
    );

    // Dropping the filter lists the whole library.
    let q = format!(
        r#"{{ works(libraryId: "{}") {{ nodes {{ id }} }} }}"#,
        s.lib.id
    );
    let resp = s.schema.execute(s.request(q.as_str())).await;
    assert!(
        resp.errors.is_empty(),
        "works unfiltered: {:?}",
        resp.errors
    );
    let data = resp.data.into_json().unwrap();
    let items = data["works"]["nodes"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .any(|w| w["id"].as_str() == Some(s.alien.id.to_string().as_str()))
    );
}
