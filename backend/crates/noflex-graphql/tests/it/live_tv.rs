// SPDX-License-Identifier: AGPL-3.0-only
//
// E8 acceptance coverage for the GraphQL Live TV and DVR surface. The
// service is fake so these tests exercise resolver authorization and the
// Turso storage contract without starting an HTTP ingest or ffmpeg worker.

use std::sync::Arc;

use async_graphql::{Request, Response};
use noflex_auth::Principal;
use noflex_core::{
    ChannelId, EpgProgramId, LiveTvSourceId, RecordingId, RecordingScheduleId, UserId,
};
use noflex_graphql::{
    LiveTvService, LiveTvServiceError, NoflexSchema, PlaybackRouteKind, PlaybackSession,
    SchemaContext, build_schema,
};
use noflex_storage::{
    RequestUser, Storage,
    domain::{
        EpgProgram, LiveTvChannel, LiveTvSource, Recording, RecordingSchedule,
        RecordingScheduleStatus, RecordingStatus, User, UserRestrictions,
    },
};
use noflex_storage_turso::TursoStore;
use serde_json::Value;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::Mutex;

#[derive(Default)]
struct ServiceCalls {
    refreshes: Vec<LiveTvSourceId>,
    starts: Vec<(UserId, ChannelId)>,
    cancels: Vec<(UserId, RecordingScheduleId)>,
    deletes: Vec<(UserId, RecordingId, bool)>,
}

struct FakeLiveTvService {
    storage: Arc<TursoStore>,
    calls: Mutex<ServiceCalls>,
}

#[async_trait::async_trait]
impl LiveTvService for FakeLiveTvService {
    async fn refresh_source(&self, source_id: LiveTvSourceId) -> Result<(), LiveTvServiceError> {
        self.calls.lock().await.refreshes.push(source_id);
        let mut source = self
            .storage
            .get_live_tv_source(source_id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("source"))?;
        source.last_refresh_at = Some(OffsetDateTime::now_utc());
        source.last_error = None;
        self.storage
            .upsert_live_tv_source(&source)
            .await
            .map_err(LiveTvServiceError::internal)?;
        Ok(())
    }

    async fn start_channel(
        &self,
        owner: UserId,
        channel_id: ChannelId,
    ) -> Result<PlaybackSession, LiveTvServiceError> {
        self.calls.lock().await.starts.push((owner, channel_id));
        let channel = self
            .storage
            .get_live_tv_channel(channel_id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .filter(|channel| channel.enabled)
            .ok_or(LiveTvServiceError::NotFound("channel"))?;
        if self
            .storage
            .get_user_restrictions(owner)
            .await
            .map_err(LiveTvServiceError::internal)?
            .is_some_and(|restrictions| restrictions.max_concurrent_transcodes == Some(0))
        {
            return Err(LiveTvServiceError::LimitReached);
        }
        let plan = noflex_transcoder::TranscodePlan {
            segment_container: noflex_transcoder::HlsSegmentContainer::Ts,
            video: noflex_transcoder::VideoPlan::None,
            audio: noflex_transcoder::AudioPlan::None,
        };
        Ok(PlaybackSession {
            session_id: async_graphql::ID(format!("live-{}", channel.id)),
            route: PlaybackRouteKind::Transcode,
            url: format!("/stream/job/live-{}/master.m3u8?token=fake", channel.id),
            file_url: None,
            job_id: Some(async_graphql::ID(format!("live-{}", channel.id))),
            start_position_ms: 0,
            decision: noflex_transcoder::PlaybackDecisionInfo::live(
                &plan,
                noflex_transcoder::Encoder::Software264,
                None,
            ),
            subtitles: Vec::new(),
        })
    }

    async fn cancel_schedule(
        &self,
        caller: UserId,
        schedule_id: RecordingScheduleId,
    ) -> Result<RecordingSchedule, LiveTvServiceError> {
        self.calls.lock().await.cancels.push((caller, schedule_id));
        let schedule = self
            .storage
            .get_recording_schedule(schedule_id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("schedule"))?;
        let is_admin = self
            .storage
            .get_user(caller)
            .await
            .map_err(LiveTvServiceError::internal)?
            .is_some_and(|user| user.is_admin);
        if !is_admin && schedule.owner_id != caller {
            return Err(LiveTvServiceError::NotFound("schedule"));
        }
        self.storage
            .cancel_recording_schedule(schedule_id, schedule.owner_id, OffsetDateTime::now_utc())
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("schedule"))
    }

    async fn delete_recording(
        &self,
        caller: UserId,
        recording_id: RecordingId,
        delete_file: bool,
    ) -> Result<Recording, LiveTvServiceError> {
        self.calls
            .lock()
            .await
            .deletes
            .push((caller, recording_id, delete_file));
        let recording = self
            .storage
            .get_recording(recording_id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("recording"))?;
        let is_admin = self
            .storage
            .get_user(caller)
            .await
            .map_err(LiveTvServiceError::internal)?
            .is_some_and(|user| user.is_admin);
        if !is_admin && recording.owner_id != caller {
            return Err(LiveTvServiceError::NotFound("recording"));
        }
        if matches!(
            recording.status,
            RecordingStatus::Scheduled | RecordingStatus::Running
        ) {
            return Err(LiveTvServiceError::RecordingActive);
        }
        self.storage
            .delete_recording(recording_id, recording.owner_id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("recording"))
    }
}

