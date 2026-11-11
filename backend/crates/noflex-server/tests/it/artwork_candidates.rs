// SPDX-License-Identifier: AGPL-3.0-only
//
// The artwork candidate preview route against the artwork fixture guest:
// only an administrator gets the bytes, by header or by the signed token
// in `previewUrl`, and only for a candidate the service offered.

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use noflex_auth::{JwtConfig, issue_jwt, issue_media_token};
use noflex_core::{ExternalId, ExternalIdKind, ImageKind, LibraryId, SessionId, UserId, WorkId};
use noflex_plugin_host::{EngineConfig, MetadataService, PluginRegistry, build_engine};
use noflex_server::{
    artwork_candidates::{ArtworkCandidateState, routes},
    stream::artwork_candidate_token_target,
};
use noflex_storage::{
    Storage,
    domain::{Library, LibraryKind, User, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;
use tower::ServiceExt as _;

const JWT_SECRET: &[u8] = b"artwork-candidates-test-secret-0123456789";

fn jwt() -> Arc<JwtConfig> {
    Arc::new(JwtConfig::dev(JWT_SECRET).expect("JWT config"))
}

async fn seed_user(storage: &Arc<dyn Storage>, name: &str, is_admin: bool) -> User {
    let user = User {
        id: UserId::new(),
        username: name.into(),
        display_name: None,
        password_hash: String::new(),
        is_admin,
        role: if is_admin {
            noflex_core::UserRole::Admin
        } else {
            noflex_core::UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    };
    storage.upsert_user(&user).await.expect("user");
    let now = OffsetDateTime::now_utc();
    storage
        .create_session(&noflex_storage::domain::Session {
            id: SessionId::from_uuid(user.id.into_uuid()),
            user_id: user.id,
            client: None,
            device: None,
            device_id: None,
            app_version: None,
            remote_addr: None,
            created_at: now,
            last_seen_at: now,
            expires_at: now + time::Duration::days(1),
            refresh_token_hash: None,
            locked_at: None,
        })
        .await
        .expect("session");
    user
}

fn bearer(user: &User) -> String {
    let session = SessionId::from_uuid(user.id.into_uuid());
    let token = issue_jwt(&jwt(), user.id, user.is_admin, session).expect("jwt");
    format!("Bearer {token}")
}

async fn status(app: &axum::Router, path: &str, user: Option<&User>) -> StatusCode {
    let mut request = Request::builder().uri(path);
    if let Some(user) = user {
        request = request.header(header::AUTHORIZATION, bearer(user));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    response.status()
}

#[tokio::test]
async fn previews_are_for_administrators_and_offered_candidates_only() {
    let Some(wasm) = crate::fixtures::wasm("artwork") else {
        eprintln!("skipping: could not build the artwork fixture plugin");
        return;
    };
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let registry = Arc::new(
        PluginRegistry::new(build_engine(&EngineConfig::default()).expect("engine"))
            .expect("registry")
            .with_storage(storage.clone()),
    );
    registry.load_from(&wasm).expect("load fixture");
    let images = tempfile::tempdir().expect("image dir");
    let service = Arc::new(
        MetadataService::new(registry, storage.clone()).with_image_dir(images.path().into()),
    );

    let library = Library {
        id: LibraryId::new(),
        name: "Films".into(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/films".into()],
        ..Default::default()
    };
    storage.upsert_library(&library).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = Work {
        id: WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Heat".into(),
        sort_title: "heat".into(),
        year: None,
        release_date: None,
        overview: None,
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: vec![ExternalId {
            kind: ExternalIdKind::Tmdb,
            value: "movie/heat".into(),
        }],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: noflex_core::MetadataStatus::Confirmed,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };
    storage.upsert_work(&work).await.expect("work");
    let admin = seed_user(&storage, "admin", true).await;
    let viewer = seed_user(&storage, "viewer", false).await;

    let candidates = service
        .artwork_candidates(work.id, ImageKind::Backdrop)
        .await
        .expect("candidates");
    let id = candidates[0].id.as_str().to_string();
    let app = routes(ArtworkCandidateState {
        service,
        storage,
        jwt: jwt(),
    });
    let path = format!("/artwork-candidates/{id}/preview");

    assert_eq!(status(&app, &path, Some(&admin)).await, StatusCode::OK);
    assert_eq!(
        status(&app, &path, Some(&viewer)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(status(&app, &path, None).await, StatusCode::UNAUTHORIZED);
    let token = |user: &User, candidate: &str| {
        issue_media_token(
            &jwt(),
            user.id,
            None,
            &artwork_candidate_token_target(candidate),
            Duration::from_secs(60),
        )
        .expect("media token")
    };
    assert_eq!(
        status(&app, &format!("{path}?token={}", token(&admin, &id)), None).await,
        StatusCode::OK,
        "the token stands in for the header"
    );
    assert_eq!(
        status(
            &app,
            &format!("{path}?token={}", token(&admin, "other")),
            None
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "a token names one candidate"
    );
    assert_eq!(
        status(
            &app,
            "/artwork-candidates/never-offered/preview",
            Some(&admin)
        )
        .await,
        StatusCode::NOT_FOUND
    );
}
