// SPDX-License-Identifier: AGPL-3.0-only

use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use axum::http::StatusCode;
use noflex_core::{EditionId, ImageKind, LibraryId, MediaFileId, MetadataStatus, WorkId, WorkKind};
use noflex_server::{
    episode_thumbs::{EpisodeThumbError, EpisodeThumbWorker, THUMB_DIR},
    images::{ImageQuery, serve_image},
};
use noflex_storage::{
    Storage,
    domain::{Edition, Image, ImageSource, ImageTarget, Library, LibraryKind, MediaFile, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod");
}

/// Records its argv (one per line) next to itself and writes a stub
/// JPEG to its last argument.
fn fake_ffmpeg(dir: &Path) -> std::path::PathBuf {
    let ffmpeg = dir.join("ffmpeg");
    executable(
        &ffmpeg,
        "#!/bin/sh\nlog=\"$(dirname \"$0\")/argv\"\n: > \"$log\"\nout=\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> \"$log\"; out=$arg; done\nprintf jpeg > \"$out\"\n",
    );
    ffmpeg
}

fn work(library: LibraryId, kind: WorkKind, parent: Option<WorkId>, title: &str) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind,
        parent_id: parent,
        library_id: library,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
        year: None,
        release_date: None,
        overview: None,
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: Vec::new(),
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        locked_fields: Vec::new(),
        added_at: now,
        updated_at: now,
    }
}