struct Fixture {
    schema: NoflexSchema,
    storage: Arc<TursoStore>,
    service: Arc<FakeLiveTvService>,
    admin: Principal,
    owner: Principal,
    other: Principal,
    restricted: Principal,
    source: LiveTvSource,
    channels: Vec<LiveTvChannel>,
    programs: Vec<EpgProgram>,
}

fn principal(user: &User) -> Principal {
    Principal {
        user_id: user.id,
        is_admin: user.is_admin,
        session_id: None,
    }
}

fn request(query: impl Into<String>, principal: Principal) -> Request {
    Request::new(query).data(principal).data(RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    })
}

fn user(name: &str, is_admin: bool) -> User {
    User {
        id: UserId::new(),
        name: name.to_string(),
        password_hash: "argon2id$placeholder".to_string(),
        is_admin,
        role: if is_admin {
            noflex_core::UserRole::Admin
        } else {
            noflex_core::UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
    }
}

async fn fixture() -> Fixture {
    let storage = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    storage.migrate().await.expect("migrate");

    let admin = user("live-tv-admin", true);
    let owner = user("live-tv-owner", false);
    let other = user("live-tv-other", false);
    let restricted = user("live-tv-restricted", false);
    for user in [&admin, &owner, &other, &restricted] {
        storage.upsert_user(user).await.expect("user");
    }
    storage
        .set_user_restrictions(&UserRestrictions {
            user_id: restricted.id,
            allowed_parental_rating_ids: vec![],
            allow_unrated: true,
            max_stream_bitrate_bps: None,
            max_concurrent_transcodes: Some(0),
        })
        .await
        .expect("restricted user");

    let source = LiveTvSource {
        id: LiveTvSourceId::new(),
        name: "Private IPTV".to_string(),
        enabled: true,
        last_refresh_at: None,
        last_error: None,
        playlist_url: "https://private.example/playlist.m3u8?secret=playlist".to_string(),
        epg_url: Some("https://private.example/guide.xml?secret=guide".to_string()),
    };
    storage
        .upsert_live_tv_source(&source)
        .await
        .expect("source");

    let channel_news = LiveTvChannel {
        id: ChannelId::new(),
        source_id: source.id,
        external_id: "news".to_string(),
        name: "News".to_string(),
        number: Some("1".to_string()),
        logo_url: Some("https://private.example/logo.png".to_string()),
        group: Some("News".to_string()),
        enabled: true,
        stream_url: "https://private.example/live/news.m3u8?secret=stream".to_string(),
    };
    let channel_sports = LiveTvChannel {
        id: ChannelId::new(),
        source_id: source.id,
        external_id: "sports".to_string(),
        name: "Sports".to_string(),
        number: Some("2".to_string()),
        logo_url: None,
        group: Some("Sports".to_string()),
        enabled: true,
        stream_url: "https://private.example/live/sports.m3u8?secret=stream".to_string(),
    };
    let now = OffsetDateTime::now_utc();
    let program_news = EpgProgram {
        id: EpgProgramId::new(),
        channel_id: channel_news.id,
        external_id: "news-now".to_string(),
        title: "News Now".to_string(),
        subtitle: Some("Top stories".to_string()),
        description: Some("Current events".to_string()),
        category: Some("News".to_string()),
        start_at: now - Duration::minutes(5),
        end_at: now + Duration::minutes(25),
    };
    let program_sports = EpgProgram {
        id: EpgProgramId::new(),
        channel_id: channel_sports.id,
        external_id: "sports-now".to_string(),
        title: "Sports Live".to_string(),
        subtitle: None,
        description: None,
        category: Some("Sports".to_string()),
        start_at: now + Duration::minutes(10),
        end_at: now + Duration::minutes(40),
    };
    let program_later = EpgProgram {
        id: EpgProgramId::new(),
        channel_id: channel_news.id,
        external_id: "news-later".to_string(),
        title: "News Later".to_string(),
        subtitle: None,
        description: None,
        category: Some("News".to_string()),
        start_at: now + Duration::hours(2),
        end_at: now + Duration::hours(3),
    };
    let channels = vec![channel_news, channel_sports];
    let programs = vec![program_news, program_sports, program_later];
    storage
        .apply_live_tv_refresh(source.id, &channels, &programs, now)
        .await
        .expect("catalog refresh");

    let service = Arc::new(FakeLiveTvService {
        storage: storage.clone(),
        calls: Mutex::default(),
    });
    let schema = build_schema(SchemaContext {
        storage: Some(storage.clone()),
        live_tv: Some(service.clone()),
        ..Default::default()
    });

    Fixture {
        schema,
        storage,
        service,
        admin: principal(&admin),
        owner: principal(&owner),
        other: principal(&other),
        restricted: principal(&restricted),
        source,
        channels,
        programs,
    }
}

fn json(response: Response, label: &str) -> Value {
    assert!(response.errors.is_empty(), "{label}: {:?}", response.errors);
    response.data.into_json().expect("GraphQL JSON")
}

fn assert_error(response: &Response, label: &str, expected: &str) {
    assert!(
        !response.errors.is_empty(),
        "{label} unexpectedly succeeded"
    );
    assert!(
        response
            .errors
            .iter()
            .any(|error| error.message.contains(expected)),
        "{label} errors {:?} do not contain {expected:?}",
        response.errors
    );
}

fn gql_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).expect("RFC3339 time")
}

