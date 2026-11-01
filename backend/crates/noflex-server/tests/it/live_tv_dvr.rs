// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_graphql::Request as GraphqlRequest;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{Request, StatusCode, header},
    response::Redirect,
    routing::get,
};
use noflex_auth::{JwtConfig, issue_jwt, issue_media_token};
use noflex_core::{AudioCodec, UserRole, VideoCodec};
use noflex_graphql::{LiveTvService, SchemaContext, build_schema};
use noflex_server::{
    config::{HwAccel, LadderEntry, TranscodeFallback, TranscoderConfig},
    live_tv::{LiveTvError, LiveTvIngest},
    live_tv_runtime::ServerLiveTv,
    restricted_routes::{RestrictedRouteState, routes},
    stream::{PlaybackJobTarget, ServerPlaybackSessions},
};
use noflex_storage::{
    LiveTvChannel, LiveTvSource, Recording, RecordingSchedule, RecordingScheduleStatus,
    RecordingStatus, RequestUser, Storage, domain::User,
};
use noflex_storage_turso::TursoStore;
use noflex_transcoder::{FfmpegJobManager, HwBackend};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tower::ServiceExt;

const JWT_SECRET: &[u8] = b"live-tv-dvr-integration-tests-key-32b";
const M3U: &str = "#EXTM3U\n#EXTINF:-1 tvg-id=\"news\" tvg-name=\"News HD\" tvg-chno=\"7\",News\nhttp://127.0.0.1/live/news\n";
const XMLTV: &str = r#"<tv><programme channel="news" start="20250101120000 +0000" stop="20250101130000 +0000"><title>Local News</title><category>News</category></programme></tv>"#;

struct Fixture {
    store: Arc<dyn Storage>,
    owner: User,
    foreign: User,
    admin: User,
    channel: LiveTvChannel,
    root: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store: Arc<dyn Storage> = Arc::new(store);
    let root = tempfile::tempdir().expect("tempdir");
    let now = OffsetDateTime::now_utc();
    let owner = user("owner", false, now);
    let foreign = user("foreign", false, now);
    let admin = user("admin", true, now);
    for user in [&owner, &foreign, &admin] {
        store.upsert_user(user).await.expect("user");
        create_session(store.as_ref(), user).await;
    }
    let source = LiveTvSource {
        id: noflex_core::LiveTvSourceId::new(),
        name: "Local tuner".into(),
        enabled: true,
        last_refresh_at: None,
        last_error: None,
        playlist_url: "http://127.0.0.1/playlist.m3u".into(),
        epg_url: None,
    };
    store.upsert_live_tv_source(&source).await.expect("source");
    let channel = LiveTvChannel {
        id: noflex_core::ChannelId::new(),
        source_id: source.id,
        external_id: "news".into(),
        name: "News".into(),
        number: Some("7".into()),
        logo_url: None,
        group: Some("Local".into()),
        enabled: true,
        stream_url: "http://127.0.0.1/live/news".into(),
    };
    store
        .apply_live_tv_refresh(source.id, std::slice::from_ref(&channel), &[], now)
        .await
        .expect("channel");
    Fixture {
        store,
        owner,
        foreign,
        admin,
        channel,
        root,
    }
}

fn user(name: &str, is_admin: bool, created_at: OffsetDateTime) -> User {
    User {
        id: noflex_core::UserId::new(),
        name: name.into(),
        password_hash: String::new(),
        is_admin,
        role: if is_admin {
            UserRole::Admin
        } else {
            UserRole::User
        },
        created_at,
    }
}

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("permissions");
    }
}

async fn local_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    format!("http://{address}")
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition timed out");
}

