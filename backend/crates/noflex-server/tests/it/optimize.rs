// SPDX-License-Identifier: AGPL-3.0-only
//
// The smart-transcoding pipeline against real ffmpeg: from a viewer's device
// and watch history to a playable optimized version.

use std::{path::Path, sync::Arc};

use axum::{body::to_bytes, response::Response};
use noflex_core::{
    AudioCodec, DeviceId, EditionId, LibraryId, MediaFileId, MetadataStatus, PlayTarget, SessionId,
    UserId, VideoCodec, VideoProfile, WorkId, WorkKind,
};
use noflex_graphql::{ActiveStreamRecord, PlaybackSession, PlaybackSessionService, PlaybackStart};
use noflex_server::{
    config::OptimizeConfig, file_serve::RootCapability, optimize_worker::OptimizeWorker,
    optimized_serve,
};
use noflex_storage::{
    Storage,
    domain::{
        Device, Edition, Library, MediaFile, OptimizedVersionStatus, SmartTranscodingSettings,
        User, Work,
    },
};
use noflex_storage_turso::TursoStore;
use noflex_transcoder::{
    AudioCodecSupport, ClientCapabilities, HlsProfile, HlsSegmentContainer, HwBackend,
    VideoCodecSupport, optimize::version_plays, probe::ProbeReport,
};
use time::OffsetDateTime;

struct Idle;