fn schedule(
    owner_id: UserId,
    channel_id: ChannelId,
    start_at: OffsetDateTime,
    end_at: OffsetDateTime,
) -> RecordingSchedule {
    let now = OffsetDateTime::now_utc();
    RecordingSchedule {
        id: RecordingScheduleId::new(),
        owner_id,
        channel_id,
        program_id: None,
        start_at,
        end_at,
        padding_before_secs: 0,
        padding_after_secs: 0,
        status: RecordingScheduleStatus::Scheduled,
        created_at: now,
        updated_at: now,
    }
}

async fn completed_recording(
    storage: &Arc<TursoStore>,
    owner_id: UserId,
    channel_id: ChannelId,
    start_at: OffsetDateTime,
) -> Recording {
    let schedule = schedule(
        owner_id,
        channel_id,
        start_at,
        start_at + Duration::minutes(30),
    );
    storage
        .create_recording_schedule(&schedule)
        .await
        .expect("recording schedule");
    let (_triggered, recording) = storage
        .claim_due_recording(OffsetDateTime::now_utc())
        .await
        .expect("claim recording")
        .expect("due recording");
    storage
        .finish_recording(
            recording.id,
            RecordingStatus::Completed,
            Some(&format!("/private/recordings/{}.mkv", recording.id)),
            None,
            Some(42),
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("finish recording");
    storage
        .get_recording(recording.id)
        .await
        .expect("get recording")
        .expect("completed recording")
}

#[tokio::test]
async fn live_tv_surfaces_reject_anonymous_and_non_admin_source_mutations() {
    let fixture = fixture().await;
    let source_id = fixture.source.id;
    let channel_id = fixture.channels[0].id;
    let start = gql_time(fixture.programs[0].start_at);
    let end = gql_time(fixture.programs[0].end_at);
    let anonymous = [
        "{ liveTvSources { id } }".to_string(),
        "{ liveTvChannels { nodes { id } } }".to_string(),
        format!(
            r#"{{ liveTvGuide(channelIds: ["{channel_id}"], start: "{start}", end: "{end}") {{ nodes {{ id }} }} }}"#
        ),
        "{ recordingSchedules { nodes { id } } }".to_string(),
        "{ recordings { nodes { id } } }".to_string(),
        r#"mutation { createLiveTvSource(input: { name: "Nope", playlistUrl: "https://private.example/nope" }) { id } }"#
            .to_string(),
        format!(
            r#"mutation {{ updateLiveTvSource(id: "{source_id}", input: {{ name: "Nope" }}) {{ id }} }}"#
        ),
        format!(r#"mutation {{ deleteLiveTvSource(id: "{source_id}") {{ id }} }}"#),
        format!(r#"mutation {{ refreshLiveTvSource(id: "{source_id}") {{ id }} }}"#),
        format!(r#"mutation {{ startLiveTvChannel(channelId: "{channel_id}") {{ jobId }} }}"#),
        format!(
            r#"mutation {{ scheduleRecording(channelId: "{channel_id}", startAt: "{start}", endAt: "{end}") {{ id }} }}"#
        ),
        format!(
            r#"mutation {{ cancelRecordingSchedule(id: "{}") {{ id }} }}"#,
            RecordingScheduleId::new()
        ),
        format!(
            r#"mutation {{ deleteRecording(id: "{}", deleteFile: false) {{ id }} }}"#,
            RecordingId::new()
        ),
    ];
    for query in anonymous {
        assert_error(
            &fixture.schema.execute(query).await,
            "anonymous Live TV request",
            "authentication required",
        );
    }

    let non_admin = [
        r#"mutation { createLiveTvSource(input: { name: "Nope", playlistUrl: "https://private.example/nope" }) { id } }"#
            .to_string(),
        format!(
            r#"mutation {{ updateLiveTvSource(id: "{source_id}", input: {{ name: "Nope" }}) {{ id }} }}"#
        ),
        format!(r#"mutation {{ deleteLiveTvSource(id: "{source_id}") {{ id }} }}"#),
        format!(r#"mutation {{ refreshLiveTvSource(id: "{source_id}") {{ id }} }}"#),
    ];
    for query in non_admin {
        assert_error(
            &fixture.schema.execute(request(query, fixture.owner)).await,
            "non-admin source mutation",
            "administrator required",
        );
    }
    assert!(fixture.service.calls.lock().await.refreshes.is_empty());
}

#[tokio::test]
async fn admin_can_crud_sources_refresh_and_schema_hides_private_fields() {
    let fixture = fixture().await;
    let create = r#"mutation {
        createLiveTvSource(input: {
            name: "Second Source",
            playlistUrl: "https://private.example/second.m3u?secret=second",
            epgUrl: "https://private.example/second.xml?secret=second",
            enabled: false
        }) { id name enabled lastRefreshAt lastError }
    }"#;
    let data = json(
        fixture.schema.execute(request(create, fixture.admin)).await,
        "create source",
    );
    let created_id: LiveTvSourceId = data["createLiveTvSource"]["id"]
        .as_str()
        .expect("created source id")
        .parse()
        .map(LiveTvSourceId::from_uuid)
        .expect("source UUID");
    assert_eq!(data["createLiveTvSource"]["name"], "Second Source");
    assert_eq!(data["createLiveTvSource"]["enabled"], false);
    assert!(
        !data
            .to_string()
            .contains("https://private.example/second.m3u")
    );
    assert!(
        !data
            .to_string()
            .contains("https://private.example/second.xml")
    );

    let persisted = fixture
        .storage
        .get_live_tv_source(created_id)
        .await
        .expect("created source")
        .expect("created source row");
    assert_eq!(
        persisted.playlist_url,
        "https://private.example/second.m3u?secret=second"
    );
    assert_eq!(
        persisted.epg_url.as_deref(),
        Some("https://private.example/second.xml?secret=second")
    );

    let update = format!(
        r#"mutation {{ updateLiveTvSource(id: "{created_id}", input: {{ name: "Updated Source", playlistUrl: "https://private.example/updated.m3u?secret=updated", clearEpgUrl: true, enabled: true }}) {{ id name enabled lastRefreshAt lastError }} }}"#
    );
    let data = json(
        fixture.schema.execute(request(update, fixture.admin)).await,
        "update source",
    );
    assert_eq!(data["updateLiveTvSource"]["name"], "Updated Source");
    assert_eq!(data["updateLiveTvSource"]["enabled"], true);
    assert!(
        !data
            .to_string()
            .contains("https://private.example/updated.m3u")
    );
    let persisted = fixture
        .storage
        .get_live_tv_source(created_id)
        .await
        .expect("updated source")
        .expect("updated source row");
    assert_eq!(
        persisted.playlist_url,
        "https://private.example/updated.m3u?secret=updated"
    );
    assert!(persisted.epg_url.is_none());

    let refresh = format!(
        r#"mutation {{ refreshLiveTvSource(id: "{created_id}") {{ id name enabled lastRefreshAt lastError }} }}"#
    );
    let data = json(
        fixture
            .schema
            .execute(request(refresh, fixture.admin))
            .await,
        "refresh source",
    );
    assert_eq!(data["refreshLiveTvSource"]["name"], "Updated Source");
    assert!(data["refreshLiveTvSource"]["lastRefreshAt"].is_string());
    assert_eq!(
        fixture.service.calls.lock().await.refreshes,
        vec![created_id]
    );

    let delete = format!(r#"mutation {{ deleteLiveTvSource(id: "{created_id}") {{ id }} }}"#);
    let data = json(
        fixture
            .schema
            .execute(request(delete.clone(), fixture.admin))
            .await,
        "delete source",
    );
    assert!(data["deleteLiveTvSource"].is_object());
    let data = json(
        fixture.schema.execute(request(delete, fixture.admin)).await,
        "delete missing source",
    );
    assert!(data["deleteLiveTvSource"].is_null());

    for (type_name, hidden_field) in [
        ("LiveTvSource", "playlistUrl"),
        ("LiveTvSource", "epgUrl"),
        ("LiveTvChannel", "streamUrl"),
        ("Recording", "path"),
    ] {
        let introspection = format!(r#"{{ __type(name: "{type_name}") {{ fields {{ name }} }} }}"#);
        let data = json(
            fixture.schema.execute(introspection).await,
            "private field introspection",
        );
        let fields = data["__type"]["fields"].as_array().expect("schema fields");
        assert!(!fields.iter().any(|field| field["name"] == hidden_field));
    }

    for query in [
        "{ liveTvSources { playlistUrl } }".to_string(),
        "{ liveTvChannels { nodes { streamUrl } } }".to_string(),
        "{ recordings { nodes { path } } }".to_string(),
    ] {
        assert_error(
            &fixture.schema.execute(request(query, fixture.admin)).await,
            "private GraphQL field",
            "Unknown field",
        );
    }
}

#[tokio::test]
async fn authenticated_channels_and_guide_honor_list_and_time_windows() {
    let fixture = fixture().await;
    let source_id = fixture.source.id;
    let first_page = format!(
        r#"{{ liveTvChannels(sourceId: "{source_id}", first: 1) {{ nodes {{ id name number enabled }} pageInfo {{ hasNextPage endCursor }} }} }}"#
    );
    let data = json(
        fixture
            .schema
            .execute(request(first_page, fixture.owner))
            .await,
        "first channel page",
    );
    let first_items = data["liveTvChannels"]["nodes"]
        .as_array()
        .expect("first channel items");
    assert_eq!(first_items.len(), 1);
    assert_eq!(data["liveTvChannels"]["pageInfo"]["hasNextPage"], true);
    let cursor = data["liveTvChannels"]["pageInfo"]["endCursor"]
        .as_str()
        .expect("channel cursor");

    let second_page = format!(
        r#"{{ liveTvChannels(sourceId: "{source_id}", first: 1, after: "{cursor}") {{ nodes {{ id name }} pageInfo {{ hasNextPage endCursor }} }} }}"#
    );
    let data = json(
        fixture
            .schema
            .execute(request(second_page, fixture.owner))
            .await,
        "second channel page",
    );
    let second_items = data["liveTvChannels"]["nodes"]
        .as_array()
        .expect("second channel items");
    assert_eq!(second_items.len(), 1);
    assert_eq!(data["liveTvChannels"]["pageInfo"]["hasNextPage"], false);
    assert_ne!(first_items[0]["id"], second_items[0]["id"]);

    let start = fixture.programs[0].start_at - Duration::minutes(1);
    let end = fixture.programs[1].end_at + Duration::minutes(1);
    let guide = format!(
        r#"{{ liveTvGuide(channelIds: ["{}", "{}"], start: "{}", end: "{}", first: 10) {{ nodes {{ id channelId title startAt endAt }} }} }}"#,
        fixture.channels[0].id,
        fixture.channels[1].id,
        gql_time(start),
        gql_time(end),
    );
    let data = json(
        fixture.schema.execute(request(guide, fixture.owner)).await,
        "guide window",
    );
    let programs = data["liveTvGuide"]["nodes"].as_array().expect("guide rows");
    assert_eq!(programs.len(), 2);
    let titles: Vec<&str> = programs
        .iter()
        .map(|program| program["title"].as_str().expect("program title"))
        .collect();
    assert!(titles.contains(&"News Now"));
    assert!(titles.contains(&"Sports Live"));
    assert!(!titles.contains(&"News Later"));

    let invalid = format!(
        r#"{{ liveTvGuide(channelIds: ["{}"], start: "{}", end: "{}") {{ nodes {{ id }} }} }}"#,
        fixture.channels[0].id,
        gql_time(end),
        gql_time(start),
    );
    assert_error(
        &fixture
            .schema
            .execute(request(invalid, fixture.owner))
            .await,
        "invalid guide window",
        "end must be after start",
    );
}

#[tokio::test]
async fn schedule_recording_enforces_owner_and_channel_program_time_validation() {
    let fixture = fixture().await;
    let channel = fixture.channels[0].id;
    let other_channel = fixture.channels[1].id;
    let program = fixture.programs[0].id;
    let other_program = fixture.programs[1].id;
    let start = OffsetDateTime::now_utc() + Duration::hours(1);
    let end = start + Duration::minutes(30);
    let valid = format!(
        r#"mutation {{ scheduleRecording(channelId: "{channel}", programId: "{program}", startAt: "{}", endAt: "{}", paddingBeforeSecs: 5, paddingAfterSecs: 7) {{ id ownerId channelId programId startAt endAt paddingBeforeSecs paddingAfterSecs status }} }}"#,
        gql_time(start),
        gql_time(end),
    );
    let data = json(
        fixture.schema.execute(request(valid, fixture.owner)).await,
        "valid schedule",
    );
    assert_eq!(
        data["scheduleRecording"]["ownerId"],
        fixture.owner.user_id.to_string()
    );
    assert_eq!(data["scheduleRecording"]["channelId"], channel.to_string());
    assert_eq!(data["scheduleRecording"]["programId"], program.to_string());
    assert_eq!(data["scheduleRecording"]["status"], "SCHEDULED");
    assert_eq!(data["scheduleRecording"]["paddingBeforeSecs"], 5);
    assert_eq!(data["scheduleRecording"]["paddingAfterSecs"], 7);

    let invalid_channel = format!(
        r#"mutation {{ scheduleRecording(channelId: "{}", startAt: "{}", endAt: "{}") {{ id }} }}"#,
        ChannelId::new(),
        gql_time(start),
        gql_time(end),
    );
    assert_error(
        &fixture
            .schema
            .execute(request(invalid_channel, fixture.owner))
            .await,
        "invalid recording channel",
        "channel not found",
    );

    let wrong_program = format!(
        r#"mutation {{ scheduleRecording(channelId: "{channel}", programId: "{other_program}", startAt: "{}", endAt: "{}") {{ id }} }}"#,
        gql_time(start),
        gql_time(end),
    );
    assert_error(
        &fixture
            .schema
            .execute(request(wrong_program, fixture.owner))
            .await,
        "program on another channel",
        "program does not belong to channel",
    );

    let unknown_program = format!(
        r#"mutation {{ scheduleRecording(channelId: "{channel}", programId: "{}", startAt: "{}", endAt: "{}") {{ id }} }}"#,
        EpgProgramId::new(),
        gql_time(start),
        gql_time(end),
    );
    assert_error(
        &fixture
            .schema
            .execute(request(unknown_program, fixture.owner))
            .await,
        "unknown recording program",
        "program not found",
    );

    let invalid_time = format!(
        r#"mutation {{ scheduleRecording(channelId: "{other_channel}", startAt: "{}", endAt: "{}") {{ id }} }}"#,
        gql_time(start),
        gql_time(start),
    );
    assert_error(
        &fixture
            .schema
            .execute(request(invalid_time, fixture.owner))
            .await,
        "invalid recording window",
        "end must be after start",
    );
}

#[tokio::test]
async fn recording_lists_are_self_scoped_and_admin_can_select_other_owners() {
    let fixture = fixture().await;
    let now = OffsetDateTime::now_utc();
    let owner_schedule = schedule(
        fixture.owner.user_id,
        fixture.channels[0].id,
        now + Duration::hours(1),
        now + Duration::hours(2),
    );
    let other_schedule = schedule(
        fixture.other.user_id,
        fixture.channels[0].id,
        now + Duration::hours(3),
        now + Duration::hours(4),
    );
    for schedule in [&owner_schedule, &other_schedule] {
        fixture
            .storage
            .create_recording_schedule(schedule)
            .await
            .expect("schedule row");
    }
    let owner_recording = completed_recording(
        &fixture.storage,
        fixture.owner.user_id,
        fixture.channels[0].id,
        now - Duration::hours(2),
    )
    .await;
    let other_recording = completed_recording(
        &fixture.storage,
        fixture.other.user_id,
        fixture.channels[0].id,
        now - Duration::hours(1),
    )
    .await;

    let data = json(
        fixture
            .schema
            .execute(request(
                "{ recordingSchedules { nodes { id ownerId } } }",
                fixture.owner,
            ))
            .await,
        "owner schedules",
    );
    let schedules = data["recordingSchedules"]["nodes"]
        .as_array()
        .expect("owner schedules");
    assert_eq!(schedules.len(), 2);
    let schedule_ids: Vec<String> = schedules
        .iter()
        .map(|schedule| schedule["id"].as_str().expect("schedule id").to_string())
        .collect();
    let owner_schedule_id = owner_schedule.id.to_string();
    let owner_recording_schedule_id = owner_recording
        .schedule_id
        .expect("owner recording schedule")
        .to_string();
    assert!(schedule_ids.contains(&owner_schedule_id));
    assert!(schedule_ids.contains(&owner_recording_schedule_id));

    let data = json(
        fixture
            .schema
            .execute(request(
                "{ recordings { nodes { id ownerId status } } }",
                fixture.owner,
            ))
            .await,
        "owner recordings",
    );
    let recordings = data["recordings"]["nodes"]
        .as_array()
        .expect("owner recordings");
    assert_eq!(recordings.len(), 1);
    assert_eq!(recordings[0]["id"], owner_recording.id.to_string());
    assert_eq!(recordings[0]["status"], "COMPLETED");
    assert!(!data.to_string().contains("/private/recordings"));

    for query in [
        format!(
            r#"{{ recordingSchedules(owner: "{}") {{ nodes {{ id }} }} }}"#,
            fixture.other.user_id
        ),
        format!(
            r#"{{ recordings(owner: "{}") {{ nodes {{ id }} }} }}"#,
            fixture.other.user_id
        ),
    ] {
        assert_error(
            &fixture.schema.execute(request(query, fixture.owner)).await,
            "foreign recording list",
            "recordings not found",
        );
    }

    let other_schedules = format!(
        r#"{{ recordingSchedules(owner: "{}") {{ nodes {{ id ownerId }} }} }}"#,
        fixture.other.user_id
    );
    let data = json(
        fixture
            .schema
            .execute(request(other_schedules, fixture.admin))
            .await,
        "admin other schedules",
    );
    let other_schedule_ids: Vec<String> = data["recordingSchedules"]["nodes"]
        .as_array()
        .expect("admin other schedules")
        .iter()
        .map(|schedule| schedule["id"].as_str().expect("schedule id").to_string())
        .collect();
    let other_schedule_id = other_schedule.id.to_string();
    let other_recording_schedule_id = other_recording
        .schedule_id
        .expect("other recording schedule")
        .to_string();
    assert!(other_schedule_ids.contains(&other_schedule_id));
    assert!(other_schedule_ids.contains(&other_recording_schedule_id));

    let other_recordings = format!(
        r#"{{ recordings(owner: "{}") {{ nodes {{ id ownerId status }} }} }}"#,
        fixture.other.user_id
    );
    let data = json(
        fixture
            .schema
            .execute(request(other_recordings, fixture.admin))
            .await,
        "admin other recordings",
    );
    assert_eq!(
        data["recordings"]["nodes"][0]["id"],
        other_recording.id.to_string()
    );
}

#[tokio::test]
async fn cancel_recording_schedule_checks_owner_admin_and_triggered_state() {
    let fixture = fixture().await;
    let now = OffsetDateTime::now_utc();
    let own = schedule(
        fixture.owner.user_id,
        fixture.channels[0].id,
        now + Duration::hours(1),
        now + Duration::hours(2),
    );
    let foreign = schedule(
        fixture.other.user_id,
        fixture.channels[0].id,
        now + Duration::hours(3),
        now + Duration::hours(4),
    );
    for schedule in [&own, &foreign] {
        fixture
            .storage
            .create_recording_schedule(schedule)
            .await
            .expect("schedule row");
    }

    let cancel_own = format!(
        r#"mutation {{ cancelRecordingSchedule(id: "{}") {{ id ownerId status }} }}"#,
        own.id
    );
    let data = json(
        fixture
            .schema
            .execute(request(cancel_own, fixture.owner))
            .await,
        "cancel own schedule",
    );
    assert_eq!(data["cancelRecordingSchedule"]["status"], "CANCELLED");
    assert_eq!(
        fixture
            .storage
            .get_recording_schedule(own.id)
            .await
            .expect("own schedule")
            .expect("own schedule row")
            .status,
        RecordingScheduleStatus::Cancelled
    );

    let cancel_foreign = format!(
        r#"mutation {{ cancelRecordingSchedule(id: "{}") {{ id }} }}"#,
        foreign.id
    );
    assert_error(
        &fixture
            .schema
            .execute(request(cancel_foreign.clone(), fixture.owner))
            .await,
        "foreign schedule cancellation",
        "schedule not found",
    );
    let cancel_foreign_admin = format!(
        r#"mutation {{ cancelRecordingSchedule(id: "{}") {{ id status }} }}"#,
        foreign.id
    );
    let data = json(
        fixture
            .schema
            .execute(request(cancel_foreign_admin, fixture.admin))
            .await,
        "admin schedule cancellation",
    );
    assert_eq!(data["cancelRecordingSchedule"]["status"], "CANCELLED");

    let triggered = schedule(
        fixture.owner.user_id,
        fixture.channels[0].id,
        now - Duration::minutes(10),
        now + Duration::minutes(20),
    );
    fixture
        .storage
        .create_recording_schedule(&triggered)
        .await
        .expect("triggered schedule row");
    let (claimed, recording) = fixture
        .storage
        .claim_due_recording(OffsetDateTime::now_utc())
        .await
        .expect("claim triggered schedule")
        .expect("triggered row");
    assert_eq!(claimed.status, RecordingScheduleStatus::Triggered);
    let cancel_triggered = format!(
        r#"mutation {{ cancelRecordingSchedule(id: "{}") {{ id status }} }}"#,
        triggered.id
    );
    let data = json(
        fixture
            .schema
            .execute(request(cancel_triggered, fixture.owner))
            .await,
        "cancel triggered schedule",
    );
    assert_eq!(data["cancelRecordingSchedule"]["status"], "CANCELLED");
    assert_eq!(
        fixture
            .storage
            .get_recording(recording.id)
            .await
            .expect("cancelled recording")
            .expect("cancelled recording row")
            .status,
        RecordingStatus::Cancelled
    );

    let calls = fixture.service.calls.lock().await;
    assert!(calls.cancels.contains(&(fixture.owner.user_id, own.id)));
    assert!(calls.cancels.contains(&(fixture.owner.user_id, foreign.id)));
    assert!(calls.cancels.contains(&(fixture.admin.user_id, foreign.id)));
    assert!(
        calls
            .cancels
            .contains(&(fixture.owner.user_id, triggered.id))
    );
}

#[tokio::test]
async fn delete_recording_is_authorized_by_service_and_records_delete_file_flag() {
    let fixture = fixture().await;
    let now = OffsetDateTime::now_utc();
    let owner_recording = completed_recording(
        &fixture.storage,
        fixture.owner.user_id,
        fixture.channels[0].id,
        now - Duration::hours(2),
    )
    .await;
    let other_recording = completed_recording(
        &fixture.storage,
        fixture.other.user_id,
        fixture.channels[0].id,
        now - Duration::hours(1),
    )
    .await;

    let delete_owner = format!(
        r#"mutation {{ deleteRecording(id: "{}", deleteFile: true) {{ id ownerId status }} }}"#,
        owner_recording.id
    );
    let data = json(
        fixture
            .schema
            .execute(request(delete_owner, fixture.owner))
            .await,
        "delete own recording",
    );
    assert_eq!(
        data["deleteRecording"]["id"],
        owner_recording.id.to_string()
    );
    assert_eq!(data["deleteRecording"]["status"], "COMPLETED");
    assert!(!data.to_string().contains("/private/recordings"));
    assert!(
        fixture
            .storage
            .get_recording(owner_recording.id)
            .await
            .expect("deleted own recording")
            .is_none()
    );

    let delete_foreign = format!(
        r#"mutation {{ deleteRecording(id: "{}", deleteFile: false) {{ id }} }}"#,
        other_recording.id
    );
    assert_error(
        &fixture
            .schema
            .execute(request(delete_foreign.clone(), fixture.owner))
            .await,
        "delete foreign recording",
        "recording not found",
    );
    assert!(
        fixture
            .storage
            .get_recording(other_recording.id)
            .await
            .expect("foreign recording")
            .is_some()
    );

    let data = json(
        fixture
            .schema
            .execute(request(delete_foreign, fixture.admin))
            .await,
        "admin delete foreign recording",
    );
    assert_eq!(
        data["deleteRecording"]["id"],
        other_recording.id.to_string()
    );
    assert!(
        fixture
            .storage
            .get_recording(other_recording.id)
            .await
            .expect("deleted foreign recording")
            .is_none()
    );

    let calls = fixture.service.calls.lock().await;
    assert!(
        calls
            .deletes
            .contains(&(fixture.owner.user_id, owner_recording.id, true))
    );
    assert!(
        calls
            .deletes
            .contains(&(fixture.owner.user_id, other_recording.id, false))
    );
    assert!(
        calls
            .deletes
            .contains(&(fixture.admin.user_id, other_recording.id, false))
    );
}

#[tokio::test]
async fn start_live_tv_channel_passes_auth_user_and_enforces_fake_restriction() {
    let fixture = fixture().await;
    let channel_id = fixture.channels[0].id;
    let start = format!(
        r#"mutation {{ startLiveTvChannel(channelId: "{channel_id}") {{ route url jobId }} }}"#
    );

    let response = fixture
        .schema
        .execute(Request::new(start.clone()).data(fixture.owner))
        .await;
    assert_error(
        &response,
        "principal without RequestUser",
        "authentication required",
    );

    let data = json(
        fixture
            .schema
            .execute(request(start.clone(), fixture.owner))
            .await,
        "start live channel",
    );
    assert_eq!(data["startLiveTvChannel"]["route"], "TRANSCODE");
    assert_eq!(
        data["startLiveTvChannel"]["url"],
        format!("/stream/job/live-{channel_id}/master.m3u8?token=fake")
    );
    assert_eq!(
        data["startLiveTvChannel"]["jobId"],
        format!("live-{channel_id}")
    );
    assert!(!data.to_string().contains("private.example/live"));

    let response = fixture
        .schema
        .execute(request(start, fixture.restricted))
        .await;
    assert_error(
        &response,
        "restricted live channel",
        "concurrent stream limit reached",
    );

    let calls = fixture.service.calls.lock().await;
    assert!(calls.starts.contains(&(fixture.owner.user_id, channel_id)));
    assert!(
        calls
            .starts
            .contains(&(fixture.restricted.user_id, channel_id))
    );
}

#[tokio::test]
async fn guide_and_schedule_inputs_are_bounded() {
    let fixture = fixture().await;
    let channel = fixture.channels[0].id;
    let now = OffsetDateTime::now_utc();

    let too_many = (0..201)
        .map(|_| format!("\"{}\"", ChannelId::new()))
        .collect::<Vec<_>>()
        .join(",");
    let guide = format!(
        r#"{{ liveTvGuide(channelIds: [{too_many}], start: "{}", end: "{}") {{ nodes {{ id }} }} }}"#,
        gql_time(now),
        gql_time(now + Duration::hours(1)),
    );
    assert_error(
        &fixture.schema.execute(request(guide, fixture.owner)).await,
        "oversized channel list",
        "at most 200 channels",
    );
    let wide = format!(
        r#"{{ liveTvGuide(channelIds: ["{channel}"], start: "{}", end: "{}") {{ nodes {{ id }} }} }}"#,
        gql_time(now),
        gql_time(now + Duration::days(15)),
    );
    assert_error(
        &fixture.schema.execute(request(wide, fixture.owner)).await,
        "oversized guide window",
        "exceeds 14 days",
    );

    let schedule = |start: OffsetDateTime, end: OffsetDateTime, padding: i32| {
        format!(
            r#"mutation {{ scheduleRecording(channelId: "{channel}", startAt: "{}", endAt: "{}", paddingAfterSecs: {padding}) {{ id }} }}"#,
            gql_time(start),
            gql_time(end),
        )
    };
    for (label, query, expected) in [
        (
            "past window",
            schedule(now - Duration::hours(2), now - Duration::hours(1), 0),
            "end must be in the future",
        ),
        (
            "long window",
            schedule(now, now + Duration::hours(13), 0),
            "exceeds 12 hours",
        ),
        (
            "long padding",
            schedule(now, now + Duration::hours(1), 3601),
            "padding exceeds 1 hour",
        ),
    ] {
        assert_error(
            &fixture.schema.execute(request(query, fixture.owner)).await,
            label,
            expected,
        );
    }
}
