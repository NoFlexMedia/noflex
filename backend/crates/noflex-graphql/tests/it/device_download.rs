// SPDX-License-Identifier: AGPL-3.0-only
//
// `deviceDownload` hands out a token-bearing URL for a file the caller names,
// so it's the access checks that matter: the device and the title must both be
// the caller's to use.

use std::sync::Arc;

use async_graphql::{Json, Request};
use noflex_auth::Principal;
use noflex_core::{
    DeviceId, EditionId, LibraryId, MediaFileId, MetadataStatus, UserId, WorkId, WorkKind,
};
use noflex_graphql::{
    PlaybackSession, PlaybackSessionService, SchemaContext, SmartDownloadFile, build_schema,
};
use noflex_storage::{
    RequestUser, Storage,
    domain::{Device, Edition, Library, LibraryKind, MediaFile, User, Work},
};
use noflex_storage_turso::TursoStore;
use noflex_transcoder::ClientCapabilities;
use time::OffsetDateTime;

/// Everything plays directly, unless told nothing is ready.
struct Files {
    ready: bool,
}

#[async_trait::async_trait]
impl PlaybackSessionService for Files {
    async fn start(
        &self,
        _: UserId,
        _: Option<noflex_core::SessionId>,
        _: noflex_graphql::PlaybackStart,
    ) -> anyhow::Result<PlaybackSession> {
        anyhow::bail!("unused")
    }

    async fn stop(&self, _: UserId, _: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn active_streams(&self) -> anyhow::Result<Vec<noflex_graphql::ActiveStreamRecord>> {
        Ok(Vec::new())
    }

    async fn smart_download(
        &self,
        _: UserId,
        _: Option<noflex_core::SessionId>,
        media: &MediaFile,
        _: &ClientCapabilities,
    ) -> anyhow::Result<Option<SmartDownloadFile>> {
        Ok(self.ready.then(|| SmartDownloadFile {
            url: format!("/stream/{}/file?token=t", media.id),
            size_bytes: media.size_bytes,
            optimized: false,
            subtitles: Vec::new(),
        }))
    }
}

struct Fixture {
    storage: Arc<dyn Storage>,
    viewer: User,
    stranger: User,
    device: DeviceId,
    media: MediaFileId,
    library: LibraryId,
}

fn user(name: &str, now: OffsetDateTime) -> User {
    User {
        id: UserId::new(),
        name: name.into(),
        password_hash: String::new(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: now,
    }
}

async fn fixture() -> Fixture {
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
    let work = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Film".into(),
        sort_title: "film".into(),
        year: None,
        release_date: None,
        overview: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };
    storage.upsert_work(&work).await.expect("work");
    let edition = Edition {
        id: EditionId::new(),
        work_id: work.id,
        name: "Theatrical".into(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    storage.upsert_edition(&edition).await.expect("edition");
    let media = MediaFile {
        id: MediaFileId::new(),
        edition_id: edition.id,
        path: "/srv/films/film.mkv".into(),
        part_index: 0,
        size_bytes: 1_000,
        mtime: now,
        container: Some("matroska".into()),
        probe: Json(serde_json::json!({})),
        subtitle_tracks: vec![],
    };
    storage.upsert_media_file(&media).await.expect("media");
    storage
        .grant_library_access(viewer.id, library.id)
        .await
        .expect("grant");
    let device = DeviceId::new();
    storage
        .upsert_device(&Device {
            id: device,
            user_id: viewer.id,
            name: "Phone".into(),
            capabilities: Json(serde_json::to_value(ClientCapabilities::web()).expect("caps")),
            smart_transcode: false,
            max_bitrate_bps: None,
            allowed_video_codecs: vec![],
            smart_downloads: false,
            download_budget_bytes: None,
            downloads_wifi_only: true,
            created_at: now,
            last_seen_at: now,
        })
        .await
        .expect("device");
    Fixture {
        storage,
        viewer,
        stranger,
        device,
        media: media.id,
        library: library.id,
    }
}

fn request(fixture: &Fixture, as_user: &User, device: DeviceId) -> Request {
    let query = format!(
        r#"{{ deviceDownload(deviceId: "{device}", mediaFileId: "{}") {{ url sizeBytes partIndex work {{ title }} }} }}"#,
        fixture.media
    );
    Request::new(query)
        .data(Principal {
            user_id: as_user.id,
            is_admin: false,
            session_id: None,
        })
        .data(RequestUser {
            user_id: as_user.id,
            is_admin: false,
        })
}

fn schema(fixture: &Fixture, ready: bool) -> noflex_graphql::NoflexSchema {
    build_schema(SchemaContext {
        storage: Some(fixture.storage.clone()),
        playback_sessions: Some(Arc::new(Files { ready })),
        ..Default::default()
    })
}

#[tokio::test]
async fn answers_with_smart_downloads_off_and_null_when_nothing_is_ready() {
    let fixture = fixture().await;
    let resp = schema(&fixture, true)
        .execute(request(&fixture, &fixture.viewer, fixture.device))
        .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    let data = resp.data.into_json().expect("json");
    assert_eq!(data["deviceDownload"]["work"]["title"], "Film");
    assert_eq!(data["deviceDownload"]["sizeBytes"], 1_000);

    let resp = schema(&fixture, false)
        .execute(request(&fixture, &fixture.viewer, fixture.device))
        .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    assert!(resp.data.into_json().expect("json")["deviceDownload"].is_null());
}

#[tokio::test]
async fn refuses_someone_elses_device_and_a_title_outside_their_libraries() {
    let fixture = fixture().await;
    let schema = schema(&fixture, true);

    let resp = schema
        .execute(request(&fixture, &fixture.stranger, fixture.device))
        .await;
    assert_eq!(resp.errors[0].message, "device not found");

    fixture
        .storage
        .revoke_library_access(fixture.viewer.id, fixture.library)
        .await
        .expect("revoke");
    let resp = schema
        .execute(request(&fixture, &fixture.viewer, fixture.device))
        .await;
    assert_eq!(resp.errors[0].message, "media file not found");
}

#[tokio::test]
async fn only_an_administrator_turns_downloads_off_and_then_none_are_offered() {
    let fixture = fixture().await;
    let schema = schema(&fixture, true);
    let as_user = |query: &str, is_admin: bool| {
        Request::new(query)
            .data(Principal {
                user_id: fixture.viewer.id,
                is_admin,
                session_id: None,
            })
            .data(RequestUser {
                user_id: fixture.viewer.id,
                is_admin,
            })
    };
    let turn_off = "mutation { setDownloadsAllowed(allowed: false) }";

    let refused = schema.execute(as_user(turn_off, false)).await;
    assert_eq!(refused.errors[0].message, "administrator required");
    let allowed = schema.execute("{ serverInfo { downloadsAllowed } }").await;
    assert_eq!(
        allowed.data.into_json().expect("json")["serverInfo"]["downloadsAllowed"],
        true
    );

    let done = schema.execute(as_user(turn_off, true)).await;
    assert!(done.errors.is_empty(), "{:?}", done.errors);
    let info = schema.execute("{ serverInfo { downloadsAllowed } }").await;
    assert_eq!(
        info.data.into_json().expect("json")["serverInfo"]["downloadsAllowed"],
        false
    );

    let resp = schema
        .execute(request(&fixture, &fixture.viewer, fixture.device))
        .await;
    assert_eq!(
        resp.errors[0].message,
        "This server doesn’t allow downloads."
    );
    let code = resp.errors[0]
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("code"));
    assert_eq!(
        code,
        Some(&async_graphql::Value::from("DOWNLOADS_NOT_ALLOWED"))
    );
}