#[async_trait::async_trait]
impl PlaybackSessionService for Idle {
    async fn start(
        &self,
        _user: UserId,
        _login: Option<SessionId>,
        _request: PlaybackStart,
    ) -> anyhow::Result<PlaybackSession> {
        anyhow::bail!("not used")
    }
    async fn stop(&self, _user: UserId, _session_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn active_streams(&self) -> anyhow::Result<Vec<ActiveStreamRecord>> {
        Ok(Vec::new())
    }
}

/// A client that decodes H.264 and AAC over fragmented-MP4 HLS, and direct-plays
/// nothing, so any source needs a version.
fn h264_client() -> ClientCapabilities {
    ClientCapabilities {
        direct_play: vec![],
        hls: vec![HlsProfile {
            segment_container: HlsSegmentContainer::Fmp4,
            video_codecs: vec![VideoCodec::H264],
            audio_codecs: vec![AudioCodec::Aac],
        }],
        video_codecs: vec![VideoCodecSupport {
            codec: VideoCodec::H264,
            profiles: vec![VideoProfile::H264High, VideoProfile::H264Main],
            max_level: None,
            max_bit_depth: Some(8),
            max_width: None,
            max_height: Some(1080),
            max_pixels: None,
            video_ranges: vec![],
        }],
        audio_codecs: vec![AudioCodecSupport {
            codec: AudioCodec::Aac,
            max_channels: Some(6),
        }],
        subtitle_codecs: vec![],
        max_height: Some(1080),
        max_bitrate_bps: None,
    }
}

/// A 10-second source no browser plays directly, with two audio tracks.
fn make_source(path: &Path) -> bool {
    std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=320x180:r=24:d=10"])
        .args(["-f", "lavfi", "-i", "sine=f=440:d=10"])
        .args(["-f", "lavfi", "-i", "sine=f=880:d=10"])
        .args(["-map", "0", "-map", "1", "-map", "2"])
        .args([
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
        ])
        .args(["-b:v", "200k", "-c:a", "libmp3lame"])
        .args([
            "-metadata:s:a:0",
            "language=eng",
            "-metadata:s:a:1",
            "language=jpn",
        ])
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

struct Fixture {
    store: Arc<TursoStore>,
    user: UserId,
    media: MediaFile,
    dir: std::path::PathBuf,
}

async fn fixture(source: &Path, dir: std::path::PathBuf) -> Fixture {
    let store = Arc::new(TursoStore::open_temp().await.expect("store"));
    store.migrate().await.expect("migrate");
    let now = OffsetDateTime::now_utc();
    let user = User {
        id: UserId::new(),
        name: "viewer".to_string(),
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: true,
        role: noflex_core::UserRole::Admin,
        created_at: now,
    };
    store.upsert_user(&user).await.expect("user");
    let library = Library {
        id: LibraryId::new(),
        name: "Films".to_string(),
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let work = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Source".to_string(),
        sort_title: "source".to_string(),
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
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&work).await.expect("work");
    let edition = Edition {
        id: EditionId::new(),
        work_id: work.id,
        name: "Default".to_string(),
        runtime_ms: Some(10_000),
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
    store.upsert_edition(&edition).await.expect("edition");
    let probe = noflex_scanner::ffprobe::probe_media_file(source, None)
        .await
        .expect("probe");
    let media = MediaFile {
        id: MediaFileId::new(),
        edition_id: edition.id,
        path: source.display().to_string(),
        part_index: 0,
        size_bytes: std::fs::metadata(source).expect("source").len(),
        mtime: now,
        container: Some("matroska".to_string()),
        probe: async_graphql::Json(serde_json::to_value(&probe).expect("probe json")),
        subtitle_tracks: vec![],
    };
    store.upsert_media_file(&media).await.expect("media");
    store
        .record_playback_progress(user.id, PlayTarget::Work(work.id), 1_000)
        .await
        .expect("progress");
    store
        .upsert_device(&Device {
            id: DeviceId::new(),
            user_id: user.id,
            name: "Phone".to_string(),
            capabilities: async_graphql::Json(
                serde_json::to_value(h264_client()).expect("capabilities"),
            ),
            smart_transcode: true,
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
        store,
        user: user.id,
        media,
        dir,
    }
}

fn worker(fixture: &Fixture, config: OptimizeConfig) -> OptimizeWorker {
    OptimizeWorker {
        storage: fixture.store.clone(),
        playback: Arc::new(Idle),
        plugins: None,
        config,
        ladder: vec![],
        ffmpeg_path: "ffmpeg".into(),
        backend: HwBackend::Software,
        vaapi_device: None,
        gpu_tonemap: false,
        tonemap_curve: Default::default(),
        dir: fixture.dir.join("optimized"),
    }
}

fn config() -> OptimizeConfig {
    OptimizeConfig {
        encode_video: vec![VideoCodec::H264],
        ..Default::default()
    }
}

async fn text(response: Response) -> String {
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn a_watched_title_becomes_a_playable_version_with_every_audio_track() {
    let dir = std::env::temp_dir().join(format!("noflex-optimize-it-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mkv");
    if !make_source(&source) {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let fixture = fixture(&source, dir.clone()).await;
    let worker = worker(&fixture, config());

    worker.plan().await.expect("plan");
    worker
        .plan()
        .await
        .expect("planning twice queues nothing new");
    let queued = fixture.store.list_optimized_versions().await.expect("list");
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(queued[0].status, OptimizedVersionStatus::Queued);

    assert!(worker.run_next().await.expect("run"));
    assert!(!worker.run_next().await.expect("nothing left"));

    let version = fixture
        .store
        .get_optimized_version(queued[0].id)
        .await
        .expect("get")
        .expect("version");
    assert_eq!(
        version.status,
        OptimizedVersionStatus::Ready,
        "{:?}",
        version.error
    );
    let path = version.path.clone().expect("path");
    assert_eq!(
        std::fs::metadata(&path).expect("file").len(),
        version.size_bytes.expect("size")
    );
    assert!(
        !Path::new(&format!("{}.raw.mp4", path.trim_end_matches(".mp4"))).exists(),
        "intermediate output is cleaned up"
    );

    let probe: ProbeReport =
        serde_json::from_value(version.probe.clone().expect("probe")).expect("probe");
    assert_eq!(probe.audio.len(), 2, "every audio track survives");
    assert!(version_plays(&probe, &h264_client()));

    // The manifests lead to each track, and each of those resolves.
    let store: Arc<dyn Storage> = fixture.store.clone();
    let master =
        text(optimized_serve::serve_master(&store, version.clone(), Some("tok")).await).await;
    assert!(master.contains("LANGUAGE=\"eng\""), "{master}");
    assert!(master.contains("LANGUAGE=\"jpn\""), "{master}");
    assert!(master.contains("video.m3u8?token=tok"), "{master}");
    let root = RootCapability::new(&worker.dir).await.expect("root");
    let video =
        text(optimized_serve::serve_part(&version, &root, "video.m3u8", Some("tok"), None).await)
            .await;
    assert!(video.contains("#EXT-X-BYTERANGE"), "{video}");
    assert!(video.contains("file?token=tok"), "{video}");
    let audio_track = master
        .split("URI=\"")
        .nth(1)
        .and_then(|rest| rest.split('?').next())
        .expect("audio playlist");
    let audio =
        text(optimized_serve::serve_part(&version, &root, audio_track, None, None).await).await;
    let init_name = audio
        .split("URI=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("init segment");
    let init = optimized_serve::serve_part(&version, &root, init_name, None, None).await;
    assert!(init.status().is_success());
    let ranged =
        optimized_serve::serve_part(&version, &root, "file", None, Some("bytes=0-99")).await;
    assert_eq!(ranged.status(), axum::http::StatusCode::PARTIAL_CONTENT);
    let unknown = optimized_serve::serve_part(&version, &root, "audio_99.m3u8", None, None).await;
    assert_eq!(unknown.status(), axum::http::StatusCode::NOT_FOUND);

    // Using it stamps it, which is what eviction looks at.
    let used = fixture
        .store
        .get_optimized_version(version.id)
        .await
        .expect("get")
        .expect("version");
    assert!(used.last_used_at.is_some());

    // A changed source makes the version stale; a re-plan replaces it.
    let mut changed = fixture.media.clone();
    changed.size_bytes += 1;
    fixture
        .store
        .upsert_media_file(&changed)
        .await
        .expect("media");
    worker.plan().await.expect("plan");
    let after = fixture.store.list_optimized_versions().await.expect("list");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].status, OptimizedVersionStatus::Queued);
    assert_eq!(after[0].source_size, changed.size_bytes);
    assert!(!Path::new(&path).exists(), "the stale file is gone");

    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn an_administrators_folder_and_limits_replace_the_configured_ones() {
    let dir = std::env::temp_dir().join(format!("noflex-optimize-it-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mkv");
    if !make_source(&source) {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let fixture = fixture(&source, dir.clone()).await;
    let worker = worker(&fixture, config());
    let chosen = dir.join("chosen");
    std::fs::create_dir_all(&chosen).unwrap();
    // Folders an administrator picks may hold things of theirs.
    let theirs = chosen.join("holiday.mp4");
    std::fs::write(&theirs, b"not ours").unwrap();
    let unknown_version = chosen.join(format!("{}.mp4", uuid::Uuid::now_v7()));
    std::fs::write(&unknown_version, b"left by a crash").unwrap();

    let set = |max_storage_gb, max_version_gb| SmartTranscodingSettings {
        dir: Some(chosen.display().to_string()),
        max_storage_gb,
        max_version_gb,
    };
    fixture
        .store
        .set_smart_transcoding_settings(&set(Some(0), None))
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert!(
        !worker.run_next().await.expect("run"),
        "the setting's quota wins over the configured 200 GB"
    );
    assert!(theirs.exists(), "a file that isn't ours survives the sweep");
    assert!(!unknown_version.exists(), "our own orphan doesn't");

    fixture
        .store
        .set_smart_transcoding_settings(&set(None, Some(1)))
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert!(worker.run_next().await.expect("run"));
    let versions = fixture.store.list_optimized_versions().await.unwrap();
    let path = versions[0].path.as_deref().expect("finished");
    assert!(
        Path::new(path).starts_with(&chosen),
        "written to the chosen folder, not {path}"
    );

    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn nothing_runs_without_quota_or_an_opted_in_device() {
    let dir = std::env::temp_dir().join(format!("noflex-optimize-it-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mkv");
    if !make_source(&source) {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let fixture = fixture(&source, dir.clone()).await;

    let no_room = worker(
        &fixture,
        OptimizeConfig {
            max_storage_gb: 0,
            ..config()
        },
    );
    no_room.plan().await.expect("plan");
    assert!(
        !no_room.run_next().await.expect("run"),
        "a full quota starts no job"
    );

    // Opting out empties the queue on the next plan.
    let mut devices = fixture
        .store
        .devices_for_user(fixture.user)
        .await
        .expect("devices");
    let mut device = devices.pop().expect("device");
    device.smart_transcode = false;
    fixture.store.upsert_device(&device).await.expect("device");
    no_room.plan().await.expect("plan");
    assert!(
        fixture
            .store
            .list_optimized_versions()
            .await
            .expect("list")
            .is_empty()
    );

    std::fs::remove_dir_all(dir).ok();
}
