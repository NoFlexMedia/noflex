// SPDX-License-Identifier: AGPL-3.0-only
//
// Coverage for the web-client additions (`Work.images`, `Work.userData`,
// `Work.runtimeMs`, `HistoryRow.work`, `WatchRoom.work`) and for the
// access, paging, and credential-throttling fixes around them.

use std::{collections::HashSet, sync::Arc};

use async_graphql::Request;
use futures::StreamExt;
use noflex_auth::JwtConfig;
use noflex_core::{
    EditionId, ImageKind, LibraryId, MediaFileId, PlayTarget, UserId, WatchRoomId, WorkId, WorkKind,
};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_storage::{
    RequestUser, Storage,
    domain::{
        ActivityKind, Edition, Image, ImageTarget, Library, LibraryConfig, LibraryKind, MediaFile,
        User, WatchRoom, WatchRoomMember, WatchRoomState, Work,
    },
};
use noflex_storage_turso::TursoStore;
use serde_json::Value;
use time::OffsetDateTime;

fn user(name: &str, is_admin: bool) -> User {
    User {
        id: UserId::new(),
        username: name.to_string(),
        display_name: None,
        password_hash: "argon2id$placeholder".to_string(),
        is_admin,
        role: if is_admin {
            noflex_core::UserRole::Admin
        } else {
            noflex_core::UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    }
}

fn library(name: &str) -> Library {
    Library {
        id: LibraryId::new(),
        name: name.to_string(),
        kind: LibraryKind::Movies,
        paths: vec![format!("/srv/{name}")],
        config: LibraryConfig::default(),
    }
}

fn work(library_id: LibraryId, title: &str) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
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

fn edition(work_id: WorkId, runtime_ms: Option<i64>) -> Edition {
    Edition {
        id: EditionId::new(),
        work_id,
        name: "Default".to_string(),
        runtime_ms,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    }
}

fn media(edition_id: EditionId) -> MediaFile {
    MediaFile {
        id: MediaFileId::new(),
        edition_id,
        path: format!("/srv/{edition_id}.mkv"),
        part_index: 0,
        size_bytes: 1024,
        mtime: OffsetDateTime::now_utc(),
        container: Some("matroska".to_string()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: Vec::new(),
    }
}

fn as_user(query: impl Into<String>, user: &User) -> Request {
    Request::new(query).data(RequestUser {
        user_id: user.id,
        is_admin: user.is_admin,
    })
}

fn data(resp: async_graphql::Response) -> Value {
    assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
    resp.data.into_json().expect("json")
}

fn error_contains(resp: &async_graphql::Response, needle: &str) -> bool {
    resp.errors.iter().any(|e| e.message.contains(needle))
}

/// One library the viewer can see (`visible`) and one they can't.
struct Fixture {
    schema: NoflexSchema,
    storage: Arc<dyn Storage>,
    admin: User,
    viewer: User,
    other: User,
    visible: LibraryId,
    hidden: LibraryId,
}

async fn fixture() -> Fixture {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let admin = user("admin", true);
    let viewer = user("viewer", false);
    let other = user("other", false);
    for u in [&admin, &viewer, &other] {
        storage.upsert_user(u).await.expect("user");
    }
    let visible = library("visible");
    let hidden = library("hidden");
    storage.upsert_library(&visible).await.expect("library");
    storage.upsert_library(&hidden).await.expect("library");
    for u in [&viewer, &other] {
        storage
            .grant_library_access(u.id, visible.id)
            .await
            .expect("grant");
    }
    let schema = build_schema(SchemaContext {
        storage: Some(storage.clone()),
        jwt: Some(Arc::new(
            JwtConfig::dev(b"ui-api-test-secret-32-bytes-long!").expect("jwt"),
        )),
        ..Default::default()
    });
    Fixture {
        schema,
        storage,
        admin,
        viewer,
        other,
        visible: visible.id,
        hidden: hidden.id,
    }
}

impl Fixture {
    async fn work(&self, library: LibraryId, title: &str) -> (Work, Edition) {
        let w = work(library, title);
        self.storage.upsert_work(&w).await.expect("work");
        let e = edition(w.id, Some(5_400_000));
        self.storage.upsert_edition(&e).await.expect("edition");
        (w, e)
    }
}

#[tokio::test]
async fn work_images_user_data_and_runtime_resolve() {
    let f = fixture().await;
    let (w, _) = f.work(f.visible, "Arrival").await;
    f.storage
        .upsert_image(&Image {
            target: ImageTarget::Work(w.id),
            kind: ImageKind::Primary,
            source: noflex_storage::domain::ImageSource::Local,
            path: "/srv/poster.jpg".to_string(),
            width: Some(1000),
            height: Some(1500),
            language: None,
            provider: None,
        })
        .await
        .expect("image");
    f.storage
        .record_playback_progress(f.viewer.id, PlayTarget::Work(w.id), 60_000)
        .await
        .expect("progress");

    let query = "{ recentlyAdded(first: 50) { nodes { id images { kind url } userData { positionMs } runtimeMs } } }";
    let json = data(f.schema.execute(as_user(query, &f.viewer)).await);
    let row = &json["recentlyAdded"]["nodes"][0];
    assert_eq!(row["id"], w.id.to_string());
    assert_eq!(row["images"][0]["kind"], "PRIMARY");
    assert!(row["images"][0]["url"].as_str().is_some());
    assert_eq!(row["userData"]["positionMs"], 60_000);
    assert_eq!(row["runtimeMs"], 5_400_000);

    // A different viewer has no play state for the work yet.
    let json = data(f.schema.execute(as_user(query, &f.other)).await);
    assert!(json["recentlyAdded"]["nodes"][0]["userData"]["positionMs"].is_null());
}

#[tokio::test]
async fn work_images_follow_the_library_artwork_preference() {
    use noflex_storage::domain::ImageSource;

    let f = fixture().await;
    let (w, _) = f.work(f.visible, "Dune").await;
    for (source, path) in [
        (ImageSource::Local, "/srv/dune/poster.jpg"),
        (ImageSource::Provider, "/cache/provider-art/dune.jpg"),
    ] {
        f.storage
            .upsert_image(&Image {
                target: ImageTarget::Work(w.id),
                kind: ImageKind::Primary,
                source,
                path: path.to_string(),
                width: None,
                height: None,
                language: None,
                provider: None,
            })
            .await
            .expect("image");
    }
    let query = format!(
        r#"{{ work(id: "{}") {{ images {{ kind source url }} }} }}"#,
        w.id
    );
    let images = |json: Value| json["work"]["images"].clone();

    let got = images(data(f.schema.execute(as_user(&query, &f.viewer)).await));
    assert_eq!(got.as_array().map(Vec::len), Some(1), "one image per kind");
    assert_eq!(got[0]["source"], "PROVIDER", "provider art is the default");
    assert!(
        got[0]["url"]
            .as_str()
            .is_some_and(|u| u.contains("source=provider"))
    );

    let update = format!(
        r#"mutation {{ updateLibraryConfig(id: "{}", artworkPreference: PREFER_LOCAL) {{ config {{ artworkPreference }} }} }}"#,
        f.visible
    );
    let json = data(f.schema.execute(as_user(&update, &f.admin)).await);
    assert_eq!(
        json["updateLibraryConfig"]["config"]["artworkPreference"],
        "PREFER_LOCAL"
    );

    let got = images(data(f.schema.execute(as_user(&query, &f.viewer)).await));
    assert_eq!(got[0]["source"], "LOCAL", "takes effect without a rescan");
    assert!(
        got[0]["url"]
            .as_str()
            .is_some_and(|u| u.contains("source=local"))
    );
}

#[tokio::test]
async fn work_list_with_ui_fields_fits_the_complexity_budget() {
    let f = fixture().await;
    let query = "{ recentlyAdded(first: 50) { nodes { id title sortTitle year overview kind libraryId \
                 images { kind url width height } userData { positionMs played favorite } runtimeMs } } }";
    data(f.schema.execute(as_user(query, &f.viewer)).await);
}

#[tokio::test]
async fn history_row_work_respects_access_and_pages_without_gaps() {
    let f = fixture().await;
    let mut expected = HashSet::new();
    for i in 0..6 {
        let library = if i % 2 == 0 { f.visible } else { f.hidden };
        let (w, e) = f.work(library, &format!("Work {i}")).await;
        // Alternate work and edition targets so both join paths run.
        let target = if i % 4 < 2 {
            PlayTarget::Work(w.id)
        } else {
            PlayTarget::Edition(e.id)
        };
        for user in [f.viewer.id, f.admin.id] {
            f.storage
                .record_playback_progress(user, target, 1_000)
                .await
                .expect("progress");
        }
        if library == f.visible {
            expected.insert(w.id.to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    // Admin bypasses library grants: every row of their own history
    // hydrates.
    let all = "{ playHistory(first: 50) { nodes { work { id } } } }";
    let json = data(f.schema.execute(as_user(all, &f.admin)).await);
    let rows = json["playHistory"]["nodes"].as_array().expect("rows");
    assert_eq!(rows.len(), 6);
    assert!(rows.iter().all(|row| row["work"]["id"].is_string()));

    // The viewer pages two at a time and must see every visible row
    // exactly once, never a hidden one.
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let after = cursor
            .as_deref()
            .map(|c| format!(r#", after: "{c}""#))
            .unwrap_or_default();
        let query = format!(
            r#"{{ playHistory(first: 2{after}) {{ nodes {{ id work {{ id }} }} pageInfo {{ hasNextPage endCursor }} }} }}"#
        );
        let json = data(f.schema.execute(as_user(query, &f.viewer)).await);
        for row in json["playHistory"]["nodes"].as_array().expect("rows") {
            seen.push(
                row["work"]["id"]
                    .as_str()
                    .expect("visible work")
                    .to_string(),
            );
        }
        let page_info = &json["playHistory"]["pageInfo"];
        if page_info["hasNextPage"] != true {
            break;
        }
        cursor = page_info["endCursor"].as_str().map(ToOwned::to_owned);
    }
    assert_eq!(seen.len(), expected.len(), "{seen:?}");
    assert_eq!(seen.into_iter().collect::<HashSet<_>>(), expected);
}

#[tokio::test]
async fn watch_room_work_is_null_when_inaccessible() {
    let f = fixture().await;
    let mut rooms = Vec::new();
    for library in [f.visible, f.hidden] {
        let (w, e) = f.work(library, "Room work").await;
        let m = media(e.id);
        f.storage.upsert_media_file(&m).await.expect("media");
        let now = OffsetDateTime::now_utc();
        let room = WatchRoom {
            id: WatchRoomId::new(),
            host_user_id: f.viewer.id,
            media_file_id: m.id,
            state: WatchRoomState::Waiting,
            position_ms: 0,
            sequence: 0,
            created_at: now,
            updated_at: now,
        };
        f.storage.create_watch_room(&room).await.expect("room");
        f.storage
            .join_watch_room(&WatchRoomMember {
                room_id: room.id,
                user_id: f.viewer.id,
                joined_at: now,
            })
            .await
            .expect("join");
        rooms.push((room.id, w.id));
    }
    for ((room, work), visible) in rooms.into_iter().zip([true, false]) {
        let query = format!(r#"{{ watchRoom(id: "{room}") {{ id work {{ id }} }} }}"#);
        let json = data(f.schema.execute(as_user(query, &f.viewer)).await);
        if visible {
            assert_eq!(json["watchRoom"]["work"]["id"], work.to_string());
        } else {
            assert!(json["watchRoom"]["work"].is_null());
        }
    }
}

#[tokio::test]
async fn activity_is_filtered_for_non_admins() {
    let f = fixture().await;
    let (seen, _) = f.work(f.visible, "Seen").await;
    let (hidden, _) = f.work(f.hidden, "Hidden").await;
    let now = OffsetDateTime::now_utc();
    let me = f.viewer.id.to_string();
    let them = f.other.id.to_string();
    let rows = [
        (ActivityKind::PlayStarted, Some(me.as_str()), Some(seen.id)),
        (
            ActivityKind::PlayStarted,
            Some(them.as_str()),
            Some(seen.id),
        ),
        (
            ActivityKind::PlayStarted,
            Some(me.as_str()),
            Some(hidden.id),
        ),
        (ActivityKind::ItemAdded, Some("scanner"), Some(seen.id)),
        (ActivityKind::ScanCompleted, Some("scanner"), None),
    ];
    for (kind, actor, subject) in rows {
        let subject = subject.map(|id| id.to_string());
        f.storage
            .append_activity(kind, actor, subject.as_deref(), None, now)
            .await
            .expect("append");
    }

    let query = "{ activity(first: 50) { nodes { kind actor subject } } }";
    let json = data(f.schema.execute(as_user(query, &f.admin)).await);
    assert_eq!(json["activity"]["nodes"].as_array().expect("rows").len(), 5);

    let json = data(f.schema.execute(as_user(query, &f.viewer)).await);
    let rows = json["activity"]["nodes"].as_array().expect("rows");
    assert_eq!(rows.len(), 2, "{rows:?}");
    for row in rows {
        assert_eq!(row["subject"], seen.id.to_string());
        match row["kind"].as_str() {
            Some("PLAY_STARTED") => assert_eq!(row["actor"], me),
            Some("ITEM_ADDED") => assert!(row["actor"].is_null()),
            other => panic!("unexpected row kind {other:?}"),
        }
    }
}

#[tokio::test]
async fn metadata_search_and_plugin_lifecycle_require_admin() {
    let f = fixture().await;
    let search = r#"{ searchMetadata(query: "x", kind: MOVIE) { title } }"#;
    let anonymous = f.schema.execute(search).await;
    assert!(error_contains(&anonymous, "authentication required"));
    let viewer = f.schema.execute(as_user(search, &f.viewer)).await;
    assert!(error_contains(&viewer, "administrator required"));

    let lifecycle = "subscription { pluginLifecycle { kind } }";
    let mut stream = f.schema.execute_stream(as_user(lifecycle, &f.viewer));
    let first = stream.next().await.expect("one response");
    assert!(error_contains(&first, "administrator required"));
}

#[tokio::test]
async fn aliased_logins_exceed_the_complexity_budget() {
    let f = fixture().await;
    let three = r#"mutation {
        a: login(username: "admin", password: "1") { token }
        b: login(username: "admin", password: "2") { token }
        c: login(username: "admin", password: "3") { token }
    }"#;
    let resp = f.schema.execute(three).await;
    assert!(error_contains(&resp, "too complex"), "{:?}", resp.errors);
}

#[tokio::test]
async fn repeated_failed_logins_are_throttled() {
    let f = fixture().await;
    let mut real = user("throttled", false);
    real.password_hash = noflex_auth::hash_password("right").await.expect("hash");
    f.storage.upsert_user(&real).await.expect("user");

    let attempt = |password: &str| {
        format!(
            r#"mutation {{ login(username: "throttled", password: "{password}") {{ token }} }}"#
        )
    };
    for _ in 0..6 {
        let resp = f.schema.execute(attempt("wrong")).await;
        assert!(error_contains(&resp, "invalid username or password"));
    }
    // Locked now: even the right password is refused without a check.
    let resp = f.schema.execute(attempt("right")).await;
    assert!(
        error_contains(&resp, "too many failed attempts"),
        "{:?}",
        resp.errors
    );
}

#[tokio::test]
async fn watch_room_stream_ends_after_subscriber_leaves() {
    let f = fixture().await;
    let (_, e) = f.work(f.visible, "Shared").await;
    let m = media(e.id);
    f.storage.upsert_media_file(&m).await.expect("media");
    let created = data(
        f.schema
            .execute(as_user(
                format!(
                    r#"mutation {{ createWatchRoom(mediaFileId: "{}") {{ id }} }}"#,
                    m.id
                ),
                &f.viewer,
            ))
            .await,
    );
    let room = created["createWatchRoom"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    for u in [&f.viewer, &f.other] {
        data(
            f.schema
                .execute(as_user(
                    format!(r#"mutation {{ joinWatchRoom(roomId: "{room}") {{ id }} }}"#),
                    u,
                ))
                .await,
        );
    }
    let control = |action: &str| {
        format!(r#"mutation {{ controlWatchRoom(roomId: "{room}", action: {action}) {{ id }} }}"#)
    };

    let mut stream = f.schema.execute_stream(as_user(
        format!(r#"subscription {{ watchRoomEvents(roomId: "{room}") {{ action }} }}"#),
        &f.other,
    ));
    // Let the subscription register before the first event.
    let first = tokio::spawn({
        let schema = f.schema.clone();
        let query = control("PLAY");
        let host = f.viewer.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            data(schema.execute(as_user(query, &host)).await);
        }
    });
    let event = stream.next().await.expect("first event");
    assert_eq!(data(event)["watchRoomEvents"]["action"], "PLAY");
    first.await.expect("control task");

    data(
        f.schema
            .execute(as_user(
                format!(r#"mutation {{ leaveWatchRoom(roomId: "{room}") }}"#),
                &f.other,
            ))
            .await,
    );
    data(f.schema.execute(as_user(control("PAUSE"), &f.viewer)).await);
    let next = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .expect("stream resolves");
    assert!(next.is_none(), "former member got {next:?}");
}
