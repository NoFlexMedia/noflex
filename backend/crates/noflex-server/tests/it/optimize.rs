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
        Device, Edition, Library, MediaFile, OptimizedVersionStatus, SmartTranscodeMark,
        SmartTranscodeOutcome, SmartTranscodingSettings, User, Work,
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
        username: "viewer".to_string(),
        display_name: None,
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: true,
        role: noflex_core::UserRole::Admin,
        created_at: now,
        avatar_updated_at: None,
        has_pin: false,
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
        metadata_provider: None,
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
            client: None,
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
        wake: Default::default(),
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

/// The fixture's device, which is the only one.
async fn phone(fixture: &Fixture) -> Device {
    fixture
        .store
        .devices_for_user(fixture.user)
        .await
        .expect("devices")
        .pop()
        .expect("device")
}

async fn turn_smart_transcoding_off(fixture: &Fixture) {
    let mut device = phone(fixture).await;
    device.smart_transcode = false;
    fixture.store.upsert_device(&device).await.expect("device");
}

async fn work_of(fixture: &Fixture, media: &MediaFile) -> WorkId {
    fixture
        .store
        .work_for_media_file(media.id)
        .await
        .expect("work")
        .expect("work")
        .id
}

async fn mark(fixture: &Fixture, work: WorkId, device: Option<DeviceId>) {
    fixture
        .store
        .add_smart_transcode_mark(&SmartTranscodeMark {
            user_id: fixture.user,
            work_id: work,
            device_id: device,
            created_at: OffsetDateTime::now_utc(),
            outcome: None,
            media_file_ids: vec![],
        })
        .await
        .expect("mark");
}

async fn only_mark(fixture: &Fixture) -> SmartTranscodeMark {
    let mut marks = fixture
        .store
        .smart_transcode_marks_for_user(fixture.user)
        .await
        .expect("marks");
    assert_eq!(marks.len(), 1, "{marks:?}");
    marks.pop().expect("mark")
}

