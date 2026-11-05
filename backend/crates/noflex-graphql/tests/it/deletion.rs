// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{EditionId, LibraryId, MediaFileId, SessionId, UserId, WorkId, WorkKind};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{
    RequestUser, Storage,
    domain::{Edition, Library, LibraryConfig, LibraryKind, MediaFile, Work},
};
use noflex_storage_turso::TursoStore;
use tempfile::TempDir;
use time::OffsetDateTime;

fn as_request_user(principal: Principal) -> RequestUser {
    RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    }
}

const TEST_JWT_SECRET: &[u8] = b"deletion-secret-32-bytes-for-tests";

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
    login(schema, jwt, "admin", "hunter2").await
}

async fn login(
    schema: &noflex_graphql::NoflexSchema,
    jwt: &JwtConfig,
    username: &str,
    password: &str,
) -> Principal {
    let query = format!(
        r#"mutation {{ login(username: "{username}", password: "{password}") {{ token }} }}"#
    );
    let resp = schema.execute(query).await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let claims = verify_jwt(jwt, data["login"]["token"].as_str().unwrap()).expect("verify token");
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
    password: &str,
) {
    let query = format!(
        r#"mutation {{ createUser(username: "{username}", password: "{password}", role: USER) {{ id }} }}"#
    );
    let resp = schema.execute(authed_request(query, admin)).await;
    assert!(resp.errors.is_empty(), "createUser: {:?}", resp.errors);
}

struct CatalogFixture {
    library: Library,
    work: Work,
    edition: Edition,
    media_file: MediaFile,
}

async fn seed_catalog(store: &TursoStore, temp_dir: &TempDir, filename: &str) -> CatalogFixture {
    let library = Library {
        id: LibraryId::new(),
        name: "Films".to_string(),
        kind: LibraryKind::Movies,
        paths: vec![temp_dir.path().display().to_string()],
        config: LibraryConfig::default(),
    };
    store.upsert_library(&library).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Alien".to_string(),
        sort_title: "Alien".to_string(),
        year: Some(1979),
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
    let edition = Edition {
        id: EditionId::new(),
        work_id: work.id,
        name: "Default".to_string(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    store.upsert_edition(&edition).await.expect("edition");
    let path = temp_dir.path().join(filename);
    tokio::fs::write(&path, b"media").await.expect("media file");
    let media_file = MediaFile {
        id: MediaFileId::new(),
        edition_id: edition.id,
        path: path.display().to_string(),
        part_index: 0,
        size_bytes: 5,
        mtime: now,
        container: Some("matroska".to_string()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: vec![],
    };
    store
        .upsert_media_file(&media_file)
        .await
        .expect("media row");
    CatalogFixture {
        library,
        work,
        edition,
        media_file,
    }
}

fn delete_media_file(id: MediaFileId, delete_file: bool) -> String {
    format!(r#"mutation {{ deleteMediaFile(id: "{id}", deleteFile: {delete_file}) }}"#)
}

#[tokio::test]
async fn deletion_requires_an_administrator() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let fixture = seed_catalog(&store, &temp_dir, "alien.mkv").await;
    let mutation = delete_media_file(fixture.media_file.id, false);

    let resp = schema.execute(mutation.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|error| error.message == "authentication required"),
        "anonymous deletion: {:?}",
        resp.errors
    );

    create_user(&schema, admin, "bob", "pw1").await;
    let user = login(&schema, &jwt, "bob", "pw1").await;
    let resp = schema
        .execute(
            Request::new(mutation.as_str())
                .data(user)
                .data(as_request_user(user)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|error| error.message == "administrator required"),
        "non-admin deletion: {:?}",
        resp.errors
    );
    assert!(
        store
            .media_file_by_id(fixture.media_file.id)
            .await
            .expect("media lookup")
            .is_some()
    );
}

#[tokio::test]
async fn row_only_media_file_deletion_keeps_the_file() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let fixture = seed_catalog(&store, &temp_dir, "alien.mkv").await;

    let resp = schema
        .execute(authed_request(
            delete_media_file(fixture.media_file.id, false),
            admin,
        ))
        .await;
    assert!(resp.errors.is_empty(), "deleteMediaFile: {:?}", resp.errors);
    assert!(
        tokio::fs::try_exists(&fixture.media_file.path)
            .await
            .unwrap()
    );

    let query = format!(
        r#"{{ mediaFiles(editionId: "{}") {{ id }} }}"#,
        fixture.edition.id
    );
    let resp = schema.execute(authed_request(query, admin)).await;
    assert!(resp.errors.is_empty(), "mediaFiles: {:?}", resp.errors);
    assert!(
        resp.data.into_json().unwrap()["mediaFiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn media_file_deletion_removes_the_file() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let fixture = seed_catalog(&store, &temp_dir, "alien.mkv").await;

    let resp = schema
        .execute(authed_request(
            delete_media_file(fixture.media_file.id, true),
            admin,
        ))
        .await;
    assert!(resp.errors.is_empty(), "deleteMediaFile: {:?}", resp.errors);
    assert!(
        !tokio::fs::try_exists(&fixture.media_file.path)
            .await
            .unwrap()
    );
    assert!(
        store
            .media_file_by_id(fixture.media_file.id)
            .await
            .expect("media lookup")
            .is_none()
    );
}

#[tokio::test]
async fn work_deletion_cascades_and_removes_files() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let fixture = seed_catalog(&store, &temp_dir, "alien.mkv").await;
    let mutation = format!(
        r#"mutation {{ deleteWork(id: "{}", deleteFile: true) }}"#,
        fixture.work.id
    );

    let resp = schema.execute(authed_request(mutation, admin)).await;
    assert!(resp.errors.is_empty(), "deleteWork: {:?}", resp.errors);
    assert!(
        !tokio::fs::try_exists(&fixture.media_file.path)
            .await
            .unwrap()
    );

    let query = format!(r#"{{ work(id: "{}") {{ id }} }}"#, fixture.work.id);
    let resp = schema.execute(authed_request(query, admin)).await;
    assert!(resp.errors.is_empty(), "work: {:?}", resp.errors);
    assert!(resp.data.into_json().unwrap()["work"].is_null());

    let query = format!(r#"{{ editions(workId: "{}") {{ id }} }}"#, fixture.work.id);
    let resp = schema.execute(authed_request(query, admin)).await;
    assert!(resp.errors.is_empty(), "editions: {:?}", resp.errors);
    assert!(
        resp.data.into_json().unwrap()["editions"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let query = format!(
        r#"{{ mediaFiles(editionId: "{}") {{ id }} }}"#,
        fixture.edition.id
    );
    let resp = schema.execute(authed_request(query, admin)).await;
    assert!(resp.errors.is_empty(), "mediaFiles: {:?}", resp.errors);
    assert!(
        resp.data.into_json().unwrap()["mediaFiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .get_library(fixture.library.id)
            .await
            .expect("library lookup")
            .is_some()
    );
}

#[tokio::test]
async fn deleting_a_nonexistent_media_file_reports_not_found() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let mutation = delete_media_file(MediaFileId::new(), false);

    let resp = schema.execute(authed_request(mutation, admin)).await;
    assert!(
        resp.errors
            .iter()
            .any(|error| error.message == "media file not found"),
        "missing media file: {:?}",
        resp.errors
    );
}
