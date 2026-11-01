// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use futures::StreamExt;
use noflex_auth::Principal;
use noflex_core::{EditionId, LibraryId, UserId};
use noflex_graphql::{
    ExportEvent, ExportEventBroadcaster, NoflexSchema, SchemaContext, build_schema,
};
use noflex_storage::{
    RequestUser, Storage,
    domain::{ExportJobStatus, User},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

struct Fixture {
    schema: NoflexSchema,
    storage: Arc<dyn Storage>,
    owner: User,
    other: User,
    admin: User,
    edition_id: EditionId,
}

fn request(query: impl Into<String>, user: &User) -> Request {
    let principal = Principal {
        user_id: user.id,
        is_admin: user.is_admin,
        session_id: None,
    };
    Request::new(query).data(principal).data(RequestUser {
        user_id: user.id,
        is_admin: user.is_admin,
    })
}

async fn fixture() -> Fixture {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let now = OffsetDateTime::now_utc();
    let owner = user("owner", false, now);
    let other = user("other", false, now);
    let admin = user("admin", true, now);
    for user in [&owner, &other, &admin] {
        storage.upsert_user(user).await.expect("upsert user");
    }

    let library = noflex_storage::domain::Library {
        id: LibraryId::new(),
        name: "Films".into(),
        kind: noflex_storage::domain::LibraryKind::Movies,
        paths: vec!["/srv/films".into()],
        ..Default::default()
    };
    storage.upsert_library(&library).await.expect("library");
    storage
        .grant_library_access(owner.id, library.id)
        .await
        .expect("owner library grant");
    let work = noflex_storage::domain::Work {
        id: noflex_core::WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Alien".into(),
        sort_title: "alien".into(),
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
        added_at: now,
        updated_at: now,
    };
    storage.upsert_work(&work).await.expect("work");
    let edition = noflex_storage::domain::Edition {
        id: EditionId::new(),
        work_id: work.id,
        name: "Director's Cut".into(),
        runtime_ms: Some(7_140_000),
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
        codec_video: None,
        codec_audio: None,
        width: None,
        height: None,
        bit_depth: None,
        hdr_format: None,
        audio_tracks: None,
        subtitle_tracks: None,
        video_codec: None,
        video_profile: None,
        video_level: None,
        pixel_format: None,
        color_primaries: None,
        transfer_characteristics: None,
        matrix_coefficients: None,
        dovi_profile: None,
        max_cll: None,
        max_fall: None,
        mastering_display_primary_r: None,
        mastering_display_primary_g: None,
        mastering_display_primary_b: None,
        mastering_display_white_point: None,
        mastering_display_min_luminance: None,
        mastering_display_max_luminance: None,
        frame_rate: None,
    };
    storage.upsert_edition(&edition).await.expect("edition");

    let schema = build_schema(SchemaContext::new(storage.clone()));
    Fixture {
        schema,
        storage,
        owner,
        other,
        admin,
        edition_id: edition.id,
    }
}

fn user(name: &str, is_admin: bool, created_at: OffsetDateTime) -> User {
    User {
        id: UserId::new(),
        name: name.into(),
        password_hash: "argon2id$placeholder".into(),
        is_admin,
        role: if is_admin {
            noflex_core::UserRole::Admin
        } else {
            noflex_core::UserRole::User
        },
        created_at,
    }
}

async fn create_job(f: &Fixture, user: &User) -> serde_json::Value {
    let mutation = format!(
        r#"mutation {{ createExportJob(editionId: "{}", format: MP4) {{ id userId editionId format status progress }} }}"#,
        f.edition_id
    );
    let response = f.schema.execute(request(mutation, user)).await;
    assert!(response.errors.is_empty(), "create: {:?}", response.errors);
    response.data.into_json().expect("json")["createExportJob"].clone()
}

#[tokio::test]
async fn owner_can_create_list_get_and_cancel_export() {
    let f = fixture().await;
    let job = create_job(&f, &f.owner).await;
    assert_eq!(job["userId"], f.owner.id.to_string());
    assert_eq!(job["editionId"], f.edition_id.to_string());
    assert_eq!(job["status"], "QUEUED");
    let id = job["id"].as_str().expect("job id");

    let response = f
        .schema
        .execute(request("{ exportJobs { nodes { id status } } }", &f.owner))
        .await;
    assert!(response.errors.is_empty(), "list: {:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap()["exportJobs"]["nodes"][0]["id"],
        id
    );

    let query = format!(r#"{{ exportJob(id: "{id}") {{ id status }} }}"#);
    let response = f.schema.execute(request(query, &f.owner)).await;
    assert!(response.errors.is_empty(), "get: {:?}", response.errors);
    assert_eq!(response.data.into_json().unwrap()["exportJob"]["id"], id);

    let cancel = format!(r#"mutation {{ cancelExportJob(id: "{id}") {{ id status }} }}"#);
    let response = f.schema.execute(request(cancel, &f.owner)).await;
    assert!(response.errors.is_empty(), "cancel: {:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap()["cancelExportJob"]["status"],
        "CANCELLED"
    );
    assert_eq!(
        f.storage
            .get_export_job(noflex_core::ExportJobId::from_uuid(
                job["id"].as_str().unwrap().parse().unwrap(),
            ))
            .await
            .expect("stored job")
            .expect("job row")
            .status,
        ExportJobStatus::Cancelled
    );
}

#[tokio::test]
async fn anonymous_export_operations_are_denied() {
    let f = fixture().await;
    for operation in [
        "{ exportJobs { nodes { id } } }".to_string(),
        format!(
            r#"{{ exportJob(id: "{}") {{ id }} }}"#,
            noflex_core::ExportJobId::new()
        ),
        format!(
            r#"mutation {{ createExportJob(editionId: "{}", format: MKV) {{ id }} }}"#,
            f.edition_id
        ),
        format!(
            r#"mutation {{ cancelExportJob(id: "{}") {{ id }} }}"#,
            noflex_core::ExportJobId::new()
        ),
    ] {
        let response = f.schema.execute(operation).await;
        assert!(
            response
                .errors
                .iter()
                .any(|error| error.message == "authentication required"),
            "errors: {:?}",
            response.errors
        );
    }
}

#[tokio::test]
async fn users_and_admin_cannot_access_another_users_jobs() {
    let f = fixture().await;
    let job = create_job(&f, &f.owner).await;
    let id = job["id"].as_str().unwrap();

    for caller in [&f.other, &f.admin] {
        let get = format!(r#"{{ exportJob(id: "{id}") {{ id }} }}"#);
        let response = f.schema.execute(request(get, caller)).await;
        assert!(response.errors.is_empty(), "get: {:?}", response.errors);
        assert!(response.data.into_json().unwrap()["exportJob"].is_null());

        let cancel = format!(r#"mutation {{ cancelExportJob(id: "{id}") {{ id }} }}"#);
        let response = f.schema.execute(request(cancel, caller)).await;
        assert_eq!(response.errors[0].message, "export job not found");

        let response = f
            .schema
            .execute(request("{ exportJobs { nodes { id } } }", caller))
            .await;
        assert!(response.errors.is_empty(), "list: {:?}", response.errors);
        assert!(
            response.data.into_json().unwrap()["exportJobs"]["nodes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn enqueue_requires_edition_access_while_admin_may_export_existing_media() {
    let f = fixture().await;
    let mutation = format!(
        r#"mutation {{ createExportJob(editionId: "{}", format: MKV) {{ id }} }}"#,
        f.edition_id
    );
    let response = f.schema.execute(request(&mutation, &f.other)).await;
    assert_eq!(response.errors[0].message, "edition not found");

    let response = f.schema.execute(request(mutation, &f.admin)).await;
    assert!(
        response.errors.is_empty(),
        "admin create: {:?}",
        response.errors
    );
}

#[tokio::test]
async fn export_subscription_is_owner_gated() {
    let f = fixture().await;
    let job = create_job(&f, &f.owner).await;
    let id = job["id"].as_str().unwrap();
    let query = format!(r#"subscription {{ exportJobStatus(id: "{id}") {{ id }} }}"#);
    let response = f
        .schema
        .execute_stream(request(query, &f.other))
        .next()
        .await
        .expect("subscription response");
    assert_eq!(response.errors[0].message, "export job not found");
}

#[tokio::test]
async fn broadcaster_emits_status_changed_to_subscribers() {
    let bus = ExportEventBroadcaster::new(8);
    let mut receiver = bus.subscribe();
    let id = noflex_core::ExportJobId::new();
    bus.emit_status(id, ExportJobStatus::Running, 0.25, None);
    let event = receiver.recv().await.expect("event delivered");
    let ExportEvent::StatusChanged {
        id: event_id,
        status,
        progress,
        ..
    } = event;
    assert_eq!(event_id, id);
    assert_eq!(status, ExportJobStatus::Running);
    assert!((progress - 0.25).abs() < f32::EPSILON);
}
