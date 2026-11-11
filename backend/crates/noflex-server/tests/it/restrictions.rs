// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::Path, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use noflex_auth::{JwtConfig, issue_jwt, issue_media_token};
use noflex_core::{AudioCodec, ImageKind, TagTarget, UserRole, VideoCodec};
use noflex_graphql::{PlaybackRouteKind, PlaybackSessionService, PlaybackStart};
use noflex_server::{
    config::{HwAccel, LadderEntry, TranscodeFallback, TranscoderConfig},
    restricted_routes::{RestrictedRouteState, routes},
    stream::ServerPlaybackSessions,
};
use noflex_storage::{
    Storage,
    domain::{
        Edition, Image, ImageSource, ImageTarget, Library, LibraryKind, MediaFile,
        OptimizedVersion, OptimizedVersionStatus, Tag, TrickplayAsset, TrickplaySheet, User,
        UserRestrictions, Work,
    },
};
use noflex_storage_turso::TursoStore;
use noflex_transcoder::{
    FfmpegJobManager, HwBackend, JobStatus,
    decision::ClientCapabilities,
    probe::{AudioStreamProbe, ProbeReport, VideoStreamProbe},
};
use time::OffsetDateTime;
use tower::ServiceExt;

const JWT_SECRET: &[u8] = b"restriction-acceptance-tests-key-32b";