#[tokio::test]
async fn real_ingestor_refreshes_local_m3u_and_xmltv() {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store: Arc<dyn Storage> = Arc::new(store);
    let app = Router::new()
        .route("/playlist.m3u", get(|| async { M3U }))
        .route("/guide.xml", get(|| async { XMLTV }));
    let base = local_server(app).await;
    let source = LiveTvSource {
        id: noflex_core::LiveTvSourceId::new(),
        name: "HTTP fixture".into(),
        enabled: true,
        last_refresh_at: None,
        last_error: None,
        playlist_url: format!("{base}/playlist.m3u"),
        epg_url: Some(format!("{base}/guide.xml")),
    };
    store.upsert_live_tv_source(&source).await.expect("source");

    let ingest = LiveTvIngest::new(store.clone()).expect("ingestor");
    let start = OffsetDateTime::from_unix_timestamp(1_735_732_800).expect("start");
    ingest
        .refresh(source.id, start, start + Duration::from_secs(7200))
        .await
        .expect("refresh");

    let channels = store
        .list_live_tv_channels(Some(source.id), 10, None)
        .await
        .expect("channels");
    assert_eq!(channels.items.len(), 1);
    assert_eq!(channels.items[0].name, "News HD");
    let guide = store
        .list_live_tv_guide(
            &[channels.items[0].id],
            start,
            start + Duration::from_secs(7200),
            10,
            None,
        )
        .await
        .expect("guide")
        .items;
    assert_eq!(guide.len(), 1);
    assert_eq!(guide[0].title, "Local News");
    let refreshed = store
        .get_live_tv_source(source.id)
        .await
        .expect("source")
        .expect("source row");
    assert!(refreshed.last_refresh_at.is_some());
    assert!(refreshed.last_error.is_none());
}

#[tokio::test]
async fn ingestor_enforces_caps_refuses_redirects_and_serializes_one_source() {
    #[derive(Clone)]
    struct Concurrency {
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }
    async fn delayed(State(state): State<Concurrency>) -> &'static str {
        let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
        state.peak.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        state.active.fetch_sub(1, Ordering::SeqCst);
        M3U
    }
    let state = Concurrency {
        active: Arc::new(AtomicUsize::new(0)),
        peak: Arc::new(AtomicUsize::new(0)),
    };
    let oversized = Bytes::from(vec![b'x'; 8 * 1024 * 1024 + 1]);
    let oversized_xmltv = Bytes::from(vec![b'x'; 64 * 1024 * 1024 + 1]);
    let app = Router::new()
        .route("/playlist.m3u", get(delayed))
        .route(
            "/redirect",
            get(|| async { Redirect::temporary("/playlist.m3u") }),
        )
        .route(
            "/oversized",
            get(move || {
                let oversized = oversized.clone();
                async move { oversized }
            }),
        )
        .route(
            "/oversized.xml",
            get(move || {
                let oversized_xmltv = oversized_xmltv.clone();
                async move { oversized_xmltv }
            }),
        )
        .with_state(state.clone());
    let base = local_server(app).await;
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store: Arc<dyn Storage> = Arc::new(store);
    let mut source = LiveTvSource {
        id: noflex_core::LiveTvSourceId::new(),
        name: "serialized".into(),
        enabled: true,
        last_refresh_at: None,
        last_error: None,
        playlist_url: format!("{base}/playlist.m3u"),
        epg_url: None,
    };
    store.upsert_live_tv_source(&source).await.expect("source");
    let ingest = Arc::new(LiveTvIngest::new(store.clone()).expect("ingestor"));
    let now = OffsetDateTime::now_utc();
    let (first, second) = tokio::join!(
        ingest.refresh(source.id, now, now + Duration::from_secs(60)),
        ingest.refresh(source.id, now, now + Duration::from_secs(60))
    );
    first.expect("first refresh");
    second.expect("second refresh");
    assert_eq!(state.peak.load(Ordering::SeqCst), 1);

    source.playlist_url = format!("{base}/redirect");
    store
        .upsert_live_tv_source(&source)
        .await
        .expect("redirect source");
    assert!(matches!(
        ingest
            .refresh(source.id, now, now + Duration::from_secs(60))
            .await,
        Err(LiveTvError::Playlist(_))
    ));
    source.playlist_url = format!("{base}/oversized");
    store
        .upsert_live_tv_source(&source)
        .await
        .expect("large source");
    assert!(matches!(
        ingest
            .refresh(source.id, now, now + Duration::from_secs(60))
            .await,
        Err(LiveTvError::ResponseTooLarge(8_388_608))
    ));
    source.playlist_url = format!("{base}/playlist.m3u");
    source.epg_url = Some(format!("{base}/oversized.xml"));
    store
        .upsert_live_tv_source(&source)
        .await
        .expect("large guide source");
    assert!(matches!(
        ingest
            .refresh(source.id, now, now + Duration::from_secs(60))
            .await,
        Err(LiveTvError::ResponseTooLarge(67_108_864))
    ));
}

