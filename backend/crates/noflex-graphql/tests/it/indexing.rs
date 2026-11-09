// SPDX-License-Identifier: AGPL-3.0-only
//
// Clients keep a card's placeholder up to date by asking for the works on
// screen, so that query can't show anyone a work they couldn't otherwise see.

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::Principal;
use noflex_core::{LibraryId, MetadataStatus, UserId, WorkId, WorkKind};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{
    IndexingProgress, RequestUser, Storage,
    domain::{Library, LibraryKind, User, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

fn user(name: &str, now: OffsetDateTime) -> User {
    User {
        id: UserId::new(),
        username: name.into(),
        display_name: None,
        password_hash: "argon2id$placeholder".into(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: now,
        avatar_updated_at: None,
        has_pin: false,
    }
}

#[tokio::test]
async fn cards_follow_indexing_only_for_works_the_viewer_can_see() {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let now = OffsetDateTime::now_utc();
    let viewer = user("viewer", now);
    let stranger = user("stranger", now);
    storage.upsert_user(&viewer).await.expect("viewer");
    storage.upsert_user(&stranger).await.expect("stranger");
    let library = Library {
        id: LibraryId::new(),
        name: "Films".into(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/films".into()],
        ..Default::default()
    };
    storage.upsert_library(&library).await.expect("library");
    storage
        .grant_library_access(viewer.id, library.id)
        .await
        .expect("grant");
    let film = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Film".into(),
        sort_title: "film".into(),
        year: None,
        release_date: None,
        overview: None,
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };
    storage.upsert_work(&film).await.expect("film");
    let progress = Arc::new(IndexingProgress::default());
    progress.sweep_begun();

    let schema = build_schema(SchemaContext {
        storage: Some(storage),
        ..Default::default()
    });
    let ask = |who: &User| {
        Request::new(format!(
            r#"{{ worksById(ids: ["{}"]) {{ title indexing }} }}"#,
            film.id
        ))
        .data(progress.clone())
        .data(Principal {
            user_id: who.id,
            is_admin: false,
            session_id: None,
        })
        .data(RequestUser {
            user_id: who.id,
            is_admin: false,
        })
    };

    let seen = schema.execute(ask(&viewer)).await;
    assert!(seen.errors.is_empty(), "{:?}", seen.errors);
    let seen = seen.data.into_json().expect("json");
    assert_eq!(seen["worksById"][0]["indexing"], "FINDING_DETAILS");

    let hidden = schema.execute(ask(&stranger)).await;
    assert!(hidden.errors.is_empty(), "{:?}", hidden.errors);
    let hidden = hidden.data.into_json().expect("json");
    assert_eq!(hidden["worksById"].as_array().map(Vec::len), Some(0));
}
