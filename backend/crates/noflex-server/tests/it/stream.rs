// SPDX-License-Identifier: AGPL-3.0-only
//
// Streaming tests drive `ServerPlaybackSessions` (the GraphQL session
// control plane) and the byte-serving functions in
// `noflex_server::stream` directly (same pattern as
// `tests/it/download.rs`), backed by an in-memory Turso store.

use std::{sync::Arc, time::Duration};

use noflex_auth::{JwtConfig, Principal, issue_media_token};
use noflex_core::{AudioCodec, HdrTransfer, PixelFormat, VideoCodec, VideoProfile};
use noflex_graphql::{PlaybackRouteKind, PlaybackSessionService, PlaybackStart, PlaybackState};
use noflex_server::stream::{
    ServerPlaybackSessions, byte_route_principal, master_playlist, propagate_token_to_playlist,
    serve_job_file, serve_media_file, serve_subtitle,
};
use noflex_storage::{
    domain::{MediaFile, User},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use noflex_transcoder::{
    AudioPlan, ClientCapabilities, FfmpegJobManager, HlsSegmentContainer, JobStatus,
    PlaybackPlanKind, SubtitleBurnSource, TranscodeReason, TranscodeReasons, VideoPlan, VideoRange,
    probe::{AudioStreamProbe, ProbeReport, VideoStreamProbe},
};
use time::OffsetDateTime;

/// Seed library access plus a work and edition. Mirrors the seeding
/// in `tests/it/download.rs`.
async fn seed_store() -> (
    Arc<dyn Storage>,
    noflex_core::EditionId,
    noflex_core::UserId,
) {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let now = OffsetDateTime::now_utc();

    let library = noflex_storage::domain::Library {
        id: noflex_core::LibraryId::new(),
        name: "Films".to_string(),
        kind: noflex_storage::domain::LibraryKind::Movies,
        paths: vec![std::env::temp_dir().to_string_lossy().into_owned()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let user = User {
        id: noflex_core::UserId::new(),
        username: "stream-user".to_string(),
        display_name: None,
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: now,
        avatar_updated_at: None,
        has_pin: false,
    };
    store.upsert_user(&user).await.expect("user");
    store
        .grant_library_access(user.id, library.id)
        .await
        .expect("library access");

    let work = noflex_storage::domain::Work {
        id: noflex_core::WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Alien".to_string(),
        sort_title: "alien".to_string(),
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

    let edition = noflex_storage::domain::Edition {
        id: noflex_core::EditionId::new(),
        work_id: work.id,
        name: "Default".to_string(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    store.upsert_edition(&edition).await.expect("edition");

    (Arc::new(store), edition.id, user.id)
}

/// A probe that exactly matches `ClientCapabilities::web()` —
/// 1080p H.264 High@4.0 8-bit SDR, AAC LC stereo, mp4 — so
/// `decide` comes back Direct.
fn web_matching_probe() -> ProbeReport {
    ProbeReport {
        container: Some("mp4".to_string()),
        video: Some(VideoStreamProbe {
            codec: VideoCodec::H264,
            profile: Some(VideoProfile::H264High),
            level: Some(40.0),
            width: 1920,
            height: 1080,
            bit_depth: Some(8),
            pixel_format: Some(PixelFormat::Yuv420p),
            frame_rate: Some(24.0),
            color_space: Some(noflex_core::ColorPrimaries::Bt709),
            transfer: Some(HdrTransfer::Sdr),
            matrix: Some(noflex_core::MatrixCoefficients::Bt709),
            dovi_profile: None,
            side_data: Default::default(),
        }),
        audio: vec![AudioStreamProbe {
            index: 1,
            codec: "aac".to_string(),
            profile: Some("LC".to_string()),
            bit_depth: Some(16),
            sample_rate: Some(48_000),
            channels: 2,
            channel_layout: Some(noflex_core::ChannelLayout::Stereo),
            language: Some("eng".to_string()),
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

/// Insert a media file whose path is a real temp file holding
/// `payload`, with `probe` as the stored probe JSON.
async fn insert_media_file(
    store: &Arc<dyn Storage>,
    edition: noflex_core::EditionId,
    dir: &std::path::Path,
    payload: &[u8],
    probe: serde_json::Value,
) -> (noflex_core::MediaFileId, std::path::PathBuf) {
    insert_media_file_with_tracks(store, edition, dir, payload, probe, Vec::new()).await
}

/// Same as `insert_media_file`, with scanner-discovered subtitle
/// tracks recorded on the row.
async fn insert_media_file_with_tracks(
    store: &Arc<dyn Storage>,
    edition: noflex_core::EditionId,
    dir: &std::path::Path,
    payload: &[u8],
    probe: serde_json::Value,
    subtitle_tracks: Vec<noflex_storage::domain::SubtitleTrack>,
) -> (noflex_core::MediaFileId, std::path::PathBuf) {
    let path = dir.join("movie.mp4");
    std::fs::write(&path, payload).expect("write media file");
    let mf = MediaFile {
        id: noflex_core::MediaFileId::new(),
        edition_id: edition,
        path: path.to_string_lossy().to_string(),
        part_index: 0,
        size_bytes: payload.len() as u64,
        mtime: OffsetDateTime::now_utc(),
        container: Some("mp4".to_string()),
        probe: probe.into(),
        subtitle_tracks,
    };
    store.upsert_media_file(&mf).await.expect("upsert media");
    (mf.id, path)
}

/// A scanner-style external sidecar track pointing at `path`.
fn sidecar_track(path: &std::path::Path, codec: &str) -> noflex_storage::domain::SubtitleTrack {
    noflex_storage::domain::SubtitleTrack {
        language: Some("eng".to_string()),
        codec: codec.to_string(),
        default: true,
        forced: false,
        external: true,
        codec_profile: None,
        srt_compatible: true,
        image_count: None,
        title: None,
        path: Some(path.to_string_lossy().to_string()),
        stream_index: None,
        downloaded: None,
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("noflex-stream-{tag}-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A shell script standing in for ffmpeg: ignores its argv and
/// sleeps until killed, so the job stays Running until stopped.
fn make_mock_ffmpeg(dir: &std::path::Path) -> std::path::PathBuf {
    let mock = dir.join("ffmpeg");
    std::fs::write(&mock, "#!/bin/sh\nsleep 30\n").expect("write mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&mock).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&mock, perms).unwrap();
    }
    mock
}

fn test_transcoder_cfg() -> noflex_server::config::TranscoderConfig {
    noflex_server::config::TranscoderConfig {
        hwaccel: noflex_server::config::HwAccel::None,
        fallback: noflex_server::config::TranscodeFallback::Auto,
        optimize: Default::default(),
        tonemap_curve: Default::default(),
        ladder: vec![noflex_server::config::LadderEntry {
            max_pixels: 409_920,
            video_codec: VideoCodec::H264,
            video_profile: VideoProfile::H264High,
            audio_codec: AudioCodec::Aac,
            container: "mpegts".to_string(),
            bitrate_kbps: 2_000,
        }],
    }
}

const TEST_JWT_SECRET: &[u8] = b"stream-test-secret-that-is-32-bytes";

fn test_jwt() -> Arc<JwtConfig> {
    Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"))
}

/// A software-backend session service over `store`/`transcoder`,
/// minting media tokens off the test JWT secret.
fn session_service(
    store: &Arc<dyn Storage>,
    transcoder: &Arc<FfmpegJobManager>,
) -> ServerPlaybackSessions {
    ServerPlaybackSessions::new(
        store.clone(),
        transcoder.clone(),
        Arc::new(test_transcoder_cfg()),
        noflex_transcoder::HwBackend::Software,
        None,
        test_jwt(),
    )
}

/// A session URL must be `prefix` plus a `?token=` that verifies
/// against `target`.
fn assert_url_token(url: &str, prefix: &str, target: &str) {
    let (path, token) = url.split_once("?token=").expect("url carries a token");
    assert_eq!(path, prefix);
    assert_eq!(
        noflex_auth::verify_media_token(&test_jwt(), token).map(|claims| claims.target),
        Some(target.to_string())
    );
}

// ---- direct file serving -----------------------------------------------

#[tokio::test]
async fn serve_media_file_full_get_returns_200() {
    let dir = temp_dir("full");
    let (store, edition, _user) = seed_store().await;
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (id, _path) =
        insert_media_file(&store, edition, &dir, &payload, serde_json::json!({})).await;

    let resp = serve_media_file(&store, id, None).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&body[..], &payload[..]);
}

#[tokio::test]
async fn media_path_outside_its_library_root_is_rejected() {
    let temp = tempfile::tempdir().expect("temp");
    let library_root = temp.path().join("library");
    let outside = temp.path().join("library-private");
    std::fs::create_dir(&library_root).expect("library root");
    std::fs::create_dir(&outside).expect("outside");
    let (store, edition, _user) = seed_store().await;
    let mut library = store
        .list_libraries()
        .await
        .expect("libraries")
        .into_iter()
        .next()
        .expect("library");
    library.paths = vec![library_root.to_string_lossy().into_owned()];
    store
        .upsert_library(&library)
        .await
        .expect("update library");
    let (id, _) =
        insert_media_file(&store, edition, &outside, b"outside", serde_json::json!({})).await;

    let response = serve_media_file(&store, id, None).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serve_media_file_range_returns_206() {
    let dir = temp_dir("range");
    let (store, edition, _user) = seed_store().await;
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (id, _path) =
        insert_media_file(&store, edition, &dir, &payload, serde_json::json!({})).await;

    let resp = serve_media_file(&store, id, Some("bytes=0-99")).await;
    assert_eq!(resp.status(), axum::http::StatusCode::PARTIAL_CONTENT);
    let cr = resp
        .headers()
        .get(axum::http::header::CONTENT_RANGE)
        .expect("content-range")
        .to_str()
        .expect("utf8")
        .to_string();
    assert!(cr.starts_with("bytes 0-99/"), "got: {cr}");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&body[..], &payload[..100]);
}

#[tokio::test]
async fn serve_media_file_unsatisfiable_range_returns_416() {
    let dir = temp_dir("range416");
    let (store, edition, _user) = seed_store().await;
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (id, _path) =
        insert_media_file(&store, edition, &dir, &payload, serde_json::json!({})).await;

    let resp = serve_media_file(&store, id, Some("bytes=2000-")).await;
    assert_eq!(resp.status(), axum::http::StatusCode::RANGE_NOT_SATISFIABLE);
    let cr = resp
        .headers()
        .get(axum::http::header::CONTENT_RANGE)
        .expect("content-range")
        .to_str()
        .expect("utf8")
        .to_string();
    assert_eq!(cr, "bytes */1024");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert!(body.is_empty());
}

#[tokio::test]
async fn serve_media_file_malformed_range_falls_back_to_full_body() {
    let dir = temp_dir("rangemal");
    let (store, edition, _user) = seed_store().await;
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (id, _path) =
        insert_media_file(&store, edition, &dir, &payload, serde_json::json!({})).await;

    let resp = serve_media_file(&store, id, Some("bytes=0-10,20-30")).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&body[..], &payload[..]);
}

#[tokio::test]
async fn serve_media_file_unknown_id_returns_404() {
    let (store, _edition, _user) = seed_store().await;
    let resp = serve_media_file(&store, noflex_core::MediaFileId::new(), None).await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

// ---- byte-route authorization ------------------------------------------

#[tokio::test]
async fn session_bound_media_token_dies_with_its_session() {
    let (store, _edition, user) = seed_store().await;
    let now = OffsetDateTime::now_utc();
    let session = noflex_core::SessionId::new();
    store
        .create_session(&noflex_storage::domain::Session {
            id: session,
            user_id: user,
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
    let jwt = test_jwt();
    let target = format!("file:{}", uuid::Uuid::now_v7());
    let token = issue_media_token(&jwt, user, Some(session), &target, Duration::from_secs(60))
        .expect("mint");
    let anonymous = Principal::default();

    let (principal, _) =
        byte_route_principal(store.as_ref(), &anonymous, &jwt, Some(&token), &target)
            .await
            .expect("live session");
    assert_eq!(principal.session_id, Some(session));

    store.revoke_session(session).await.expect("revoke");
    let refused =
        byte_route_principal(store.as_ref(), &anonymous, &jwt, Some(&token), &target).await;
    assert_eq!(refused.err(), Some(axum::http::StatusCode::UNAUTHORIZED));
}

#[tokio::test]
async fn byte_route_principal_resolution() {
    let (store, _edition, user) = seed_store().await;
    let storage = store.as_ref();
    let jwt = test_jwt();
    let anonymous = Principal::default();
    let authed = Principal {
        user_id: noflex_core::UserId::from_uuid(uuid::Uuid::now_v7()),
        is_admin: false,
        session_id: None,
    };
    let target = format!("file:{}", uuid::Uuid::now_v7());
    let token =
        issue_media_token(&jwt, user, None, &target, Duration::from_secs(60)).expect("mint");
    let other_target = format!("file:{}", uuid::Uuid::now_v7());
    let other_token =
        issue_media_token(&jwt, user, None, &other_target, Duration::from_secs(60)).expect("mint");

    // An authenticated principal passes with no token at all.
    let (principal, propagated) = byte_route_principal(storage, &authed, &jwt, None, &target)
        .await
        .expect("authed");
    assert_eq!(principal.user_id, authed.user_id);
    assert!(propagated.is_none());
    // ...and another user's token isn't propagated under its session.
    let (_, propagated) = byte_route_principal(storage, &authed, &jwt, Some(&token), &target)
        .await
        .expect("authed with foreign token");
    assert!(propagated.is_none());

    // A matching token stands in for its user's credential, and comes
    // back for playlist propagation.
    let (principal, verified) =
        byte_route_principal(storage, &anonymous, &jwt, Some(&token), &target)
            .await
            .expect("matching token");
    assert_eq!(principal.user_id, user);
    assert!(!principal.is_admin);
    assert_eq!(verified.as_deref(), Some(token.as_str()));

    // A token minted for a different target, a garbage token, or no
    // credentials don't authorize.
    for token in [Some(other_token.as_str()), Some("garbage"), None] {
        assert_eq!(
            byte_route_principal(storage, &anonymous, &jwt, token, &target)
                .await
                .unwrap_err(),
            axum::http::StatusCode::UNAUTHORIZED
        );
    }
}

// ---- session start -------------------------------------------------------

#[tokio::test]
async fn session_start_direct_when_probe_matches_caps() {
    let dir = temp_dir("session-direct");
    let (store, edition, user) = seed_store().await;
    let probe = serde_json::to_value(web_matching_probe()).expect("probe json");
    let (id, _path) = insert_media_file(&store, edition, &dir, b"fake mp4", probe).await;

    let ffmpeg_dir = temp_dir("session-direct-ffmpeg");
    let mock = make_mock_ffmpeg(&ffmpeg_dir);
    let transcoder = Arc::new(FfmpegJobManager::new(mock, ffmpeg_dir.join("cache")));
    let service = session_service(&store, &transcoder);

    let session = service
        .start(
            user,
            None,
            PlaybackStart::new(id, ClientCapabilities::web()),
        )
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Direct);
    assert!(session.job_id.is_none());
    assert_url_token(
        &session.url,
        &format!("/stream/{id}/file"),
        &format!("file:{id}"),
    );
}

#[tokio::test]
async fn session_start_unknown_media_errors() {
    let (store, _edition, _user) = seed_store().await;
    let ffmpeg_dir = temp_dir("session-404-ffmpeg");
    let mock = make_mock_ffmpeg(&ffmpeg_dir);
    let transcoder = Arc::new(FfmpegJobManager::new(mock, ffmpeg_dir.join("cache")));
    let service = session_service(&store, &transcoder);

    let err = service
        .start(
            noflex_core::UserId::new(),
            None,
            PlaybackStart::new(noflex_core::MediaFileId::new(), ClientCapabilities::web()),
        )
        .await
        .expect_err("unknown media must fail");
    assert!(
        err.to_string().contains("media file not found"),
        "got: {err}"
    );
}

#[tokio::test]
async fn session_start_transcode_spawns_hls_job() {
    let dir = temp_dir("session-tc");
    let (store, edition, user) = seed_store().await;
    let probe = serde_json::to_value(web_matching_probe()).expect("probe json");
    let (id, _path) = insert_media_file(&store, edition, &dir, b"fake mp4", probe).await;

    let ffmpeg_dir = temp_dir("session-tc-ffmpeg");
    let mock = make_mock_ffmpeg(&ffmpeg_dir);
    let cache_root = ffmpeg_dir.join("cache");
    let transcoder = Arc::new(FfmpegJobManager::new(mock, cache_root.clone()));
    let service = session_service(&store, &transcoder);

    // 480p ceiling on a 1080p probe forces a transcode.
    let mut caps = ClientCapabilities::web();
    caps.max_height = Some(480);

    let session = service
        .start(
            user,
            None,
            PlaybackStart {
                device: Some("Firefox · Linux".into()),
                ..PlaybackStart::new(id, caps)
            },
        )
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Transcode);
    let job_id = session.job_id.expect("job_id").to_string();
    assert_url_token(
        &session.url,
        &format!("/stream/job/{job_id}/master.m3u8"),
        &format!("job:{job_id}"),
    );

    // Master playlist renders from the job's stored ladder; a
    // verified token is propagated onto the variant URI so the
    // sub-playlist request stays authenticated.
    let resp = master_playlist(transcoder.registry(), &job_id, None).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("index.m3u8"), "master: {text}");
    assert!(!text.contains("token="), "master: {text}");

    let resp = master_playlist(transcoder.registry(), &job_id, Some("tok")).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("index.m3u8?token=tok"), "master: {text}");

    // Segments don't exist yet mid-transcode: 404, not an error.
    let resp = serve_job_file(&transcoder, &cache_root, &job_id, "segment_00000.ts", None).await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);

    // Write one manually, as ffmpeg would.
    let job = transcoder
        .registry()
        .get(&job_id)
        .await
        .expect("job registered");
    std::fs::write(job.output_dir.join("segment_00000.ts"), b"ts-payload").expect("segment");
    let resp = serve_job_file(&transcoder, &cache_root, &job_id, "segment_00000.ts", None).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("video/mp2t")
    );

    // A sub-playlist served with a verified token rewrites its
    // segment URIs to carry it.
    std::fs::write(
        job.output_dir.join("index.m3u8"),
        "#EXTM3U\n#EXTINF:4.0,\nsegment_00000.ts\n",
    )
    .expect("playlist");
    let resp = serve_job_file(&transcoder, &cache_root, &job_id, "index.m3u8", Some("tok")).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("segment_00000.ts?token=tok"),
        "playlist: {text}"
    );
    assert!(!text.contains("#EXTINF:4.0,?token="), "playlist: {text}");

    // Path traversal / wrong extensions are rejected.
    for bad in ["../secret.ts", "a/b.m3u8", "index.m3u8.exe", "..ts"] {
        let resp = serve_job_file(&transcoder, &cache_root, &job_id, bad, None).await;
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::BAD_REQUEST,
            "filename: {bad}"
        );
    }

    // Stop flips the running job to Cancelled.
    service.stop(user, &job_id).await.expect("stop");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let job = transcoder
            .registry()
            .get(&job_id)
            .await
            .expect("job registered");
        if job.status != JobStatus::Running {
            assert_eq!(job.status, JobStatus::Cancelled);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "job never stopped");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

// ---- unknown job ids ------------------------------------------------------

#[tokio::test]
async fn unknown_job_errors_on_all_job_surfaces() {
    let (store, _edition, _user) = seed_store().await;
    let ffmpeg_dir = temp_dir("job-404-ffmpeg");
    let mock = make_mock_ffmpeg(&ffmpeg_dir);
    let transcoder = Arc::new(FfmpegJobManager::new(mock, ffmpeg_dir.join("cache")));
    let service = session_service(&store, &transcoder);

    let resp = master_playlist(transcoder.registry(), "nope", None).await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    let resp = serve_job_file(
        &transcoder,
        &ffmpeg_dir.join("cache"),
        "nope",
        "index.m3u8",
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    let err = service
        .stop(noflex_core::UserId::new(), "nope")
        .await
        .expect_err("unknown job must fail");
    assert!(err.to_string().contains("session not found"), "got: {err}");
}

// ---- subtitle sidecar serving --------------------------------------------

#[tokio::test]
async fn serve_subtitle_returns_bytes_and_content_type() {
    let dir = temp_dir("subs");
    let (store, edition, _user) = seed_store().await;
    let srt_path = dir.join("movie.en.srt");
    let srt_payload = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n";
    std::fs::write(&srt_path, srt_payload).expect("write srt");
    let vtt_path = dir.join("movie.en.vtt");
    let vtt_payload = b"WEBVTT\n\n00:01.000 --> 00:02.000\nHello\n";
    std::fs::write(&vtt_path, vtt_payload).expect("write vtt");
    let tracks = vec![
        sidecar_track(&srt_path, "srt"),
        sidecar_track(&vtt_path, "vtt"),
    ];
    let (id, _path) = insert_media_file_with_tracks(
        &store,
        edition,
        &dir,
        b"fake mp4",
        serde_json::json!({}),
        tracks,
    )
    .await;

    // Both are delivered as WebVTT so `<track>` can render them.
    for (index, expected) in [
        (
            0usize,
            "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\nHello\n",
        ),
        (1, std::str::from_utf8(vtt_payload).expect("utf8")),
    ] {
        let resp = serve_subtitle(&store, std::path::Path::new("ffmpeg"), &dir, id, index).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/vtt; charset=utf-8")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(std::str::from_utf8(&body).expect("utf8"), expected);
    }
}

#[tokio::test]
async fn serve_subtitle_missing_track_returns_404() {
    let dir = temp_dir("subs-404");
    let (store, edition, _user) = seed_store().await;
    // A track with neither a sidecar path nor a stream index is unservable.
    let mut muxed = sidecar_track(&dir.join("unused.srt"), "srt");
    muxed.external = false;
    muxed.path = None;
    let (id, _path) = insert_media_file_with_tracks(
        &store,
        edition,
        &dir,
        b"fake mp4",
        serde_json::json!({}),
        vec![muxed],
    )
    .await;

    let ffmpeg = std::path::Path::new("ffmpeg");
    let resp = serve_subtitle(&store, ffmpeg, &dir, id, 0).await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    let resp = serve_subtitle(&store, ffmpeg, &dir, id, 7).await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn embedded_text_subtitles_are_extracted_once_and_cached() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("subs-embedded");
    let (store, edition, _user) = seed_store().await;
    let ffmpeg = dir.join("ffmpeg");
    let write_ffmpeg = |script: &str| {
        std::fs::write(&ffmpeg, script).expect("write ffmpeg");
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    };
    write_ffmpeg(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/argv\"\nprintf 'WEBVTT\\n\\n00:01.000 --> 00:02.000\\nHi\\n'\n",
    );
    let mut text = sidecar_track(&dir.join("unused.srt"), "sub_rip");
    (text.external, text.path, text.stream_index) = (false, None, Some(1));
    let mut bitmap = text.clone();
    (bitmap.codec, bitmap.stream_index) = ("pgs_image".into(), Some(0));
    let (id, media_path) = insert_media_file_with_tracks(
        &store,
        edition,
        &dir,
        b"fake mkv",
        serde_json::json!({}),
        vec![bitmap, text],
    )
    .await;
    let cache = dir.join("cache");

    let body = |resp: axum::response::Response| async move {
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    };
    let first = body(serve_subtitle(&store, &ffmpeg, &cache, id, 1).await).await;
    assert!(first.starts_with("WEBVTT"), "{first}");
    let argv = std::fs::read_to_string(dir.join("argv")).expect("argv");
    assert!(argv.contains("0:s:1"), "{argv}");
    assert!(
        argv.contains(&format!("file:{}", media_path.display())),
        "{argv}"
    );

    // A second request is served from the cache, not a new ffmpeg run.
    write_ffmpeg("#!/bin/sh\nexit 1\n");
    assert_eq!(
        body(serve_subtitle(&store, &ffmpeg, &cache, id, 1).await).await,
        first
    );

    // Bitmap tracks can only be burned in.
    let resp = serve_subtitle(&store, &ffmpeg, &cache, id, 0).await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

// ---- session burn-in ------------------------------------------------------

/// Stop the sleeping mock-ffmpeg job and wait for the registry
/// to observe the kill, so no `sleep 30` child outlives the test.
async fn stop_and_wait(
    service: &ServerPlaybackSessions,
    transcoder: &FfmpegJobManager,
    user: noflex_core::UserId,
    job_id: &str,
) {
    service.stop(user, job_id).await.expect("stop");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let job = transcoder
            .registry()
            .get(job_id)
            .await
            .expect("job registered");
        if job.status != JobStatus::Running {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "job never stopped");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn session_burn_subtitle_forces_transcode() {
    let dir = temp_dir("burn");
    let (store, edition, user) = seed_store().await;
    let srt_path = dir.join("movie.en.srt");
    std::fs::write(&srt_path, b"1\n00:00:01,000 --> 00:00:02,000\nHi\n").expect("write srt");
    let probe = serde_json::to_value(web_matching_probe()).expect("probe json");
    let (id, _path) = insert_media_file_with_tracks(
        &store,
        edition,
        &dir,
        b"fake mp4",
        probe,
        vec![sidecar_track(&srt_path, "srt")],
    )
    .await;

    let ffmpeg_dir = temp_dir("burn-ffmpeg");
    let mock = make_mock_ffmpeg(&ffmpeg_dir);
    let transcoder = Arc::new(FfmpegJobManager::new(mock, ffmpeg_dir.join("cache")));
    let service = session_service(&store, &transcoder);

    // The probe exactly matches web caps — without the burn
    // request this session would direct-play.
    let session = service
        .start(
            user,
            None,
            PlaybackStart {
                burn_subtitle: Some(0),
                ..PlaybackStart::new(id, ClientCapabilities::web())
            },
        )
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Transcode);
    let job_id = session.job_id.expect("job_id").to_string();

    let job = transcoder
        .registry()
        .get(&job_id)
        .await
        .expect("job registered");
    assert_eq!(
        job.request.subtitle_burn,
        Some(SubtitleBurnSource::External(srt_path.clone()))
    );
    assert!(matches!(
        job.request.plan.video,
        VideoPlan::Encode {
            codec: VideoCodec::H264,
            // Fitted to the ladder's one 480p rung.
            height: 480,
            tonemap: false,
            profile: VideoProfile::H264High,
            ..
        }
    ));
    assert_eq!(
        job.request.plan.audio,
        AudioPlan::Copy {
            codec: AudioCodec::Aac
        }
    );
    assert_eq!(job.request.reasons, TranscodeReasons::SubtitleBurnIn);
    assert_eq!(session.decision.plan, PlaybackPlanKind::TranscodeVideo);
    assert!(
        session.subtitles.is_empty(),
        "burned track isn't also delivered"
    );

    stop_and_wait(&service, &transcoder, user, &job_id).await;
}

#[tokio::test]
async fn transcode_starts_at_the_requested_position() {
    let dir = temp_dir("start-at");
    let (store, edition, user) = seed_store().await;
    let probe = serde_json::to_value(web_matching_probe()).expect("probe json");
    let (id, _path) =
        insert_media_file_with_tracks(&store, edition, &dir, b"fake mp4", probe, vec![]).await;
    let ffmpeg_dir = temp_dir("start-at-ffmpeg");
    let mock = make_mock_ffmpeg(&ffmpeg_dir);
    let transcoder = Arc::new(FfmpegJobManager::new(mock, ffmpeg_dir.join("cache")));
    let service = session_service(&store, &transcoder);

    // Direct play ignores the position: the client seeks.
    let direct = service
        .start(
            user,
            None,
            PlaybackStart {
                start_position_ms: Some(90_000),
                ..PlaybackStart::new(id, ClientCapabilities::web())
            },
        )
        .await
        .expect("direct");
    assert_eq!(direct.route, PlaybackRouteKind::Direct);
    assert_eq!(direct.start_position_ms, 0);

    // A client that can't direct-play gets a transcode starting there.
    let mut caps = ClientCapabilities::web();
    caps.direct_play.clear();
    let session = service
        .start(
            user,
            None,
            PlaybackStart {
                start_position_ms: Some(90_000),
                ..PlaybackStart::new(id, caps)
            },
        )
        .await
        .expect("transcode");
    assert_eq!(session.route, PlaybackRouteKind::Transcode);
    assert_eq!(session.start_position_ms, 90_000);
    let job_id = session.job_id.expect("job_id").to_string();
    let job = transcoder.registry().get(&job_id).await.expect("job");
    assert_eq!(job.request.start_secs, Some(90.0));
    stop_and_wait(&service, &transcoder, user, &job_id).await;
}

#[tokio::test]
async fn session_burn_subtitle_bad_track_errors() {
    let dir = temp_dir("burn-bad");
    let (store, edition, user) = seed_store().await;
    // Neither a sidecar nor a known stream: nothing to burn in.
    let mut muxed = sidecar_track(&dir.join("unused.srt"), "srt");
    muxed.external = false;
    muxed.path = None;
    let probe = serde_json::to_value(web_matching_probe()).expect("probe json");
    let (id, _path) =
        insert_media_file_with_tracks(&store, edition, &dir, b"fake mp4", probe, vec![muxed]).await;

    let ffmpeg_dir = temp_dir("burn-bad-ffmpeg");
    let mock = make_mock_ffmpeg(&ffmpeg_dir);
    let transcoder = Arc::new(FfmpegJobManager::new(mock, ffmpeg_dir.join("cache")));
    let service = session_service(&store, &transcoder);

    let err = service
        .start(
            user,
            None,
            PlaybackStart {
                burn_subtitle: Some(9),
                ..PlaybackStart::new(id, ClientCapabilities::web())
            },
        )
        .await
        .expect_err("out-of-range track must fail");
    assert!(
        err.to_string().contains("subtitle track not found"),
        "got: {err}"
    );

    let err = service
        .start(
            user,
            None,
            PlaybackStart {
                burn_subtitle: Some(0),
                ..PlaybackStart::new(id, ClientCapabilities::web())
            },
        )
        .await
        .expect_err("track without a source must fail");
    assert!(
        err.to_string().contains("subtitle track not found"),
        "got: {err}"
    );
}

// ---- playlist token rewrite ----------------------------------------------

#[test]
fn playlist_rewrite_appends_token_to_uri_lines_only() {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:4.0,\nsegment_00000.ts\nsub.m3u8?cdn=1\n#EXT-X-ENDLIST\n";
    let out = propagate_token_to_playlist(playlist, "tok");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "#EXTM3U");
    assert_eq!(lines[1], "#EXT-X-VERSION:3");
    assert_eq!(lines[2], "#EXTINF:4.0,");
    assert_eq!(lines[3], "segment_00000.ts?token=tok");
    // A URI that already has a query string takes `&token=`.
    assert_eq!(lines[4], "sub.m3u8?cdn=1&token=tok");
    assert_eq!(lines[5], "#EXT-X-ENDLIST");
}

// ---- out-of-band subtitles + active-stream registry ----------------------

#[tokio::test]
async fn direct_session_delivers_text_subtitles_and_tracks_activity() {
    let dir = temp_dir("session-subs");
    let (store, edition, user) = seed_store().await;
    let srt_path = dir.join("movie.en.srt");
    std::fs::write(&srt_path, b"1\n00:00:01,000 --> 00:00:02,000\nHi\n").expect("write srt");
    let probe = serde_json::to_value(web_matching_probe()).expect("probe json");
    let (id, _path) = insert_media_file_with_tracks(
        &store,
        edition,
        &dir,
        b"fake mp4",
        probe,
        vec![sidecar_track(&srt_path, "sub_rip")],
    )
    .await;
    let ffmpeg_dir = temp_dir("session-subs-ffmpeg");
    let transcoder = Arc::new(FfmpegJobManager::new(
        make_mock_ffmpeg(&ffmpeg_dir),
        ffmpeg_dir.join("cache"),
    ));
    let service = session_service(&store, &transcoder);

    // A text sidecar never forces a transcode.
    let session = service
        .start(
            user,
            None,
            PlaybackStart {
                device: Some("Safari".into()),
                ..PlaybackStart::new(id, ClientCapabilities::web())
            },
        )
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Direct);
    assert_eq!(session.decision.plan, PlaybackPlanKind::DirectPlay);
    assert!(session.decision.reasons.is_empty());
    assert_eq!(session.subtitles.len(), 1);
    assert_eq!(session.subtitles[0].index, 0);
    assert_url_token(
        &session.subtitles[0].url,
        &format!("/stream/{id}/subtitles/0"),
        &format!("file:{id}"),
    );

    // Without a declared text format there is nothing to deliver.
    let mut caps = ClientCapabilities::web();
    caps.subtitle_codecs.clear();
    let other = service
        .start(user, None, PlaybackStart::new(id, caps))
        .await
        .expect("start");
    assert!(other.subtitles.is_empty());

    let streams = service.active_streams().await.expect("streams");
    assert_eq!(streams.len(), 2);
    let first = streams
        .iter()
        .find(|s| s.id == session.session_id.to_string())
        .expect("first registered");
    assert!(first.transcode.is_none());
    assert_eq!(first.device.as_deref(), Some("Safari"));
    assert_eq!(first.media_file_id, id);

    service
        .heartbeat(
            user,
            noflex_core::PlayTarget::Edition(edition),
            4_200,
            PlaybackState::Playing,
        )
        .await;
    let streams = service.active_streams().await.expect("streams");
    assert!(streams.iter().all(|s| s.position_ms == Some(4_200)));

    // Explicit stop ends one; a STOPPED report ends the rest.
    service
        .stop(user, &session.session_id)
        .await
        .expect("stop direct");
    assert_eq!(service.active_streams().await.expect("streams").len(), 1);
    let err = service
        .stop(noflex_core::UserId::new(), &other.session_id)
        .await
        .expect_err("foreign stop");
    assert!(err.to_string().contains("session not found"));
    service
        .heartbeat(
            user,
            noflex_core::PlayTarget::Edition(edition),
            5_000,
            PlaybackState::Stopped,
        )
        .await;
    assert!(service.active_streams().await.expect("streams").is_empty());
}

#[tokio::test]
async fn pausing_and_resuming_are_told_once_per_change() {
    let dir = temp_dir("session-pause");
    let (store, edition, user) = seed_store().await;
    let probe = serde_json::to_value(web_matching_probe()).expect("probe json");
    let (id, _path) =
        insert_media_file_with_tracks(&store, edition, &dir, b"fake mp4", probe, Vec::new()).await;
    let ffmpeg_dir = temp_dir("session-pause-ffmpeg");
    let transcoder = Arc::new(FfmpegJobManager::new(
        make_mock_ffmpeg(&ffmpeg_dir),
        ffmpeg_dir.join("cache"),
    ));
    let bus = noflex_core::DomainEventBus::new(64);
    let mut events = bus.subscribe();
    let service = session_service(&store, &transcoder).with_events(bus);
    service
        .start(
            user,
            None,
            PlaybackStart::new(id, ClientCapabilities::web()),
        )
        .await
        .expect("start");

    let target = noflex_core::PlayTarget::Edition(edition);
    for (position_ms, state) in [
        (1_000, PlaybackState::Playing),
        (2_000, PlaybackState::Playing),
        (3_000, PlaybackState::Paused),
        (3_000, PlaybackState::Paused),
        (3_000, PlaybackState::Paused),
        (3_500, PlaybackState::Playing),
        (4_000, PlaybackState::Playing),
    ] {
        service.heartbeat(user, target, position_ms, state).await;
    }

    let mut told = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            noflex_core::DomainEvent::PlaybackStarted { media_file, .. } => {
                told.push(("started", media_file, None));
            }
            noflex_core::DomainEvent::PlaybackPaused {
                media_file,
                position_ms,
                ..
            } => told.push(("paused", media_file, Some(position_ms))),
            noflex_core::DomainEvent::PlaybackResumed {
                media_file,
                position_ms,
                ..
            } => told.push(("resumed", media_file, Some(position_ms))),
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert_eq!(
        told,
        vec![
            ("started", id, None),
            ("paused", id, Some(3_000)),
            ("resumed", id, Some(3_500)),
        ],
        "heartbeats in the same state stay quiet"
    );
}

fn hevc_hdr10_truehd_mkv_probe() -> ProbeReport {
    let mut probe = web_matching_probe();
    probe.container = Some("matroska".to_string());
    let video = probe.video.as_mut().expect("video");
    video.codec = VideoCodec::H265;
    video.profile = Some(VideoProfile::HevcMain10);
    video.level = Some(153.0);
    video.width = 3840;
    video.height = 2160;
    video.bit_depth = Some(10);
    video.pixel_format = Some(PixelFormat::Yuv420p10le);
    video.transfer = Some(HdrTransfer::Pq);
    probe.audio[0].codec = "truehd".to_string();
    probe.audio[0].channels = 8;
    probe.format_bit_rate = Some(40_000_000);
    probe
}

#[tokio::test]
async fn hevc_hdr_mkv_copies_video_into_fmp4_for_hevc_client() {
    let dir = temp_dir("session-hevc");
    let (store, edition, user) = seed_store().await;
    let probe = serde_json::to_value(hevc_hdr10_truehd_mkv_probe()).expect("probe json");
    let (id, _path) = insert_media_file(&store, edition, &dir, b"fake mkv", probe).await;
    let ffmpeg_dir = temp_dir("session-hevc-ffmpeg");
    let transcoder = Arc::new(FfmpegJobManager::new(
        make_mock_ffmpeg(&ffmpeg_dir),
        ffmpeg_dir.join("cache"),
    ));
    let service = session_service(&store, &transcoder);

    let mut caps = ClientCapabilities::web();
    caps.hls[0].video_codecs.push(VideoCodec::H265);
    caps.video_codecs
        .push(noflex_transcoder::VideoCodecSupport {
            codec: VideoCodec::H265,
            profiles: vec![VideoProfile::HevcMain10, VideoProfile::HevcMain],
            max_level: Some(183.0),
            max_bit_depth: Some(10),
            max_width: Some(7680),
            max_height: Some(4320),
            max_pixels: None,
            video_ranges: vec![VideoRange::Hdr10],
        });
    let session = service
        .start(user, None, PlaybackStart::new(id, caps))
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Transcode);
    assert_eq!(session.decision.plan, PlaybackPlanKind::TranscodeAudio);
    let codes: Vec<_> = session.decision.reasons.iter().map(|r| r.code).collect();
    assert_eq!(codes, vec![TranscodeReason::AudioCodecNotSupported]);
    assert_eq!(session.decision.encoder.as_deref(), Some("copy"));
    assert_eq!(session.decision.target.video_range, Some(VideoRange::Hdr10));

    let job_id = session.job_id.expect("job").to_string();
    let job = transcoder.registry().get(&job_id).await.expect("job");
    assert_eq!(
        job.request.plan.segment_container,
        HlsSegmentContainer::Fmp4
    );
    assert!(matches!(
        job.request.plan.video,
        VideoPlan::Copy {
            codec: VideoCodec::H265,
            range: VideoRange::Hdr10
        }
    ));
    assert_eq!(
        job.request.plan.audio,
        AudioPlan::Encode {
            codec: AudioCodec::Aac,
            channels: 6
        }
    );

    let resp = master_playlist(transcoder.registry(), &job_id, None).await;
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("CODECS=\"hvc1.2.4.L153.B0,mp4a.40.2\""),
        "master: {text}"
    );
    assert!(text.contains("VIDEO-RANGE=PQ"), "master: {text}");
    assert!(text.contains("RESOLUTION=3840x2160"), "master: {text}");

    // fMP4 artifacts are servable.
    std::fs::write(job.output_dir.join("init.mp4"), b"init").expect("init");
    let resp = serve_job_file(
        &transcoder,
        &ffmpeg_dir.join("cache"),
        &job_id,
        "init.mp4",
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let streams = service.active_streams().await.expect("streams");
    let progress = streams[0].transcode.as_ref().expect("transcode progress");
    assert_eq!(progress.status, "running");
    assert_eq!(progress.encoder, "copy");
    assert!(!progress.hardware);

    stop_and_wait(&service, &transcoder, user, &job_id).await;
    assert!(service.active_streams().await.expect("streams").is_empty());
}

#[tokio::test]
async fn live_start_refuses_non_http_inputs() {
    let (store, _edition, user) = seed_store().await;
    let ffmpeg_dir = temp_dir("live-scheme-ffmpeg");
    let transcoder = Arc::new(FfmpegJobManager::new(
        make_mock_ffmpeg(&ffmpeg_dir),
        ffmpeg_dir.join("cache"),
    ));
    let service = session_service(&store, &transcoder);
    for bad in ["file:///etc/passwd", "concat:/a|/b", "/srv/tv.ts"] {
        let err = service
            .start_live(user, noflex_core::ChannelId::new(), bad)
            .await
            .expect_err(bad);
        assert!(err.to_string().contains("http or https"), "{bad}: {err}");
    }
    assert!(service.active_streams().await.expect("streams").is_empty());
}

// ---- on-demand sessions ---------------------------------------------------

/// The source time of the first frame `segment` holds, read back through
/// ffprobe with its variant's init segment in front.
fn segment_start(init: &[u8], segment: &[u8], dir: &std::path::Path) -> f64 {
    let joined = dir.join(format!("probe-{}.mp4", uuid::Uuid::now_v7()));
    std::fs::write(&joined, [init, segment].concat()).expect("joined segment");
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=start_time", "-of", "csv=p=0"])
        .arg(&joined)
        .output()
        .expect("ffprobe");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("start time")
}

async fn body_of(resp: axum::response::Response) -> (axum::http::StatusCode, Vec<u8>) {
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, body.to_vec())
}

#[tokio::test]
async fn adaptive_session_encodes_whatever_segment_and_variant_is_asked_for() {
    let dir = temp_dir("on-demand");
    let source = dir.join("source.mkv");
    let made = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=1280x720:r=24:d=30"])
        .args(["-f", "lavfi", "-i", "sine=f=440:d=30"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"])
        .arg(&source)
        .status()
        .is_ok_and(|status| status.success());
    if !made {
        eprintln!("skipping: no ffmpeg to make a source with");
        return;
    }
    let probe = noflex_scanner::ffprobe::probe_media_file(&source, None)
        .await
        .expect("probe");
    assert!(probe.duration_ms.is_some());
    let (store, edition, user) = seed_store().await;
    let payload = std::fs::read(&source).expect("source");
    let (id, _path) = insert_media_file(
        &store,
        edition,
        &dir,
        &payload,
        serde_json::to_value(&probe).expect("probe json"),
    )
    .await;
    let cache_root = dir.join("cache");
    let transcoder = Arc::new(FfmpegJobManager::new("ffmpeg", cache_root.clone()));
    let service = session_service(&store, &transcoder);

    // Under its size, so it has to be encoded.
    let mut caps = ClientCapabilities::web();
    caps.max_height = Some(480);
    let session = service
        .start(
            user,
            None,
            PlaybackStart {
                start_position_ms: Some(9_000),
                adaptive: true,
                ..PlaybackStart::new(id, caps)
            },
        )
        .await
        .expect("start");
    assert_eq!(session.route, PlaybackRouteKind::Transcode);
    assert!(session.adaptive);
    // The playlist covers the whole title, so the player seeks to the start itself.
    assert_eq!(session.start_position_ms, 0);
    let labels: Vec<&str> = session.qualities.iter().map(|q| q.label.as_str()).collect();
    assert_eq!(labels, ["480p", "360p"]);
    let job_id = session.job_id.expect("job_id").to_string();

    let (_, master) = body_of(master_playlist(transcoder.registry(), &job_id, None).await).await;
    let master = String::from_utf8_lossy(&master).to_string();
    assert!(
        master.find("v0.m3u8") < master.find("v1.m3u8"),
        "best first: {master}"
    );
    let fetch = |name: &'static str| serve_job_file(&transcoder, &cache_root, &job_id, name, None);
    let (status, playlist) = body_of(fetch("v1.m3u8").await).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let playlist = String::from_utf8_lossy(&playlist).to_string();
    assert!(
        playlist.contains("v1_00007.m4s\n#EXT-X-ENDLIST"),
        "{playlist}"
    );

    // The segment it resumes in, already encoding since the session started.
    let (status, segment) = body_of(fetch("v0_00002.m4s").await).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let (status, init) = body_of(fetch("v0_init.mp4").await).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let start = segment_start(&init, &segment, &dir) - noflex_transcoder::PTS_PAD_SECS;
    assert!((start - 8.0).abs() < 0.1, "segment 2 starts at {start}");

    // A seek far ahead, and a switch to the other variant, each start a run there.
    let (status, segment) = body_of(fetch("v1_00006.m4s").await).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let (_, init) = body_of(fetch("v1_init.mp4").await).await;
    let start = segment_start(&init, &segment, &dir) - noflex_transcoder::PTS_PAD_SECS;
    assert!((start - 24.0).abs() < 0.1, "segment 6 starts at {start}");

    let (status, _) = body_of(fetch("v0_99999.m4s").await).await;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    let (status, _) = body_of(fetch("v0_run.m3u8").await).await;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);

    stop_and_wait(&service, &transcoder, user, &job_id).await;
    let session_dir = cache_root.join(&job_id);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while session_dir.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "segments outlived the session"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