struct Fixture {
    store: Arc<dyn Storage>,
    owner: User,
    foreign: User,
    admin: User,
    library: Library,
    work: Work,
    edition: Edition,
    media: MediaFile,
    rating: Tag,
    denied_rating: Tag,
    root: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let root = tempfile::tempdir().expect("tempdir");
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store: Arc<dyn Storage> = Arc::new(store);
    let now = OffsetDateTime::now_utc();
    let library = Library {
        id: noflex_core::LibraryId::new(),
        name: "Restricted".into(),
        kind: LibraryKind::Movies,
        paths: vec![root.path().to_string_lossy().into_owned()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let owner = user("owner", false, now);
    let foreign = user("foreign", false, now);
    let admin = user("admin", true, now);
    for user in [&owner, &foreign, &admin] {
        store.upsert_user(user).await.expect("user");
        create_session(store.as_ref(), user).await;
    }
    for user in [&owner, &foreign] {
        store
            .grant_library_access(user.id, library.id)
            .await
            .expect("grant");
    }
    let work = Work {
        id: noflex_core::WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Policy Movie".into(),
        sort_title: "policy movie".into(),
        year: Some(2026),
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
        id: noflex_core::EditionId::new(),
        work_id: work.id,
        name: "Default".into(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    store.upsert_edition(&edition).await.expect("edition");
    let media_path = root.path().join("movie.mp4");
    std::fs::write(&media_path, b"media bytes").expect("media fixture");
    let media = MediaFile {
        id: noflex_core::MediaFileId::new(),
        edition_id: edition.id,
        path: media_path.to_string_lossy().into_owned(),
        part_index: 0,
        size_bytes: 11,
        mtime: now,
        container: Some("mp4".into()),
        probe: serde_json::to_value(probe()).expect("probe").into(),
        subtitle_tracks: vec![],
    };
    store.upsert_media_file(&media).await.expect("media");
    let rating = Tag {
        id: noflex_core::TagId::new(),
        namespace: "parental".into(),
        parent_id: None,
        slug: "pg-13".into(),
        display: "PG-13".into(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    store.upsert_tag(&rating).await.expect("rating");
    let denied_rating = Tag {
        id: noflex_core::TagId::new(),
        namespace: "parental".into(),
        parent_id: None,
        slug: "r".into(),
        display: "R".into(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    store
        .upsert_tag(&denied_rating)
        .await
        .expect("denied rating");
    store
        .assign_tag(rating.id, TagTarget::Work(work.id), 1.0, "test")
        .await
        .expect("assign rating");
    for user in [&owner, &foreign] {
        set_policy(&store, user.id, vec![rating.id], None, None).await;
    }
    seed_images_and_trickplay(&store, root.path(), &work, &edition, now).await;
    Fixture {
        store,
        owner,
        foreign,
        admin,
        library,
        work,
        edition,
        media,
        rating,
        denied_rating,
        root,
    }
}

fn user(name: &str, admin: bool, now: OffsetDateTime) -> User {
    User {
        id: noflex_core::UserId::new(),
        username: name.into(),
        display_name: None,
        password_hash: String::new(),
        is_admin: admin,
        role: if admin {
            UserRole::Admin
        } else {
            UserRole::User
        },
        created_at: now,
        avatar_updated_at: None,
        has_pin: false,
    }
}

async fn set_policy(
    store: &Arc<dyn Storage>,
    user: noflex_core::UserId,
    ratings: Vec<noflex_core::TagId>,
    bitrate: Option<u64>,
    concurrent: Option<u32>,
) {
    store
        .set_user_restrictions(&UserRestrictions {
            user_id: user,
            allowed_parental_rating_ids: ratings,
            allow_unrated: false,
            max_stream_bitrate_bps: bitrate,
            max_concurrent_transcodes: concurrent,
        })
        .await
        .expect("policy");
}

async fn seed_images_and_trickplay(
    store: &Arc<dyn Storage>,
    root: &Path,
    work: &Work,
    edition: &Edition,
    now: OffsetDateTime,
) {
    let png = root.join("image/poster.png");
    std::fs::create_dir_all(png.parent().unwrap()).expect("image dir");
    image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
        .save(&png)
        .expect("image");
    for target in [ImageTarget::Work(work.id), ImageTarget::Edition(edition.id)] {
        store
            .upsert_image(&Image {
                target,
                kind: ImageKind::Primary,
                source: ImageSource::Local,
                path: png.to_string_lossy().into_owned(),
                width: Some(2),
                height: Some(2),
                language: None,
                provider: None,
            })
            .await
            .expect("image row");
    }
    let dir = root
        .join("image/trickplay")
        .join(edition.id.to_string())
        .join("fixture-version");
    std::fs::create_dir_all(&dir).expect("trickplay dir");
    let vtt = dir.join("index.vtt");
    let sheet = dir.join("sheet-0000.jpg");
    std::fs::write(
        &vtt,
        format!(
            "WEBVTT\n\n00:00.000 --> 00:10.000\n/trickplay/{}/sheets/0#xywh=0,0,160,90\n",
            edition.id
        ),
    )
    .expect("vtt");
    std::fs::write(&sheet, b"jpeg").expect("sheet");
    store
        .upsert_trickplay(&TrickplayAsset {
            edition_id: edition.id,
            vtt_path: vtt.to_string_lossy().into_owned(),
            thumb_width: 160,
            thumb_height: 90,
            tile_columns: 1,
            tile_rows: 1,
            interval_ms: 10_000,
            source_media_mtime: now,
            source_fingerprint: "fixture".into(),
            generated_at: now,
            sheets: vec![TrickplaySheet {
                edition_id: edition.id,
                index: 0,
                path: sheet.to_string_lossy().into_owned(),
                tile_count: 1,
            }],
            chapter_cues: vec![],
        })
        .await
        .expect("trickplay row");
}

fn probe() -> ProbeReport {
    ProbeReport {
        container: Some("mp4".into()),
        video: Some(VideoStreamProbe {
            codec: VideoCodec::H264,
            profile: Some(noflex_core::VideoProfile::H264High),
            level: Some(4.0),
            width: 1920,
            height: 1080,
            bit_depth: Some(8),
            pixel_format: Some(noflex_core::PixelFormat::Yuv420p),
            frame_rate: Some(24.0),
            color_space: Some(noflex_core::ColorPrimaries::Bt709),
            transfer: Some(noflex_core::HdrTransfer::Sdr),
            matrix: Some(noflex_core::MatrixCoefficients::Bt709),
            dovi_profile: None,
            side_data: Default::default(),
        }),
        audio: vec![AudioStreamProbe {
            index: 1,
            codec: "aac".into(),
            profile: Some("LC".into()),
            bit_depth: Some(16),
            sample_rate: Some(48_000),
            channels: 2,
            channel_layout: Some(noflex_core::ChannelLayout::Stereo),
            language: Some("eng".into()),
            default: true,
            forced: false,
            title: None,
            audio_object_type: None,
        }],
        subtitle: vec![],
        format_bit_rate: Some(5_000_000),
        duration_ms: None,
        chapters: Vec::new(),
    }
}

fn jwt() -> Arc<JwtConfig> {
    Arc::new(JwtConfig::dev(JWT_SECRET).expect("JWT config"))
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
            locked_at: None,
        })
        .await
        .expect("session");
}

fn bearer(user: &User) -> String {
    let token = issue_jwt(&jwt(), user.id, user.is_admin, session_id(user)).expect("jwt");
    format!("Bearer {token}")
}

fn request(path: &str, user: Option<&User>) -> Request<Body> {
    let mut builder = Request::builder().uri(path);
    if let Some(user) = user {
        builder = builder.header(header::AUTHORIZATION, bearer(user));
    }
    builder.body(Body::empty()).expect("request")
}

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("permissions");
    }
}

fn config() -> Arc<TranscoderConfig> {
    Arc::new(TranscoderConfig {
        hwaccel: HwAccel::None,
        fallback: TranscodeFallback::Auto,
        optimize: Default::default(),
        tonemap_curve: Default::default(),
        ladder: vec![ladder(480, 2_000), ladder(720, 4_000), ladder(1080, 8_000)],
    })
}

fn ladder(height: u32, bitrate_kbps: u32) -> LadderEntry {
    LadderEntry {
        max_pixels: u64::from(height) * u64::from(height) * 16 / 9,
        video_codec: VideoCodec::H264,
        video_profile: noflex_core::VideoProfile::H264High,
        audio_codec: AudioCodec::Aac,
        container: "mpegts".into(),
        bitrate_kbps,
    }
}

fn service(
    store: &Arc<dyn Storage>,
    manager: &Arc<FfmpegJobManager>,
) -> Arc<ServerPlaybackSessions> {
    Arc::new(ServerPlaybackSessions::new(
        store.clone(),
        manager.clone(),
        config(),
        HwBackend::Software,
        None,
        jwt(),
    ))
}

fn router(
    fixture: &Fixture,
    manager: Arc<FfmpegJobManager>,
    sessions: Arc<ServerPlaybackSessions>,
) -> Router {
    routes(RestrictedRouteState {
        storage: fixture.store.clone(),
        jwt: jwt(),
        transcoder: manager,
        playback_job_authorizer: sessions,
        transcode_cache_dir: fixture.root.path().join("jobs"),
        image_cache_dir: fixture.root.path().join("image"),
        recordings_dir: fixture.root.path().join("recordings"),
        subtitles: None,
    })
}

async fn assert_status(app: &Router, path: &str, user: Option<&User>, expected: StatusCode) {
    let response = app
        .clone()
        .oneshot(request(path, user))
        .await
        .expect("route");
    assert_eq!(response.status(), expected, "{path}");
}

#[tokio::test]
async fn media_image_and_trickplay_headers_require_current_work_access() {
    let f = fixture().await;
    let ffmpeg = f.root.path().join("sleep-ffmpeg");
    executable(&ffmpeg, "#!/bin/sh\nsleep 30\n");
    let manager = Arc::new(FfmpegJobManager::new(ffmpeg, f.root.path().join("jobs")));
    let app = router(&f, manager.clone(), service(&f.store, &manager));
    let paths = [
        format!("/stream/{}/file", f.media.id),
        format!("/images/work/{}/primary", f.work.id),
        format!("/images/edition/{}/primary", f.edition.id),
        format!("/trickplay/{}/index.vtt", f.edition.id),
        format!("/trickplay/{}/sheets/0", f.edition.id),
    ];
    for path in &paths {
        assert_status(&app, path, Some(&f.owner), StatusCode::OK).await;
    }

    f.store
        .revoke_library_access(f.owner.id, f.library.id)
        .await
        .expect("revoke library");
    for path in &paths {
        assert_status(&app, path, Some(&f.owner), StatusCode::NOT_FOUND).await;
    }
    f.store
        .grant_library_access(f.owner.id, f.library.id)
        .await
        .expect("restore library");
    set_policy(&f.store, f.owner.id, vec![f.denied_rating.id], None, None).await;
    for path in &paths {
        assert_status(&app, path, Some(&f.owner), StatusCode::NOT_FOUND).await;
    }
}

#[tokio::test]
async fn hls_routes_enforce_owner_admin_policy_and_exact_scoped_token() {
    let f = fixture().await;
    let ffmpeg = f.root.path().join("sleep-ffmpeg");
    executable(&ffmpeg, "#!/bin/sh\nsleep 30\n");
    let manager = Arc::new(FfmpegJobManager::new(ffmpeg, f.root.path().join("jobs")));
    let sessions = service(&f.store, &manager);
    let mut caps = ClientCapabilities::web();
    caps.max_height = Some(480);
    let session = sessions
        .start(f.owner.id, None, PlaybackStart::new(f.media.id, caps))
        .await
        .expect("start");
    let job_id = session.job_id.expect("job id").to_string();
    let job = manager.registry().get(&job_id).await.expect("job");
    std::fs::write(job.output_dir.join("segment_00000.ts"), b"segment").expect("segment");
    let app = router(&f, manager, sessions);
    let master = format!("/stream/job/{job_id}/master.m3u8");
    let segment = format!("/stream/job/{job_id}/segment_00000.ts");

    for path in [&master, &segment] {
        assert_status(&app, path, Some(&f.owner), StatusCode::OK).await;
        assert_status(&app, path, Some(&f.admin), StatusCode::OK).await;
        assert_status(&app, path, Some(&f.foreign), StatusCode::NOT_FOUND).await;
    }
    set_policy(&f.store, f.owner.id, vec![f.denied_rating.id], None, None).await;
    assert_status(&app, &master, Some(&f.owner), StatusCode::NOT_FOUND).await;

    // A token replaces the header credential, not authorization: the
    // tightened policy applies to it immediately.
    let token = issue_media_token(
        &jwt(),
        f.owner.id,
        None,
        &format!("job:{job_id}"),
        Duration::from_secs(60),
    )
    .expect("media token");
    for path in [&master, &segment] {
        assert_status(
            &app,
            &format!("{path}?token={token}"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    set_policy(&f.store, f.owner.id, vec![f.rating.id], None, None).await;
    assert_status(
        &app,
        &format!("{master}?token={token}"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_status(
        &app,
        &format!("{segment}?token={token}"),
        None,
        StatusCode::OK,
    )
    .await;
    let wrong = issue_media_token(
        &jwt(),
        f.owner.id,
        None,
        "job:foreign",
        Duration::from_secs(60),
    )
    .expect("token");
    assert_status(
        &app,
        &format!("{master}?token={wrong}"),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn a_picture_is_for_those_signed_in_with_a_token_for_that_picture() {
    let f = fixture().await;
    let manager = Arc::new(FfmpegJobManager::new(
        f.root.path().join("ffmpeg"),
        f.root.path().join("jobs"),
    ));
    let app = router(&f, manager.clone(), service(&f.store, &manager));
    let signer = noflex_server::stream::JwtMediaUrlSigner::new(jwt());
    let token = |of: &User| {
        noflex_storage::MediaUrlSigner::sign_avatar_token(&signer, f.foreign.id, None, of.id)
            .expect("token")
    };
    let picture = format!("/avatars/{}", f.owner.id);

    assert_status(&app, &picture, Some(&f.foreign), StatusCode::NOT_FOUND).await;
    f.store
        .set_user_avatar(f.owner.id, b"jpeg", OffsetDateTime::now_utc())
        .await
        .expect("avatar");

    assert_status(&app, &picture, None, StatusCode::UNAUTHORIZED).await;
    assert_status(
        &app,
        &format!("{picture}?token={}", token(&f.foreign)),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let response = app
        .clone()
        .oneshot(request(
            &format!("{picture}?v=1&token={}", token(&f.owner)),
            None,
        ))
        .await
        .expect("route");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
}

#[tokio::test]
async fn media_and_image_tokens_carry_their_users_restrictions() {
    let f = fixture().await;
    let ffmpeg = f.root.path().join("sleep-ffmpeg");
    executable(&ffmpeg, "#!/bin/sh\nsleep 30\n");
    let manager = Arc::new(FfmpegJobManager::new(ffmpeg, f.root.path().join("jobs")));
    let app = router(&f, manager.clone(), service(&f.store, &manager));
    let signer = noflex_server::stream::JwtMediaUrlSigner::new(jwt());
    let image_token = |user: &User| {
        noflex_storage::MediaUrlSigner::sign_image_token(
            &signer,
            user.id,
            None,
            ImageTarget::Work(f.work.id),
        )
        .expect("image token")
    };
    let file_token = |user: &User| {
        issue_media_token(
            &jwt(),
            user.id,
            None,
            &format!("file:{}", f.media.id),
            Duration::from_secs(60),
        )
        .expect("file token")
    };
    let image = format!("/images/work/{}/primary", f.work.id);
    let file = format!("/stream/{}/file", f.media.id);

    // No Authorization header: the token alone authenticates.
    let response = app
        .clone()
        .oneshot(request(
            &format!("{image}?token={}", image_token(&f.owner)),
            None,
        ))
        .await
        .expect("route");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, max-age=86400"
    );
    assert_status(
        &app,
        &format!("{image}?w=1&token={}", image_token(&f.owner)),
        None,
        StatusCode::OK,
    )
    .await;
    assert_status(
        &app,
        &format!("{file}?token={}", file_token(&f.owner)),
        None,
        StatusCode::OK,
    )
    .await;
    // Image tokens are scoped to their target.
    assert_status(
        &app,
        &format!(
            "/images/edition/{}/primary?token={}",
            f.edition.id,
            image_token(&f.owner)
        ),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;

    // A restricted user's tokens don't reach restricted works, so a
    // shared link can't bypass the recipient's — or sender's — policy.
    set_policy(&f.store, f.foreign.id, vec![f.denied_rating.id], None, None).await;
    assert_status(
        &app,
        &format!("{image}?token={}", image_token(&f.foreign)),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_status(
        &app,
        &format!("{file}?token={}", file_token(&f.foreign)),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;

    // Admin tokens bypass restrictions like admin headers do.
    assert_status(
        &app,
        &format!("{image}?token={}", image_token(&f.admin)),
        None,
        StatusCode::OK,
    )
    .await;
}

#[tokio::test]
async fn a_ready_version_offers_its_file_beside_its_playlist() {
    let f = fixture().await;
    set_policy(
        &f.store,
        f.owner.id,
        vec![f.rating.id],
        Some(3_000_000),
        None,
    )
    .await;
    let mut within_policy = probe();
    within_policy.format_bit_rate = Some(2_000_000);
    let version = OptimizedVersion {
        id: noflex_core::OptimizedVersionId::new(),
        media_file_id: f.media.id,
        variant_key: "h264-1080".to_string(),
        status: OptimizedVersionStatus::Ready,
        priority: 1,
        plan: serde_json::json!({}),
        source_size: f.media.size_bytes,
        source_mtime: f.media.mtime,
        progress: 1.0,
        attempts: 0,
        path: None,
        size_bytes: Some(1),
        index: None,
        probe: Some(serde_json::to_value(within_policy).expect("probe")),
        error: None,
        created_at: OffsetDateTime::now_utc(),
        finished_at: None,
        last_used_at: None,
        unwanted_since: None,
    };
    f.store
        .enqueue_optimized_version(&version)
        .await
        .expect("enqueue");
    f.store
        .update_optimized_version(&version)
        .await
        .expect("ready");
    let manager = Arc::new(FfmpegJobManager::new(
        f.root.path().join("no-ffmpeg"),
        f.root.path().join("jobs"),
    ));
    let session = service(&f.store, &manager)
        .start(
            f.owner.id,
            None,
            PlaybackStart::new(f.media.id, ClientCapabilities::web()),
        )
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Optimized);
    let id = version.id;
    assert!(
        session
            .url
            .starts_with(&format!("/optimized/{id}/master.m3u8?token="))
    );
    let file = session.file_url.expect("file");
    assert!(file.starts_with(&format!("/optimized/{id}/file?token=")));
}

#[tokio::test]
async fn playback_policy_clamps_capabilities_and_ladder_bitrate() {
    let f = fixture().await;
    set_policy(
        &f.store,
        f.owner.id,
        vec![f.rating.id],
        Some(3_000_000),
        None,
    )
    .await;
    let ffmpeg = f.root.path().join("sleep-ffmpeg");
    executable(&ffmpeg, "#!/bin/sh\nsleep 30\n");
    let manager = Arc::new(FfmpegJobManager::new(ffmpeg, f.root.path().join("jobs")));
    let sessions = service(&f.store, &manager);
    let mut caps = ClientCapabilities::web();
    caps.max_bitrate_bps = Some(12_000_000);
    let session = sessions
        .start(f.owner.id, None, PlaybackStart::new(f.media.id, caps))
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Transcode);
    assert_eq!(session.file_url, None);
    let job_id = session.job_id.expect("job");
    let job = manager
        .registry()
        .get(job_id.as_ref())
        .await
        .expect("registered");
    assert!(job.request.ladder.bitrate_kbps <= 3_000);
    assert_eq!(job.request.ladder.bitrate_kbps, 2_000);
}

#[tokio::test]
async fn concurrent_limit_is_atomic_and_terminal_or_stopped_jobs_release_it() {
    let f = fixture().await;
    set_policy(&f.store, f.owner.id, vec![f.rating.id], None, Some(1)).await;
    let ffmpeg = f.root.path().join("sleep-ffmpeg");
    executable(&ffmpeg, "#!/bin/sh\nsleep 30\n");
    let manager = Arc::new(FfmpegJobManager::new(ffmpeg, f.root.path().join("jobs")));
    let sessions = service(&f.store, &manager);
    let mut caps = ClientCapabilities::web();
    caps.max_height = Some(480);
    let (left, right) = tokio::join!(
        sessions.start(
            f.owner.id,
            None,
            PlaybackStart::new(f.media.id, caps.clone())
        ),
        sessions.start(
            f.owner.id,
            None,
            PlaybackStart::new(f.media.id, caps.clone())
        )
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let running = left.or(right).expect("one start");
    let running_job = running.job_id.expect("job");
    sessions
        .stop(f.owner.id, running_job.as_ref())
        .await
        .expect("stop");
    let replacement = sessions
        .start(
            f.owner.id,
            None,
            PlaybackStart::new(f.media.id, caps.clone()),
        )
        .await
        .expect("capacity after stop");
    let replacement_job = replacement.job_id.expect("replacement job");
    sessions
        .stop(f.owner.id, replacement_job.as_ref())
        .await
        .expect("stop replacement");

    let complete = f.root.path().join("complete-ffmpeg");
    executable(&complete, "#!/bin/sh\nexit 0\n");
    let manager = Arc::new(FfmpegJobManager::new(
        complete,
        f.root.path().join("complete-jobs"),
    ));
    let sessions = service(&f.store, &manager);
    let first = sessions
        .start(
            f.owner.id,
            None,
            PlaybackStart::new(f.media.id, caps.clone()),
        )
        .await
        .expect("completing start");
    let completed_job = first.job_id.expect("completed job");
    wait_terminal(&manager, completed_job.as_ref(), JobStatus::Completed).await;
    sessions
        .start(
            f.owner.id,
            None,
            PlaybackStart::new(f.media.id, caps.clone()),
        )
        .await
        .expect("capacity after completion");

    let missing = f.root.path().join("missing-then-created-ffmpeg");
    let manager = Arc::new(FfmpegJobManager::new(
        &missing,
        f.root.path().join("failed-jobs"),
    ));
    let sessions = service(&f.store, &manager);
    sessions
        .start(
            f.owner.id,
            None,
            PlaybackStart::new(f.media.id, caps.clone()),
        )
        .await
        .expect_err("missing ffmpeg fails start");
    executable(&missing, "#!/bin/sh\nsleep 30\n");
    sessions
        .start(f.owner.id, None, PlaybackStart::new(f.media.id, caps))
        .await
        .expect("failed start released reservation");
}

async fn wait_terminal(manager: &FfmpegJobManager, job_id: &str, expected: JobStatus) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let status = manager.registry().get(job_id).await.expect("job").status;
        if status != JobStatus::Running {
            assert_eq!(status, expected);
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "job stayed running");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
