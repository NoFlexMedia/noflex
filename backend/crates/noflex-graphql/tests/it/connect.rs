// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex Connect hands a command to another of the viewer's own devices, so
// the checks that matter are whose devices and which titles, and that a
// listening device is what makes one reachable.

use std::{sync::Arc, time::Duration};

use async_graphql::{Json, Request};
use futures::StreamExt;
use noflex_auth::Principal;
use noflex_core::{DeviceId, LibraryId, MetadataStatus, UserId, WorkId, WorkKind};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_storage::{
    RequestUser, Storage,
    domain::{Device, Library, LibraryKind, User, Work},
};
use noflex_storage_turso::TursoStore;
use noflex_transcoder::ClientCapabilities;
use time::OffsetDateTime;

struct Fixture {
    storage: Arc<dyn Storage>,
    schema: NoflexSchema,
    viewer: User,
    stranger: User,
    tv: DeviceId,
    work: WorkId,
    library: LibraryId,
}

fn user(name: &str, now: OffsetDateTime) -> User {
    User {
        id: UserId::new(),
        username: name.into(),
        display_name: None,
        password_hash: String::new(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: now,
        avatar_updated_at: None,
        has_pin: false,
    }
}

fn device(owner: &User, name: &str, now: OffsetDateTime) -> Device {
    Device {
        id: DeviceId::new(),
        user_id: owner.id,
        name: name.into(),
        client: None,
        capabilities: Json(serde_json::to_value(ClientCapabilities::web()).expect("caps")),
        smart_transcode: false,
        max_bitrate_bps: None,
        allowed_video_codecs: vec![],
        smart_downloads: false,
        download_budget_bytes: None,
        downloads_wifi_only: true,
        created_at: now,
        last_seen_at: now,
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
    storage
        .grant_library_access(viewer.id, library.id)
        .await
        .expect("grant");
    let tv = device(&viewer, "Living room TV", now);
    storage.upsert_device(&tv).await.expect("tv");
    storage
        .upsert_device(&device(&stranger, "Their TV", now))
        .await
        .expect("stranger's device");
    let schema = build_schema(SchemaContext::new(storage.clone()));
    Fixture {
        storage,
        schema,
        viewer,
        stranger,
        tv: tv.id,
        work: work.id,
        library: library.id,
    }
}

fn request(query: impl Into<String>, as_user: &User) -> Request {
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

fn cast(fixture: &Fixture) -> String {
    format!(
        r#"mutation {{ castToDevice(deviceId: "{}", workId: "{}", positionMs: 90000) {{ online playback {{ state }} }} }}"#,
        fixture.tv, fixture.work
    )
}

#[tokio::test]
async fn a_cast_reaches_a_listening_device_and_nothing_else_can_send_to_it() {
    let fixture = fixture().await;
    let schema = &fixture.schema;

    let offline = schema
        .execute(request(cast(&fixture), &fixture.viewer))
        .await;
    assert_eq!(
        offline.errors[0].message,
        "That device isn’t available right now."
    );
    assert_eq!(
        serde_json::to_value(&offline.errors[0].extensions).expect("extensions")["code"],
        "DEVICE_OFFLINE"
    );

    let listen = format!(
        r#"subscription {{ deviceCommands(deviceId: "{}") {{ kind workId positionMs }} }}"#,
        fixture.tv
    );
    let mut commands = schema.execute_stream(request(listen.clone(), &fixture.viewer));
    let sender = schema.clone();
    let viewer = fixture.viewer.clone();
    let query = cast(&fixture);
    let (sent_tx, sent_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = sent_tx.send(sender.execute(request(query, &viewer)).await);
    });
    let command = tokio::time::timeout(Duration::from_secs(2), commands.next())
        .await
        .expect("command timed out")
        .expect("subscription ended");
    let sent = sent_rx.await.expect("cast response");
    assert!(sent.errors.is_empty(), "{:?}", sent.errors);
    let sent = sent.data.into_json().expect("json");
    assert_eq!(sent["castToDevice"]["online"], true);
    // The controller has something to show before the device has said a word.
    assert_eq!(sent["castToDevice"]["playback"]["state"], "BUFFERING");
    let command = command.data.into_json().expect("json");
    assert_eq!(command["deviceCommands"]["kind"], "LOAD");
    assert_eq!(
        command["deviceCommands"]["workId"],
        fixture.work.to_string()
    );
    assert_eq!(command["deviceCommands"]["positionMs"], 90_000);

    // What the device says about itself shows up in the list.
    let report = format!(
        r#"mutation {{ reportDeviceState(deviceId: "{}", playback: {{ workId: "{}", state: PLAYING, positionMs: 91000, durationMs: 600000 }}) }}"#,
        fixture.tv, fixture.work
    );
    let reported = schema.execute(request(report, &fixture.viewer)).await;
    assert!(reported.errors.is_empty(), "{:?}", reported.errors);
    let listed = schema
        .execute(request(
            "{ connectDevices { online device { name } playback { state positionMs work { title } } } }",
            &fixture.viewer,
        ))
        .await;
    assert!(listed.errors.is_empty(), "{:?}", listed.errors);
    let tv = &listed.data.into_json().expect("json")["connectDevices"][0];
    assert_eq!(tv["device"]["name"], "Living room TV");
    assert_eq!(tv["online"], true);
    assert_eq!(tv["playback"]["state"], "PLAYING");
    assert_eq!(tv["playback"]["work"]["title"], "Film");

    // Stopping it shows it idle in the very response, not at the device's next report.
    let stopped = schema
        .execute(request(
            format!(r#"mutation {{ controlDevice(deviceId: "{}", action: STOP) {{ online playback {{ state }} }} }}"#, fixture.tv),
            &fixture.viewer,
        ))
        .await;
    assert!(stopped.errors.is_empty(), "{:?}", stopped.errors);
    let stopped = stopped.data.into_json().expect("json");
    assert_eq!(stopped["controlDevice"]["online"], true);
    assert!(stopped["controlDevice"]["playback"].is_null(), "{stopped}");

    // Someone else's device is not found, whether to listen, send or report.
    let spy = schema
        .execute_stream(request(listen, &fixture.stranger))
        .next()
        .await
        .expect("a response");
    assert_eq!(spy.errors[0].message, "device not found");
    let forged = schema
        .execute(request(cast(&fixture), &fixture.stranger))
        .await;
    assert_eq!(forged.errors[0].message, "device not found");
    let theirs = schema
        .execute(request(
            "{ connectDevices { device { name } } }",
            &fixture.stranger,
        ))
        .await;
    assert_eq!(
        theirs.data.into_json().expect("json")["connectDevices"]
            .as_array()
            .map(Vec::len),
        Some(1),
        "a viewer sees only their own devices"
    );
}

#[tokio::test]
async fn a_title_outside_the_viewers_libraries_cannot_be_cast() {
    let fixture = fixture().await;
    fixture
        .storage
        .revoke_library_access(fixture.viewer.id, fixture.library)
        .await
        .expect("revoke");
    let refused = fixture
        .schema
        .execute(request(cast(&fixture), &fixture.viewer))
        .await;
    assert_eq!(refused.errors[0].message, "title not found");
}