/// A second film from the same file, for a test that needs two titles.
async fn another_title(fixture: &Fixture) -> MediaFile {
    let now = OffsetDateTime::now_utc();
    let original = fixture
        .store
        .work_for_media_file(fixture.media.id)
        .await
        .expect("work")
        .expect("work");
    let work = Work {
        id: WorkId::new(),
        title: "Another".to_string(),
        sort_title: "another".to_string(),
        ..original
    };
    fixture.store.upsert_work(&work).await.expect("work");
    let edition = Edition {
        id: EditionId::new(),
        work_id: work.id,
        name: "Default".to_string(),
        runtime_ms: Some(10_000),
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    fixture
        .store
        .upsert_edition(&edition)
        .await
        .expect("edition");
    let path = format!("{}.copy.mkv", fixture.media.path);
    std::fs::copy(&fixture.media.path, &path).expect("copy");
    let media = MediaFile {
        id: MediaFileId::new(),
        edition_id: edition.id,
        path,
        mtime: now,
        ..fixture.media.clone()
    };
    fixture
        .store
        .upsert_media_file(&media)
        .await
        .expect("media");
    media
}

#[tokio::test]
async fn a_mark_is_for_the_devices_it_names_until_its_taken_back() {
    let dir = std::env::temp_dir().join(format!("noflex-mark-scope-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mkv");
    if !make_source(&source) {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let fixture = fixture(&source, dir.clone()).await;
    turn_smart_transcoding_off(&fixture).await;
    let worker = worker(&fixture, config());
    let work = work_of(&fixture, &fixture.media).await;
    let device = phone(&fixture).await.id;

    // For all devices with it on, of which there are none: nothing to do, and saying so.
    mark(&fixture, work, None).await;
    worker.plan().await.expect("plan");
    assert_eq!(
        only_mark(&fixture).await.outcome,
        Some(SmartTranscodeOutcome::NoDevices)
    );
    assert!(
        fixture
            .store
            .list_optimized_versions()
            .await
            .unwrap()
            .is_empty()
    );

    // For this device, whatever its setting: queued, and ahead of any guess.
    fixture
        .store
        .remove_smart_transcode_mark(fixture.user, work, None)
        .await
        .unwrap();
    mark(&fixture, work, Some(device)).await;
    worker.plan().await.expect("plan");
    let planned = only_mark(&fixture).await;
    assert_eq!(planned.outcome, Some(SmartTranscodeOutcome::Planned));
    assert_eq!(planned.media_file_ids, [fixture.media.id]);
    let queued = fixture.store.list_optimized_versions().await.unwrap();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(
        queued[0].priority < 0,
        "ahead of guesses, which count from 0"
    );

    // Unmarked, it leaves the queue.
    fixture
        .store
        .remove_smart_transcode_mark(fixture.user, work, Some(device))
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert!(
        fixture
            .store
            .list_optimized_versions()
            .await
            .unwrap()
            .is_empty()
    );

    // A device that's been forgotten takes its mark with it.
    mark(&fixture, work, Some(device)).await;
    fixture
        .store
        .delete_device(fixture.user, device)
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert!(
        fixture
            .store
            .smart_transcode_marks_for_user(fixture.user)
            .await
            .unwrap()
            .is_empty()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn watching_a_marked_title_ends_the_mark_but_having_seen_it_doesnt() {
    let dir = std::env::temp_dir().join(format!("noflex-mark-watch-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mkv");
    if !make_source(&source) {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let fixture = fixture(&source, dir.clone()).await;
    let worker = worker(&fixture, config());
    let work = work_of(&fixture, &fixture.media).await;

    // Seen before it was marked: they mean to see it again.
    fixture
        .store
        .mark_played(fixture.user, PlayTarget::Work(work), true)
        .await
        .expect("played");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    mark(&fixture, work, None).await;
    worker.plan().await.expect("plan");
    assert_eq!(
        only_mark(&fixture).await.outcome,
        Some(SmartTranscodeOutcome::Planned)
    );

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    fixture
        .store
        .mark_played(fixture.user, PlayTarget::Work(work), true)
        .await
        .expect("played");
    worker.plan().await.expect("plan");
    assert!(
        fixture
            .store
            .smart_transcode_marks_for_user(fixture.user)
            .await
            .unwrap()
            .is_empty(),
        "watched since it was marked"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_mark_waiting_for_space_evicts_guesses_and_is_never_evicted_itself() {
    let dir = std::env::temp_dir().join(format!("noflex-mark-space-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mkv");
    if !make_source(&source) {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let fixture = fixture(&source, dir.clone()).await;
    let worker = worker(&fixture, config());
    const TWO_GB: u64 = 2 * 1024 * 1024 * 1024;

    // A guess, prepared, and taking up the whole of the server's space. This
    // leaves it at the quota, not over, so nothing evicts it for its own sake.
    worker.plan().await.expect("plan");
    assert!(worker.run_next().await.expect("run"));
    let mut guess = fixture
        .store
        .list_optimized_versions()
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        guess.status,
        OptimizedVersionStatus::Ready,
        "{:?}",
        guess.error
    );
    guess.size_bytes = Some(TWO_GB);
    fixture
        .store
        .update_optimized_version(&guess)
        .await
        .unwrap();
    fixture
        .store
        .set_smart_transcoding_settings(&SmartTranscodingSettings {
            max_storage_gb: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert!(
        fixture
            .store
            .get_optimized_version(guess.id)
            .await
            .unwrap()
            .is_some(),
        "a guess at the quota stays until something needs the room"
    );

    let other = another_title(&fixture).await;
    let other_work = work_of(&fixture, &other).await;
    mark(&fixture, other_work, None).await;
    worker.plan().await.expect("plan");
    assert!(
        fixture
            .store
            .get_optimized_version(guess.id)
            .await
            .unwrap()
            .is_none(),
        "the guess made room for what was marked"
    );

    assert!(worker.run_next().await.expect("run"), "and it runs");
    let mut marked = fixture
        .store
        .optimized_versions_for_media(other.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        marked.status,
        OptimizedVersionStatus::Ready,
        "{:?}",
        marked.error
    );
    marked.size_bytes = Some(TWO_GB * 2);
    fixture
        .store
        .update_optimized_version(&marked)
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert!(
        fixture
            .store
            .get_optimized_version(marked.id)
            .await
            .unwrap()
            .is_some(),
        "over the quota, but marked"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_copy_left_unwanted_and_unplayed_is_swept_up_while_there_is_room() {
    let dir = std::env::temp_dir().join(format!("noflex-sweep-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mkv");
    if !make_source(&source) {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let fixture = fixture(&source, dir.clone()).await;
    let worker = worker(&fixture, config());
    let two_days_ago = OffsetDateTime::now_utc() - time::Duration::days(2);

    worker.plan().await.expect("plan");
    assert!(worker.run_next().await.expect("run"));
    let ready = fixture
        .store
        .list_optimized_versions()
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        ready.status,
        OptimizedVersionStatus::Ready,
        "{:?}",
        ready.error
    );
    let file = std::path::PathBuf::from(ready.path.clone().expect("path"));
    assert!(file.exists());
    let stored = async || fixture.store.get_optimized_version(ready.id).await.unwrap();

    // Still wanted, so it doesn't matter what an earlier plan thought.
    fixture
        .store
        .mark_optimized_version_unwanted(ready.id, two_days_ago)
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert_eq!(stored().await.unwrap().unwanted_since, None);

    turn_smart_transcoding_off(&fixture).await;
    worker.plan().await.expect("plan");
    assert!(
        stored().await.expect("kept").unwanted_since.is_some(),
        "out of a plan once is no reason to remake it later"
    );

    let mut played = stored().await.unwrap();
    played.last_used_at = Some(OffsetDateTime::now_utc());
    fixture
        .store
        .update_optimized_version(&played)
        .await
        .unwrap();
    let mut device = phone(&fixture).await;
    device.smart_transcode = true;
    fixture.store.upsert_device(&device).await.unwrap();
    worker.plan().await.expect("plan");
    fixture
        .store
        .mark_optimized_version_unwanted(ready.id, two_days_ago)
        .await
        .unwrap();
    turn_smart_transcoding_off(&fixture).await;
    worker.plan().await.expect("plan");
    assert!(stored().await.is_some(), "someone's just played it");

    played.last_used_at = Some(two_days_ago);
    fixture
        .store
        .update_optimized_version(&played)
        .await
        .unwrap();
    worker.plan().await.expect("plan");
    assert!(stored().await.is_none());
    assert!(!file.exists(), "and it frees the disk");
}
