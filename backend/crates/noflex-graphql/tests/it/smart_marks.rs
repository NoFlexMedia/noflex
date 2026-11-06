// SPDX-License-Identifier: AGPL-3.0-only
//
// Marks are per viewer and name a title and maybe a device, so it's who can name
// what that matters, along with what a series stands for as it's watched.

use std::sync::Arc;

use async_graphql::{Json, Request};
use noflex_auth::Principal;
use noflex_core::{DeviceId, LibraryId, MetadataStatus, PlayTarget, UserId, WorkId, WorkKind};
use noflex_graphql::{SchemaContext, build_schema, smart::marked_works};
use noflex_storage::{
    RequestUser, Storage,
    domain::{Device, Library, LibraryKind, User, Work},
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
        name: name.into(),
        password_hash: "argon2id$placeholder".into(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: now,
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