fn schedule(f: &Fixture, start_at: OffsetDateTime, end_at: OffsetDateTime) -> RecordingSchedule {
    let now = OffsetDateTime::now_utc();
    RecordingSchedule {
        id: noflex_core::RecordingScheduleId::new(),
        owner_id: f.owner.id,
        channel_id: f.channel.id,
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

async fn runtime(f: &Fixture, ffmpeg: PathBuf) -> Arc<ServerLiveTv> {
    let manager = Arc::new(FfmpegJobManager::new(
        f.root.path().join("unused-ffmpeg"),
        f.root.path().join("hls"),
    ));
    let playback = Arc::new(ServerPlaybackSessions::new(
        f.store.clone(),
        manager,
        transcoder_config(),
        HwBackend::Software,
        None,
        jwt(),
    ));
    tokio::fs::create_dir_all(f.root.path().join("recordings"))
        .await
        .expect("recordings dir");
    Arc::new(
        ServerLiveTv::new(f.store.clone(), playback, ffmpeg, f.root.path().into(), 1)
            .expect("live TV runtime"),
    )
}

async fn recording_for(f: &Fixture, schedule: &RecordingSchedule) -> Recording {
    f.store
        .recording_for_schedule(schedule.id)
        .await
        .expect("recording lookup")
        .expect("recording")
}

async fn wait_for_terminal(f: &Fixture, schedule: &RecordingSchedule) -> Recording {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let recording = recording_for(f, schedule).await;
            if recording.status != RecordingStatus::Running {
                break recording;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("recording timed out")
}

#[tokio::test]
async fn dvr_worker_completes_atomically_and_failure_cleans_temp() {
    let f = fixture().await;
    let ffmpeg = f.root.path().join("ffmpeg-success");
    executable(
        &ffmpeg,
        "#!/bin/sh\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf recorded > \"$out\"\n",
    );
    let service = runtime(&f, ffmpeg).await;
    let success = schedule(
        &f,
        OffsetDateTime::now_utc() - Duration::from_secs(1),
        OffsetDateTime::now_utc() + Duration::from_secs(30),
    );
    f.store
        .create_recording_schedule(&success)
        .await
        .expect("schedule");
    service.poll_once().await.expect("poll");
    let completed = wait_for_terminal(&f, &success).await;
    assert_eq!(completed.status, RecordingStatus::Completed);
    let final_path = completed.path.as_deref().expect("final path");
    assert_eq!(std::fs::read(final_path).expect("final file"), b"recorded");
    assert!(
        !f.root
            .path()
            .join("recordings")
            .join(format!("{}.part.mkv", completed.id))
            .exists()
    );

    let ffmpeg = f.root.path().join("ffmpeg-failure");
    executable(
        &ffmpeg,
        "#!/bin/sh\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf partial > \"$out\"\nexit 9\n",
    );
    let service = runtime(&f, ffmpeg).await;
    let failed = schedule(
        &f,
        OffsetDateTime::now_utc() - Duration::from_secs(1),
        OffsetDateTime::now_utc() + Duration::from_secs(30),
    );
    f.store
        .create_recording_schedule(&failed)
        .await
        .expect("schedule");
    service.poll_once().await.expect("poll");
    let recording = wait_for_terminal(&f, &failed).await;
    assert_eq!(recording.status, RecordingStatus::Failed);
    assert!(recording.error.as_deref().is_some_and(|e| e.contains('9')));
    assert!(
        !f.root
            .path()
            .join("recordings")
            .join(format!("{}.part.mkv", recording.id))
            .exists()
    );
}

#[tokio::test]
async fn dvr_cancellation_kills_reaps_and_stays_cancelled() {
    let f = fixture().await;
    let ffmpeg = f.root.path().join("ffmpeg-blocking");
    let pid_file = f.root.path().join("ffmpeg.pid");
    executable(
        &ffmpeg,
        &format!(
            "#!/bin/sh\necho $$ > '{}'\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf partial > \"$out\"\nsleep 30\n",
            pid_file.display()
        ),
    );
    let service = runtime(&f, ffmpeg).await;
    let scheduled = schedule(
        &f,
        OffsetDateTime::now_utc() - Duration::from_secs(1),
        OffsetDateTime::now_utc() + Duration::from_secs(30),
    );
    f.store
        .create_recording_schedule(&scheduled)
        .await
        .expect("schedule");
    service.poll_once().await.expect("poll");
    wait_for(|| pid_file.exists()).await;
    let pid = std::fs::read_to_string(&pid_file).expect("pid");
    service
        .cancel_schedule(f.owner.id, scheduled.id)
        .await
        .expect("cancel");
    let cancelled = wait_for_terminal(&f, &scheduled).await;
    assert_eq!(cancelled.status, RecordingStatus::Cancelled);
    wait_for(|| {
        !std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        recording_for(&f, &scheduled).await.status,
        RecordingStatus::Cancelled
    );
    assert!(
        !f.root
            .path()
            .join("recordings")
            .join(format!("{}.part.mkv", cancelled.id))
            .exists()
    );
    assert!(
        !f.root
            .path()
            .join("recordings")
            .join(format!("{}.mkv", cancelled.id))
            .exists()
    );
}

#[tokio::test]
async fn dvr_uses_late_remaining_duration_and_does_not_spawn_expired() {
    let f = fixture().await;
    let args_file = f.root.path().join("args");
    let marker = f.root.path().join("spawned");
    let ffmpeg = f.root.path().join("ffmpeg-args");
    executable(
        &ffmpeg,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ntouch '{}'\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf data > \"$out\"\n",
            args_file.display(),
            marker.display()
        ),
    );
    let service = runtime(&f, ffmpeg).await;
    let late = schedule(
        &f,
        OffsetDateTime::now_utc() - Duration::from_secs(30),
        OffsetDateTime::now_utc() + Duration::from_secs(5),
    );
    f.store
        .create_recording_schedule(&late)
        .await
        .expect("late schedule");
    service.poll_once().await.expect("poll late");
    assert_eq!(
        wait_for_terminal(&f, &late).await.status,
        RecordingStatus::Completed
    );
    let args = std::fs::read_to_string(&args_file).expect("args");
    let args = args.lines().collect::<Vec<_>>();
    let duration = args[args.iter().position(|arg| *arg == "-t").expect("-t") + 1]
        .parse::<f64>()
        .expect("duration");
    assert!(duration > 0.0 && duration <= 5.0, "duration was {duration}");

    std::fs::remove_file(&marker).expect("clear marker");
    let expired = schedule(
        &f,
        OffsetDateTime::now_utc() - Duration::from_secs(30),
        OffsetDateTime::now_utc() - Duration::from_secs(1),
    );
    f.store
        .create_recording_schedule(&expired)
        .await
        .expect("expired schedule");
    service.poll_once().await.expect("poll expired");
    let recording = wait_for_terminal(&f, &expired).await;
    assert_eq!(recording.status, RecordingStatus::Failed);
    assert!(
        recording
            .error
            .as_deref()
            .is_some_and(|error| error.contains("window elapsed"))
    );
    assert!(!marker.exists());
}

fn jwt() -> Arc<JwtConfig> {
    Arc::new(JwtConfig::dev(JWT_SECRET).expect("JWT config"))
}

fn transcoder_config() -> Arc<TranscoderConfig> {
    Arc::new(TranscoderConfig {
        hwaccel: HwAccel::None,
        fallback: TranscodeFallback::Auto,
        ladder: vec![LadderEntry {
            max_height: 720,
            video_codec: VideoCodec::H264,
            audio_codec: AudioCodec::Aac,
            container: "mpegts".into(),
            bitrate_kbps: 4_000,
        }],
    })
}

// Each test user gets one session keyed by the user's own UUID, so
// `bearer` can mint a session-bound token without a storage handle.
fn session_id(user: &User) -> noflex_core::SessionId {
    noflex_core::SessionId::from_uuid(user.id.into_uuid())
}

async fn create_session(store: &dyn Storage, user: &User) {
    let now = OffsetDateTime::now_utc();
    store
        .create_session(&noflex_storage::domain::Session {
            id: session_id(user),
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
        })
        .await
        .expect("session");
}

fn bearer(user: &User) -> String {
    format!(
        "Bearer {}",
        issue_jwt(&jwt(), user.id, user.is_admin, session_id(user)).expect("JWT")
    )
}

fn request(path: &str, user: Option<&User>, range: Option<&str>) -> Request<Body> {
    let mut request = Request::builder().uri(path);
    if let Some(user) = user {
        request = request.header(header::AUTHORIZATION, bearer(user));
    }
    if let Some(range) = range {
        request = request.header(header::RANGE, range);
    }
    request.body(Body::empty()).expect("request")
}

#[tokio::test]
async fn start_live_tv_channel_creates_typed_hls_job_with_scoped_access() {
    let f = fixture().await;
    let ffmpeg = f.root.path().join("hls-ffmpeg");
    executable(
        &ffmpeg,
        "#!/bin/sh\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf '#EXTM3U\\n#EXT-X-ENDLIST\\n' > \"$out\"\nsleep 30\n",
    );
    let manager = Arc::new(FfmpegJobManager::new(ffmpeg, f.root.path().join("hls")));
    let sessions = Arc::new(ServerPlaybackSessions::new(
        f.store.clone(),
        manager.clone(),
        transcoder_config(),
        HwBackend::Software,
        None,
        jwt(),
    ));
    let live_tv = Arc::new(
        ServerLiveTv::new(
            f.store.clone(),
            sessions.clone(),
            f.root.path().join("unused-dvr"),
            f.root.path().into(),
            1,
        )
        .expect("live TV"),
    );
    let schema = build_schema(SchemaContext {
        storage: Some(f.store.clone()),
        live_tv: Some(live_tv),
        ..Default::default()
    });
    let response = schema
        .execute(
            GraphqlRequest::new(format!(
                "mutation {{ startLiveTvChannel(channelId: \"{}\") {{ route url jobId }} }}",
                f.channel.id
            ))
            .data(RequestUser {
                user_id: f.owner.id,
                is_admin: false,
            }),
        )
        .await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    let data = response.data.into_json().expect("data");
    let job = data["startLiveTvChannel"]["jobId"]
        .as_str()
        .expect("job id");
    assert_eq!(
        sessions.playback_job_target(job).await,
        Some(PlaybackJobTarget::LiveChannel(f.channel.id))
    );
    let app = routes(RestrictedRouteState {
        storage: f.store.clone(),
        jwt: jwt(),
        transcoder: manager,
        playback_job_authorizer: sessions,
        transcode_cache_dir: f.root.path().join("hls"),
        image_cache_dir: f.root.path().join("images"),
        recordings_dir: f.root.path().join("recordings"),
    });
    let path = format!("/stream/job/{job}/master.m3u8");
    for user in [&f.owner, &f.admin] {
        let response = app
            .clone()
            .oneshot(request(&path, Some(user), None))
            .await
            .expect("route");
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(
        app.clone()
            .oneshot(request(&path, Some(&f.foreign), None))
            .await
            .expect("route")
            .status(),
        StatusCode::NOT_FOUND
    );
    let exact = issue_media_token(
        &jwt(),
        f.owner.id,
        None,
        &format!("job:{job}"),
        Duration::from_secs(60),
    )
    .expect("media token");
    assert_eq!(
        app.clone()
            .oneshot(request(&format!("{path}?token={exact}"), None, None))
            .await
            .expect("route")
            .status(),
        StatusCode::OK
    );
    let foreign = issue_media_token(
        &jwt(),
        f.foreign.id,
        None,
        &format!("job:{job}"),
        Duration::from_secs(60),
    )
    .expect("media token");
    assert_eq!(
        app.clone()
            .oneshot(request(&format!("{path}?token={foreign}"), None, None))
            .await
            .expect("route")
            .status(),
        StatusCode::NOT_FOUND,
        "a token is only as good as its user's access"
    );
}

async fn completed_recording(f: &Fixture, owner: noflex_core::UserId, path: &Path) -> Recording {
    let mut scheduled = schedule(
        f,
        OffsetDateTime::now_utc() - Duration::from_secs(2),
        OffsetDateTime::now_utc() + Duration::from_secs(30),
    );
    scheduled.owner_id = owner;
    f.store
        .create_recording_schedule(&scheduled)
        .await
        .expect("schedule");
    let (_, recording) = f
        .store
        .claim_due_recording(OffsetDateTime::now_utc())
        .await
        .expect("claim")
        .expect("due");
    let path = path.to_string_lossy();
    assert!(
        f.store
            .finish_recording(
                recording.id,
                RecordingStatus::Completed,
                Some(&path),
                None,
                Some(10),
                OffsetDateTime::now_utc(),
            )
            .await
            .expect("finish")
    );
    f.store
        .get_recording(recording.id)
        .await
        .expect("lookup")
        .expect("recording")
}

fn recording_router(f: &Fixture, sessions: Arc<ServerPlaybackSessions>) -> Router {
    let manager = Arc::new(FfmpegJobManager::new(
        f.root.path().join("unused"),
        f.root.path().join("hls"),
    ));
    routes(RestrictedRouteState {
        storage: f.store.clone(),
        jwt: jwt(),
        transcoder: manager,
        playback_job_authorizer: sessions,
        transcode_cache_dir: f.root.path().join("hls"),
        image_cache_dir: f.root.path().join("images"),
        recordings_dir: f.root.path().join("recordings"),
    })
}

#[tokio::test]
async fn recording_file_route_enforces_owner_admin_range_state_and_confinement() {
    let f = fixture().await;
    let recordings = f.root.path().join("recordings");
    std::fs::create_dir_all(&recordings).expect("recordings");
    let file = recordings.join("show.mkv");
    std::fs::write(&file, b"0123456789").expect("recording file");
    let completed = completed_recording(&f, f.owner.id, &file).await;
    let sessions = Arc::new(ServerPlaybackSessions::new(
        f.store.clone(),
        Arc::new(FfmpegJobManager::new(
            f.root.path().join("unused"),
            f.root.path().join("hls"),
        )),
        transcoder_config(),
        HwBackend::Software,
        None,
        jwt(),
    ));
    let app = recording_router(&f, sessions);
    let path = format!("/recordings/{}/file", completed.id);
    for user in [&f.owner, &f.admin] {
        assert_eq!(
            app.clone()
                .oneshot(request(&path, Some(user), None))
                .await
                .expect("route")
                .status(),
            StatusCode::OK
        );
    }
    let response = app
        .clone()
        .oneshot(request(&path, Some(&f.owner), Some("bytes=2-5")))
        .await
        .expect("range");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
        "2345"
    );
    assert_eq!(
        app.clone()
            .oneshot(request(&path, Some(&f.foreign), None))
            .await
            .expect("route")
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.clone()
            .oneshot(request(&path, None, None))
            .await
            .expect("route")
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let outside = f.root.path().join("outside.mkv");
    std::fs::write(&outside, b"outside").expect("outside");
    let escaped = completed_recording(&f, f.owner.id, &outside).await;
    assert_eq!(
        app.clone()
            .oneshot(request(
                &format!("/recordings/{}/file", escaped.id),
                Some(&f.owner),
                None,
            ))
            .await
            .expect("route")
            .status(),
        StatusCode::NOT_FOUND
    );

    let scheduled = schedule(
        &f,
        OffsetDateTime::now_utc() - Duration::from_secs(1),
        OffsetDateTime::now_utc() + Duration::from_secs(30),
    );
    f.store
        .create_recording_schedule(&scheduled)
        .await
        .expect("schedule");
    let (_, running) = f
        .store
        .claim_due_recording(OffsetDateTime::now_utc())
        .await
        .expect("claim")
        .expect("running");
    assert_eq!(
        app.oneshot(request(
            &format!("/recordings/{}/file", running.id),
            Some(&f.owner),
            None,
        ))
        .await
        .expect("route")
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn delete_recording_is_file_first_and_confined() {
    let f = fixture().await;
    let recordings = f.root.path().join("recordings");
    std::fs::create_dir_all(&recordings).expect("recordings");
    let service = runtime(&f, f.root.path().join("unused-ffmpeg")).await;

    let recording = claim_recording(&f).await;
    let path = recordings.join(format!("{}.mkv", recording.id));
    std::fs::write(&path, b"delete me").expect("file");
    let recording = finish_at(&f, recording, &path).await;
    service
        .delete_recording(f.owner.id, recording.id, true)
        .await
        .expect("delete");
    assert!(!path.exists());
    assert!(
        f.store
            .get_recording(recording.id)
            .await
            .expect("lookup")
            .is_none()
    );

    let recording = claim_recording(&f).await;
    let directory_path = recordings.join(format!("{}.mkv", recording.id));
    std::fs::create_dir(&directory_path).expect("directory obstacle");
    let recording = finish_at(&f, recording, &directory_path).await;
    assert!(
        service
            .delete_recording(f.owner.id, recording.id, true)
            .await
            .is_err()
    );
    assert!(
        f.store
            .get_recording(recording.id)
            .await
            .expect("lookup")
            .is_some()
    );

    let recording = claim_recording(&f).await;
    let outside = f.root.path().join(format!("{}.mkv", recording.id));
    std::fs::write(&outside, b"keep me").expect("outside");
    let recording = finish_at(&f, recording, &outside).await;
    assert!(
        service
            .delete_recording(f.owner.id, recording.id, true)
            .await
            .is_err()
    );
    assert!(outside.exists());
    assert!(
        f.store
            .get_recording(recording.id)
            .await
            .expect("lookup")
            .is_some()
    );
}

async fn claim_recording(f: &Fixture) -> Recording {
    let scheduled = schedule(
        f,
        OffsetDateTime::now_utc() - Duration::from_secs(1),
        OffsetDateTime::now_utc() + Duration::from_secs(30),
    );
    f.store
        .create_recording_schedule(&scheduled)
        .await
        .expect("schedule");
    f.store
        .claim_due_recording(OffsetDateTime::now_utc())
        .await
        .expect("claim")
        .expect("due")
        .1
}

async fn finish_at(f: &Fixture, recording: Recording, path: &Path) -> Recording {
    assert!(
        f.store
            .finish_recording(
                recording.id,
                RecordingStatus::Completed,
                Some(&path.to_string_lossy()),
                None,
                Some(7),
                OffsetDateTime::now_utc(),
            )
            .await
            .expect("finish")
    );
    f.store
        .get_recording(recording.id)
        .await
        .expect("lookup")
        .expect("recording")
}
