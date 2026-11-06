// SPDX-License-Identifier: AGPL-3.0-only
//
// Marks are per viewer and name a title and maybe a device, so it's who can name
// what that matters, along with what a series stands for as it's watched.

use std::sync::Arc;

use async_graphql::{Json, Request};
use noflex_auth::Principal;
use noflex_core::{
    DeviceId, EditionId, LibraryId, MediaFileId, MetadataStatus, OptimizedVersionId, PlayTarget,
    UserId, WorkId, WorkKind,
};
use noflex_graphql::{SchemaContext, build_schema, smart::marked_works};
use noflex_storage::{
    RequestUser, Storage,
    domain::{
        Device, Edition, Library, LibraryKind, MediaFile, OptimizedVersion, OptimizedVersionStatus,
        SmartTranscodeMark, SmartTranscodeOutcome, User, Work,
    },
};
use noflex_storage_turso::TursoStore;
use time::{Duration, OffsetDateTime};
use tokio::sync::Notify;

struct Fixture {
    storage: Arc<dyn Storage>,
    viewer: User,
    stranger: User,
    library: LibraryId,
    film: WorkId,
    wake: Arc<Notify>,
}

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
    }
}

fn work(library: LibraryId, kind: WorkKind, title: &str, now: OffsetDateTime) -> Work {
    Work {
        id: WorkId::new(),
        kind,
        parent_id: None,
        library_id: library,
        title: title.into(),
        sort_title: title.to_lowercase(),
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
    storage
        .grant_library_access(viewer.id, library.id)
        .await
        .expect("grant");
    let film = work(library.id, WorkKind::Movie, "Film", now);
    storage.upsert_work(&film).await.expect("film");
    Fixture {
        storage,
        viewer,
        stranger,
        library: library.id,
        film: film.id,
        wake: Arc::new(Notify::new()),
    }
}

async fn device_of(fixture: &Fixture, owner: &User) -> DeviceId {
    let now = OffsetDateTime::now_utc();
    let id = DeviceId::new();
    fixture
        .storage
        .upsert_device(&Device {
            id,
            user_id: owner.id,
            name: "TV".into(),
            capabilities: Json(serde_json::json!({})),
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
    id
}

async fn run(fixture: &Fixture, as_user: &User, query: String) -> async_graphql::Response {
    build_schema(SchemaContext {
        storage: Some(fixture.storage.clone()),
        smart_transcode_wake: Some(fixture.wake.clone()),
        ..Default::default()
    })
    .execute(
        Request::new(query)
            .data(Principal {
                user_id: as_user.id,
                is_admin: false,
                session_id: None,
            })
            .data(RequestUser {
                user_id: as_user.id,
                is_admin: false,
            }),
    )
    .await
}

async fn run_as_admin(fixture: &Fixture, query: String) -> async_graphql::Response {
    let admin = fixture.viewer.id;
    build_schema(SchemaContext {
        storage: Some(fixture.storage.clone()),
        smart_transcode_wake: Some(fixture.wake.clone()),
        ..Default::default()
    })
    .execute(
        Request::new(query)
            .data(Principal {
                user_id: admin,
                is_admin: true,
                session_id: None,
            })
            .data(RequestUser {
                user_id: admin,
                is_admin: true,
            }),
    )
    .await
}

const FIELDS: &str = "work { title } device { name } state progress";

fn marked(work: WorkId, device: Option<DeviceId>) -> String {
    let device = device.map_or(String::new(), |id| format!(r#", deviceId: "{id}""#));
    format!(r#"mutation {{ markForSmartTranscoding(workId: "{work}"{device}) {{ {FIELDS} }} }}"#)
}

#[tokio::test]
async fn marks_are_per_scope_wake_the_worker_and_come_back_when_removed() {
    let fixture = fixture().await;
    let tv = device_of(&fixture, &fixture.viewer).await;

    let resp = run(&fixture, &fixture.viewer, marked(fixture.film, None)).await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    let data = resp.data.into_json().expect("json");
    assert_eq!(data["markForSmartTranscoding"]["state"], "WAITING");
    assert!(data["markForSmartTranscoding"]["device"].is_null());
    tokio::time::timeout(std::time::Duration::from_secs(1), fixture.wake.notified())
        .await
        .expect("the worker hears about it at once");

    let resp = run(&fixture, &fixture.viewer, marked(fixture.film, Some(tv))).await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);

    let query = format!(
        r#"{{ mySmartTranscodeMarks(workId: "{}") {{ {FIELDS} }} all: mySmartTranscodeMarks {{ work {{ title }} }} }}"#,
        fixture.film
    );
    let data = run(&fixture, &fixture.viewer, query.clone())
        .await
        .data
        .into_json()
        .expect("json");
    assert_eq!(data["mySmartTranscodeMarks"].as_array().unwrap().len(), 2);
    assert_eq!(data["all"].as_array().unwrap().len(), 2);
    assert_eq!(data["mySmartTranscodeMarks"][1]["device"]["name"], "TV");

    // Unmarking one scope leaves the other.
    let unmark = format!(
        r#"mutation {{ unmarkForSmartTranscoding(workId: "{}", deviceId: "{tv}") {{ device {{ name }} }} }}"#,
        fixture.film
    );
    let data = run(&fixture, &fixture.viewer, unmark.clone())
        .await
        .data
        .into_json()
        .expect("json");
    assert_eq!(data["unmarkForSmartTranscoding"]["device"]["name"], "TV");
    let data = run(&fixture, &fixture.viewer, unmark)
        .await
        .data
        .into_json()
        .expect("json");
    assert!(
        data["unmarkForSmartTranscoding"].is_null(),
        "nothing left to remove"
    );
    let data = run(&fixture, &fixture.viewer, query)
        .await
        .data
        .into_json()
        .expect("json");
    assert_eq!(data["mySmartTranscodeMarks"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn refuses_a_title_out_of_reach_someone_elses_device_and_what_never_transcodes() {
    let fixture = fixture().await;
    let theirs = device_of(&fixture, &fixture.stranger).await;

    let resp = run(&fixture, &fixture.stranger, marked(fixture.film, None)).await;
    assert_eq!(resp.errors[0].message, "work not found");
    let resp = run(
        &fixture,
        &fixture.viewer,
        marked(fixture.film, Some(theirs)),
    )
    .await;
    assert_eq!(resp.errors[0].message, "device not found");

    let now = OffsetDateTime::now_utc();
    let song = work(fixture.library, WorkKind::Track, "Song", now);
    fixture.storage.upsert_work(&song).await.expect("song");
    let resp = run(&fixture, &fixture.viewer, marked(song.id, None)).await;
    assert_eq!(
        resp.errors[0].message,
        "Only films and shows can be smart transcoded"
    );

    assert!(
        fixture
            .storage
            .smart_transcode_marks_for_user(fixture.viewer.id)
            .await
            .expect("marks")
            .is_empty()
    );
}

#[tokio::test]
async fn a_marked_series_moves_on_as_its_watched_and_a_season_stays_in_itself() {
    let fixture = fixture().await;
    let now = OffsetDateTime::now_utc();
    let storage = fixture.storage.as_ref();
    let series = work(fixture.library, WorkKind::Series, "Show", now);
    storage.upsert_work(&series).await.expect("series");
    let mut seasons = Vec::new();
    let mut episodes = Vec::new();
    for season_number in 1..=2 {
        let mut season = work(fixture.library, WorkKind::Season, "Season", now);
        season.parent_id = Some(series.id);
        season.index_number = Some(season_number);
        storage.upsert_work(&season).await.expect("season");
        for number in 1..=3 {
            let mut episode = work(
                fixture.library,
                WorkKind::Episode,
                &format!("E{number}"),
                now,
            );
            episode.parent_id = Some(season.id);
            episode.parent_index = Some(season_number);
            episode.index_number = Some(number);
            storage.upsert_work(&episode).await.expect("episode");
            episodes.push(episode);
        }
        seasons.push(season);
    }
    let viewer = fixture.viewer.id;
    let ids = |works: Vec<Work>| works.into_iter().map(|work| work.id).collect::<Vec<_>>();

    let marked_at = now - Duration::hours(1);
    let next = marked_works(storage, viewer, &series, marked_at, 1)
        .await
        .expect("next");
    assert_eq!(ids(next), [episodes[0].id, episodes[1].id]);

    // Seen before the mark: still wanted. Seen since: moved past.
    storage
        .apply_watch_state(
            viewer,
            PlayTarget::Work(episodes[0].id),
            true,
            None,
            marked_at - Duration::hours(1),
        )
        .await
        .expect("before");
    storage
        .apply_watch_state(viewer, PlayTarget::Work(episodes[1].id), true, None, now)
        .await
        .expect("since");
    let next = marked_works(storage, viewer, &series, marked_at, 1)
        .await
        .expect("next");
    assert_eq!(ids(next), [episodes[0].id, episodes[2].id]);

    // A season stops at its own end, and a series carries on past it.
    let first_season = marked_works(storage, viewer, &seasons[0], marked_at, 9)
        .await
        .expect("season");
    assert_eq!(ids(first_season), [episodes[0].id, episodes[2].id]);
    let second_season = marked_works(storage, viewer, &seasons[1], marked_at, 0)
        .await
        .expect("season");
    assert_eq!(ids(second_season), [episodes[3].id]);

    // Nothing left to prepare, which is what ends a mark.
    for episode in &episodes {
        storage
            .apply_watch_state(viewer, PlayTarget::Work(episode.id), true, None, now)
            .await
            .expect("played");
    }
    assert!(
        marked_works(storage, viewer, &series, marked_at, 1)
            .await
            .expect("next")
            .is_empty()
    );
}

#[tokio::test]
async fn prepared_versions_list_ready_titles_a_viewer_can_see() {
    let fixture = fixture().await;
    let now = OffsetDateTime::now_utc();
    let edition = Edition {
        id: EditionId::new(),
        work_id: fixture.film,
        name: "Theatrical".into(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    fixture
        .storage
        .upsert_edition(&edition)
        .await
        .expect("edition");
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
    fixture
        .storage
        .upsert_media_file(&media)
        .await
        .expect("media");
    let version = |key: &str| OptimizedVersion {
        id: OptimizedVersionId::new(),
        media_file_id: media.id,
        variant_key: key.to_string(),
        status: OptimizedVersionStatus::Queued,
        priority: 0,
        plan: serde_json::json!({}),
        source_size: 1_000,
        source_mtime: now,
        progress: 0.0,
        attempts: 0,
        path: None,
        size_bytes: None,
        index: None,
        probe: None,
        error: None,
        created_at: now,
        finished_at: None,
        last_used_at: None,
        unwanted_since: None,
    };
    let queued = version("av1-720p-1556k-av1main/aac2");
    let mut ready = version("copy-h264/aac2");
    fixture
        .storage
        .enqueue_optimized_version(&queued)
        .await
        .expect("queued");
    fixture
        .storage
        .enqueue_optimized_version(&ready)
        .await
        .expect("ready");
    ready.status = OptimizedVersionStatus::Ready;
    ready.size_bytes = Some(500);
    ready.finished_at = Some(now);
    fixture
        .storage
        .update_optimized_version(&ready)
        .await
        .expect("finish");

    let query = "{ preparedVersions { description sizeBytes work { title } } }".to_string();
    let seen = run(&fixture, &fixture.viewer, query.clone()).await;
    assert!(seen.errors.is_empty(), "{:?}", seen.errors);
    let seen = seen.data.into_json().expect("json");
    assert_eq!(
        seen["preparedVersions"],
        serde_json::json!([{
            "description": "Video kept, audio converted to AAC stereo",
            "sizeBytes": 500,
            "work": { "title": "Film" },
        }])
    );

    let hidden = run(&fixture, &fixture.stranger, query).await;
    assert!(hidden.errors.is_empty(), "{:?}", hidden.errors);
    let hidden = hidden.data.into_json().expect("json");
    assert_eq!(hidden["preparedVersions"], serde_json::json!([]));
}

#[tokio::test]
async fn an_administrator_sees_who_a_version_is_for_and_can_clear_it() {
    let fixture = fixture().await;
    let now = OffsetDateTime::now_utc();
    let edition = Edition {
        id: EditionId::new(),
        work_id: fixture.film,
        name: "Theatrical".into(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    fixture
        .storage
        .upsert_edition(&edition)
        .await
        .expect("edition");
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
    fixture
        .storage
        .upsert_media_file(&media)
        .await
        .expect("media");
    // Two viewers: one has marked it for one of two devices, the other's is predicted.
    let (tv, phone) = (
        device_of(&fixture, &fixture.viewer).await,
        device_of(&fixture, &fixture.viewer).await,
    );
    let other = device_of(&fixture, &fixture.stranger).await;
    let mut predicted = fixture
        .storage
        .get_device(other)
        .await
        .expect("get")
        .expect("device");
    predicted.smart_transcode = true;
    fixture
        .storage
        .upsert_device(&predicted)
        .await
        .expect("device");
    let version = OptimizedVersion {
        id: OptimizedVersionId::new(),
        media_file_id: media.id,
        variant_key: "copy-h264/aac2".into(),
        status: OptimizedVersionStatus::Queued,
        priority: 0,
        plan: serde_json::json!({ "devices": [tv, phone, other] }),
        source_size: 1_000,
        source_mtime: now,
        progress: 0.0,
        attempts: 0,
        path: None,
        size_bytes: None,
        index: None,
        probe: None,
        error: None,
        created_at: now,
        finished_at: None,
        last_used_at: None,
        unwanted_since: None,
    };
    fixture
        .storage
        .enqueue_optimized_version(&version)
        .await
        .expect("enqueue");
    let mark = fixture
        .storage
        .add_smart_transcode_mark(&SmartTranscodeMark {
            user_id: fixture.viewer.id,
            work_id: fixture.film,
            device_id: Some(tv),
            created_at: now,
            outcome: None,
            media_file_ids: vec![],
        })
        .await
        .expect("mark");
    fixture
        .storage
        .record_smart_transcode_plan(&mark, SmartTranscodeOutcome::Planned, &[media.id])
        .await
        .expect("plan");

    let detail = format!(
        r#"{{ optimizedVersion(id: "{}") {{
            targets {{ user {{ name }} reason device {{ id }} }}
            marks {{ user {{ name }} device {{ id }} work {{ title }} }}
        }} }}"#,
        version.id
    );
    let denied = run(&fixture, &fixture.viewer, detail.clone()).await;
    assert!(!denied.errors.is_empty(), "administrators only");

    let seen = run_as_admin(&fixture, detail).await;
    assert!(seen.errors.is_empty(), "{:?}", seen.errors);
    let seen = seen.data.into_json().expect("json");
    let targets = &seen["optimizedVersion"]["targets"];
    let reason_of = |device: DeviceId| {
        targets
            .as_array()
            .expect("targets")
            .iter()
            .find(|target| target["device"]["id"] == device.to_string())
            .map(|target| (target["user"]["name"].clone(), target["reason"].clone()))
            .expect("target")
    };
    assert_eq!(
        reason_of(tv),
        (serde_json::json!("viewer"), serde_json::json!("MARKED"))
    );
    assert_eq!(
        reason_of(phone),
        (serde_json::json!("viewer"), serde_json::json!("EARLIER"))
    );
    assert_eq!(
        reason_of(other),
        (
            serde_json::json!("stranger"),
            serde_json::json!("PREDICTED")
        )
    );
    assert_eq!(
        seen["optimizedVersion"]["marks"][0]["user"]["name"],
        "viewer"
    );
    assert_eq!(
        seen["optimizedVersion"]["marks"][0]["work"]["title"],
        "Film"
    );

    let unmark = format!(
        r#"mutation {{ unmarkViewerForSmartTranscoding(userId: "{}", workId: "{}", deviceId: "{tv}") {{ work {{ title }} }} }}"#,
        fixture.viewer.id, fixture.film
    );
    assert!(
        !run(&fixture, &fixture.stranger, unmark.clone())
            .await
            .errors
            .is_empty()
    );
    let unmarked = run_as_admin(&fixture, unmark).await;
    assert!(unmarked.errors.is_empty(), "{:?}", unmarked.errors);
    assert_eq!(
        unmarked.data.into_json().expect("json")["unmarkViewerForSmartTranscoding"]["work"]["title"],
        "Film"
    );
    assert!(
        fixture
            .storage
            .list_smart_transcode_marks()
            .await
            .expect("marks")
            .is_empty()
    );

    let remove = format!(
        r#"mutation {{ removeOptimizedVersion(id: "{}") {{ id }} }}"#,
        version.id
    );
    assert!(
        !run(&fixture, &fixture.viewer, remove.clone())
            .await
            .errors
            .is_empty()
    );
    let removed = run_as_admin(&fixture, remove).await;
    assert!(removed.errors.is_empty(), "{:?}", removed.errors);
    assert!(
        fixture
            .storage
            .get_optimized_version(version.id)
            .await
            .expect("get")
            .is_none()
    );
}

#[tokio::test]
async fn an_administrator_can_stop_what_a_viewers_device_has_predicted_for_it() {
    let fixture = fixture().await;
    let phone = device_of(&fixture, &fixture.viewer).await;
    let mut wanting = fixture
        .storage
        .get_device(phone)
        .await
        .expect("get")
        .expect("device");
    wanting.smart_transcode = true;
    wanting.smart_downloads = true;
    fixture
        .storage
        .upsert_device(&wanting)
        .await
        .expect("device");
    let turn = |enabled: bool| {
        format!(
            r#"mutation {{ setViewerDeviceSmartTranscode(id: "{phone}", enabled: {enabled}) {{ smartTranscode smartDownloads }} }}"#
        )
    };
    let list = format!(
        r#"{{ userDevices(userId: "{}") {{ id name smartTranscode }} }}"#,
        fixture.viewer.id
    );

    assert!(
        !run(&fixture, &fixture.stranger, list.clone())
            .await
            .errors
            .is_empty()
    );
    assert!(
        !run(&fixture, &fixture.stranger, turn(false))
            .await
            .errors
            .is_empty(),
        "nor can anyone else reach into an account"
    );

    let listed = run_as_admin(&fixture, list).await;
    assert!(listed.errors.is_empty(), "{:?}", listed.errors);
    assert_eq!(
        listed.data.into_json().expect("json")["userDevices"][0]["id"],
        phone.to_string()
    );

    let off = run_as_admin(&fixture, turn(false)).await;
    assert!(off.errors.is_empty(), "{:?}", off.errors);
    assert!(
        fixture
            .storage
            .devices_wanting_versions()
            .await
            .expect("devices")
            .is_empty(),
        "smart downloads ask for the same predictions, so they end too"
    );

    let on = run_as_admin(&fixture, turn(true)).await;
    assert!(on.errors.is_empty(), "{:?}", on.errors);
    let stored = fixture
        .storage
        .get_device(phone)
        .await
        .expect("get")
        .expect("device");
    assert!(stored.smart_transcode);
    assert_eq!(stored.user_id, fixture.viewer.id, "still the viewer's");
}
