// SPDX-License-Identifier: AGPL-3.0-only
//
// Play history end to end: progress heartbeats become viewings, and
// the admin stats queries roll them up.

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::Principal;
use noflex_core::{LibraryId, UserId, UserRole, WorkId, WorkKind};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_storage::{
    RequestUser,
    domain::{Library, LibraryConfig, LibraryKind, User, Work},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use serde_json::Value;
use time::OffsetDateTime;

async fn schema() -> (NoflexSchema, Arc<TursoStore>) {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        ..Default::default()
    });
    (schema, store)
}

async fn user(store: &TursoStore, name: &str, admin: bool) -> User {
    let user = User {
        id: UserId::new(),
        name: name.into(),
        password_hash: String::new(),
        is_admin: admin,
        role: if admin {
            UserRole::Admin
        } else {
            UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
    };
    store.upsert_user(&user).await.expect("user");
    user
}

async fn library(store: &TursoStore, name: &str, kind: LibraryKind) -> LibraryId {
    let lib = Library {
        id: LibraryId::new(),
        name: name.into(),
        kind,
        paths: vec![format!("/srv/{name}")],
        config: LibraryConfig::default(),
    };
    store.upsert_library(&lib).await.expect("library");
    lib.id
}

async fn work(
    store: &TursoStore,
    library: LibraryId,
    kind: WorkKind,
    title: &str,
    parent: Option<WorkId>,
) -> WorkId {
    let now = OffsetDateTime::now_utc();
    let work = Work {
        id: WorkId::new(),
        kind,
        parent_id: parent,
        library_id: library,
        title: title.into(),
        sort_title: title.into(),
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
    };
    store.upsert_work(&work).await.expect("work");
    work.id
}

async fn run(schema: &NoflexSchema, as_user: &User, query: &str) -> Result<Value, String> {
    let request = Request::new(query)
        .data(Principal {
            user_id: as_user.id,
            is_admin: as_user.is_admin,
            session_id: None,
        })
        .data(RequestUser {
            user_id: as_user.id,
            is_admin: as_user.is_admin,
        });
    let resp = schema.execute(request).await;
    if resp.errors.is_empty() {
        Ok(resp.data.into_json().expect("json"))
    } else {
        Err(format!("{:?}", resp.errors))
    }
}

async fn report(schema: &NoflexSchema, as_user: &User, work: WorkId, state: &str) {
    let query = format!(
        r#"mutation {{ reportProgress(target: {{ kind: WORK, id: "{work}" }}, positionMs: 5000, state: {state}) {{ positionMs }} }}"#
    );
    run(schema, as_user, &query).await.expect("reportProgress");
}

#[tokio::test]
async fn heartbeats_roll_up_into_stats() {
    let (schema, store) = schema().await;
    let admin = user(&store, "admin", true).await;
    let viewer = user(&store, "viewer", false).await;
    let films = library(&store, "Films", LibraryKind::Movies).await;
    let shows = library(&store, "Shows", LibraryKind::Shows).await;
    let movie = work(&store, films, WorkKind::Movie, "Alien", None).await;
    let series = work(&store, shows, WorkKind::Series, "Severance", None).await;
    let season = work(&store, shows, WorkKind::Season, "Season 1", Some(series)).await;
    let episode = work(&store, shows, WorkKind::Episode, "Good News", Some(season)).await;

    // Heartbeats continue one viewing until a stop; the next play opens
    // a second one. A stop with nothing open records nothing.
    report(&schema, &admin, episode, "PLAYING").await;
    report(&schema, &admin, episode, "PAUSED").await;
    report(&schema, &admin, episode, "STOPPED").await;
    report(&schema, &admin, episode, "STOPPED").await;
    report(&schema, &admin, episode, "PLAYING").await;
    report(&schema, &admin, movie, "PLAYING").await;

    let top = run(
        &schema,
        &admin,
        "{ topUsers(range: LAST_7_DAYS) { user { name } plays libraries { plays library { name } } } }",
    )
    .await
    .expect("topUsers");
    let users = top["topUsers"].as_array().expect("users");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["user"]["name"], "admin");
    assert_eq!(users[0]["plays"], 3);
    let mut split: Vec<(String, i64)> = users[0]["libraries"]
        .as_array()
        .expect("libraries")
        .iter()
        .map(|l| {
            (
                l["library"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                l["plays"].as_i64().unwrap_or_default(),
            )
        })
        .collect();
    split.sort();
    assert_eq!(split, vec![("Films".into(), 1), ("Shows".into(), 2)]);

    let played = run(
        &schema,
        &admin,
        "{ topPlayed(range: ALL_TIME) { library { name } plays items { plays users work { title } } } }",
    )
    .await
    .expect("topPlayed");
    let libraries = played["topPlayed"].as_array().expect("libraries");
    assert_eq!(libraries[0]["library"]["name"], "Shows");
    assert_eq!(
        libraries[0]["items"][0]["work"]["title"], "Severance",
        "episodes roll up to their series"
    );
    assert_eq!(libraries[0]["items"][0]["plays"], 2);
    assert_eq!(libraries[0]["items"][0]["users"], 1);

    let chart = run(
        &schema,
        &admin,
        &format!(
            r#"{{ playHistoryChart(range: LAST_24_HOURS, libraryId: "{shows}") {{ bucket totalPlays points {{ plays }} }} }}"#
        ),
    )
    .await
    .expect("chart");
    assert_eq!(chart["playHistoryChart"]["bucket"], "HOUR");
    assert_eq!(chart["playHistoryChart"]["totalPlays"], 2);
    assert_eq!(
        chart["playHistoryChart"]["points"].as_array().map(Vec::len),
        Some(24)
    );

    let page = run(
        &schema,
        &admin,
        "{ playSessions(first: 2) { pageInfo { hasNextPage } nodes { ended work { title } rootWork { title } } } }",
    )
    .await
    .expect("playSessions");
    let nodes = page["playSessions"]["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["work"]["title"], "Alien");
    assert_eq!(nodes[1]["rootWork"]["title"], "Severance");
    assert_eq!(page["playSessions"]["pageInfo"]["hasNextPage"], true);

    // Non-admins can't read the server-wide rollups or anyone else's
    // history, but can page their own (empty) history.
    assert!(
        run(&schema, &viewer, "{ topUsers(range: ALL_TIME) { plays } }")
            .await
            .is_err()
    );
    assert!(
        run(
            &schema,
            &viewer,
            &format!(
                r#"{{ playSessions(userId: "{}") {{ nodes {{ id }} }} }}"#,
                admin.id
            )
        )
        .await
        .is_err()
    );
    let own = run(&schema, &viewer, "{ playSessions { nodes { id } } }")
        .await
        .expect("own history");
    assert_eq!(
        own["playSessions"]["nodes"].as_array().map(Vec::len),
        Some(0)
    );
}