async fn attach_media(storage: &dyn Storage, work: WorkId, runtime_ms: Option<i64>, path: &Path) {
    let edition = Edition {
        id: EditionId::new(),
        work_id: work,
        name: "Default".to_string(),
        runtime_ms,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: Vec::new(),
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
    let edition_id = storage.upsert_edition(&edition).await.expect("edition");
    std::fs::write(path, b"video").expect("source");
    storage
        .upsert_media_file(&MediaFile {
            id: MediaFileId::new(),
            edition_id,
            path: path.to_string_lossy().into_owned(),
            part_index: 0,
            size_bytes: 5,
            mtime: OffsetDateTime::now_utc(),
            container: Some("matroska".to_string()),
            probe: serde_json::json!({}).into(),
            subtitle_tracks: Vec::new(),
        })
        .await
        .expect("media");
}

struct Fixture {
    storage: Arc<dyn Storage>,
    /// Episode with media and a 10-minute runtime, no thumb.
    bare: Work,
    /// Episode that already has a local thumb.
    local: Work,
    local_thumb: String,
}

async fn seed(root: &Path) -> Fixture {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let library = Library {
        id: LibraryId::new(),
        name: "Shows".to_string(),
        kind: LibraryKind::Shows,
        paths: vec![root.to_string_lossy().into_owned()],
        ..Default::default()
    };
    storage.upsert_library(&library).await.expect("library");
    let series = work(library.id, WorkKind::Series, None, "Show");
    let season = work(library.id, WorkKind::Season, Some(series.id), "Season 1");
    let bare = work(library.id, WorkKind::Episode, Some(season.id), "Pilot");
    let local = work(library.id, WorkKind::Episode, Some(season.id), "Second");
    for w in [&series, &season, &bare, &local] {
        storage.upsert_work(w).await.expect("work");
    }
    attach_media(
        storage.as_ref(),
        bare.id,
        Some(600_000),
        &root.join("e1.mkv"),
    )
    .await;
    attach_media(
        storage.as_ref(),
        local.id,
        Some(600_000),
        &root.join("e2.mkv"),
    )
    .await;
    let local_thumb = root.join("e2.jpg").to_string_lossy().into_owned();
    storage
        .upsert_image(&Image {
            target: ImageTarget::Work(local.id),
            kind: ImageKind::Thumb,
            source: ImageSource::Local,
            path: local_thumb.clone(),
            width: None,
            height: None,
            language: None,
        })
        .await
        .expect("local thumb");
    Fixture {
        storage,
        bare,
        local,
        local_thumb,
    }
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test]
async fn sweep_grabs_one_frame_for_episodes_without_a_thumb() {
    let temp = tempfile::tempdir().expect("temp");
    let media_root = temp.path().join("media");
    std::fs::create_dir(&media_root).expect("media root");
    let cache = temp.path().join("cache");
    let f = seed(&media_root).await;

    EpisodeThumbWorker::new(f.storage.clone(), fake_ffmpeg(temp.path()), cache.clone())
        .run_sweep()
        .await;

    let thumb = f
        .storage
        .image(
            ImageTarget::Work(f.bare.id),
            ImageKind::Thumb,
            ImageSource::ALL,
        )
        .await
        .expect("get")
        .expect("generated thumb");
    let expected = cache.join(THUMB_DIR).join(format!("{}.jpg", f.bare.id));
    assert_eq!(thumb.path, expected.to_string_lossy());
    assert_eq!(std::fs::read(&expected).expect("thumb file"), b"jpeg");
    assert_eq!(
        entries(&cache.join(THUMB_DIR)),
        vec![format!("{}.jpg", f.bare.id)],
        "no temp files left, and nothing for the episode with a local thumb"
    );

    let argv = std::fs::read_to_string(temp.path().join("argv")).expect("argv");
    // 20% of the 10-minute runtime, input-side seek, one scaled frame.
    assert!(argv.starts_with("-hide_banner\n-loglevel\nerror\n-ss\n120.000\n-i\n"));
    assert!(argv.contains("\nscale=640:-2\n-frames:v\n1\n"));

    let local = f
        .storage
        .image(
            ImageTarget::Work(f.local.id),
            ImageKind::Thumb,
            ImageSource::ALL,
        )
        .await
        .expect("get")
        .expect("local thumb");
    assert_eq!(local.path, f.local_thumb);

    // The image route serves cache-dir files, so the row is usable as-is.
    let response = serve_image(
        &f.storage,
        &cache,
        ImageTarget::Work(f.bare.id),
        ImageKind::Thumb,
        ImageQuery {
            w: None,
            h: None,
            fill: None,
            source: None,
            token: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn failed_grabs_leave_nothing_behind_and_back_off() {
    let temp = tempfile::tempdir().expect("temp");
    let media_root = temp.path().join("media");
    std::fs::create_dir(&media_root).expect("media root");
    let cache = temp.path().join("cache");
    let f = seed(&media_root).await;
    let ffmpeg = temp.path().join("ffmpeg-bad");
    // Writes a partial file, counts invocations, then fails.
    executable(
        &ffmpeg,
        "#!/bin/sh\necho run >> \"$(dirname \"$0\")/runs\"\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf partial > \"$out\"\nexit 3\n",
    );

    let worker = EpisodeThumbWorker::new(f.storage.clone(), ffmpeg, cache.clone());
    worker.run_sweep().await;
    worker.run_sweep().await;

    assert!(
        f.storage
            .image(
                ImageTarget::Work(f.bare.id),
                ImageKind::Thumb,
                ImageSource::ALL,
            )
            .await
            .expect("get")
            .is_none()
    );
    assert!(entries(&cache.join(THUMB_DIR)).is_empty());
    let runs = std::fs::read_to_string(temp.path().join("runs")).expect("runs");
    assert_eq!(runs.lines().count(), 1, "second sweep is backed off");
}

#[tokio::test]
async fn only_episodes_with_a_known_runtime_are_grabbed() {
    let temp = tempfile::tempdir().expect("temp");
    let media_root = temp.path().join("media");
    std::fs::create_dir(&media_root).expect("media root");
    let f = seed(&media_root).await;
    let worker = EpisodeThumbWorker::new(
        f.storage.clone(),
        temp.path().join("missing-ffmpeg"),
        temp.path().join("cache"),
    );

    let mut movie = f.bare.clone();
    movie.id = WorkId::new();
    movie.kind = WorkKind::Movie;
    assert!(matches!(
        worker.generate(&movie).await,
        Err(EpisodeThumbError::NotEpisode)
    ));

    let mut unknown = f.bare.clone();
    unknown.id = WorkId::new();
    f.storage.upsert_work(&unknown).await.expect("work");
    attach_media(
        f.storage.as_ref(),
        unknown.id,
        None,
        &media_root.join("e3.mkv"),
    )
    .await;
    assert!(matches!(
        worker.generate(&unknown).await,
        Err(EpisodeThumbError::MissingDuration)
    ));

    let mut empty = f.bare.clone();
    empty.id = WorkId::new();
    f.storage.upsert_work(&empty).await.expect("work");
    assert!(matches!(
        worker.generate(&empty).await,
        Err(EpisodeThumbError::NoMedia)
    ));
}
