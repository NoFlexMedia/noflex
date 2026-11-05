// SPDX-License-Identifier: AGPL-3.0-only

use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use axum::http::StatusCode;
use image::{Rgb, RgbImage};
use noflex_core::{EditionId, ImageKind, LibraryId, MediaFileId, MetadataStatus, WorkId, WorkKind};
use noflex_server::{
    images::{ImageQuery, serve_image},
    movie_art::{ART_DIR, MovieArtError, MovieArtWorker},
};
use noflex_storage::{
    Storage,
    domain::{Edition, Image, ImageSource, ImageTarget, Library, LibraryKind, MediaFile, Work},
};
use noflex_storage_turso::TursoStore;
use time::{Duration, OffsetDateTime};

/// Counts its runs next to itself and copies `frame.jpg` to its last argument.
fn fake_ffmpeg(dir: &Path, frame: &RgbImage) -> std::path::PathBuf {
    frame.save(dir.join("frame.jpg")).expect("frame");
    let ffmpeg = dir.join("ffmpeg");
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\nhere=\"$(dirname \"$0\")\"\necho run >> \"$here/runs\"\nout=\nfor arg in \"$@\"; do out=$arg; done\ncp \"$here/frame.jpg\" \"$out\"\n",
    )
    .expect("script");
    let mut permissions = std::fs::metadata(&ffmpeg).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&ffmpeg, permissions).expect("chmod");
    ffmpeg
}

fn runs(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("runs"))
        .map(|log| log.lines().count())
        .unwrap_or(0)
}

fn bright() -> RgbImage {
    RgbImage::from_pixel(1280, 720, Rgb([200, 120, 60]))
}

fn movie(library: LibraryId, title: &str, age: Duration) -> Work {
    let added = OffsetDateTime::now_utc() - age;
    Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
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
        added_at: added,
        updated_at: added,
    }
}

struct Fixture {
    storage: Arc<dyn Storage>,
    library: LibraryId,
    root: std::path::PathBuf,
    cache: std::path::PathBuf,
    temp: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("media");
        std::fs::create_dir(&root).expect("media root");
        let store = TursoStore::open_temp().await.expect("store");
        store.migrate().await.expect("migrate");
        let storage: Arc<dyn Storage> = Arc::new(store);
        let library = Library {
            id: LibraryId::new(),
            name: "Movies".to_string(),
            kind: LibraryKind::Movies,
            paths: vec![root.to_string_lossy().into_owned()],
            ..Default::default()
        };
        storage.upsert_library(&library).await.expect("library");
        Self {
            storage,
            library: library.id,
            cache: temp.path().join("cache"),
            root,
            temp,
        }
    }

    /// A film with a 10-minute file, added `age` ago.
    async fn film(&self, title: &str, age: Duration) -> Work {
        let work = movie(self.library, title, age);
        self.storage.upsert_work(&work).await.expect("work");
        let edition = Edition {
            id: EditionId::new(),
            work_id: work.id,
            name: "Default".to_string(),
            runtime_ms: None,
            intro_start_ms: None,
            intro_end_ms: None,
            credits_start_ms: None,
            chapters: Vec::new(),
        };
        let edition_id = self
            .storage
            .upsert_edition(&edition)
            .await
            .expect("edition");
        let path = self.root.join(format!("{title}.mkv"));
        std::fs::write(&path, b"video").expect("source");
        self.storage
            .upsert_media_file(&MediaFile {
                id: MediaFileId::new(),
                edition_id,
                path: path.to_string_lossy().into_owned(),
                part_index: 0,
                size_bytes: 5,
                mtime: OffsetDateTime::now_utc(),
                container: Some("matroska".to_string()),
                probe: serde_json::json!({ "duration_ms": 600_000 }).into(),
                subtitle_tracks: Vec::new(),
            })
            .await
            .expect("media");
        work
    }

    fn worker(&self, ffmpeg: std::path::PathBuf) -> MovieArtWorker {
        MovieArtWorker::new(self.storage.clone(), ffmpeg, self.cache.clone())
    }

    async fn art(&self, work: &Work, kind: ImageKind) -> Option<Image> {
        self.storage
            .image(ImageTarget::Work(work.id), kind, ImageSource::ALL)
            .await
            .expect("image")
    }

    fn cached_files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.cache.join(ART_DIR))
            .map(|it| {
                it.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

fn settled() -> Duration {
    Duration::hours(2)
}

#[tokio::test]
async fn a_film_with_no_art_gets_a_poster_and_a_backdrop_from_one_frame() {
    let f = Fixture::new().await;
    let work = f.film("Bare", settled()).await;

    f.worker(fake_ffmpeg(f.temp.path(), &bright()))
        .run_sweep()
        .await;

    let poster = f.art(&work, ImageKind::Primary).await.expect("poster");
    let backdrop = f.art(&work, ImageKind::Backdrop).await.expect("backdrop");
    assert_eq!(poster.source, ImageSource::Generated);
    assert_eq!((poster.width, poster.height), (Some(480), Some(720)));
    assert_eq!((backdrop.width, backdrop.height), (Some(1280), Some(720)));
    assert_eq!(runs(f.temp.path()), 1, "one frame serves both");
    assert_eq!(f.cached_files().len(), 2, "no temp files stay behind");

    let response = serve_image(
        &f.storage,
        &f.cache,
        ImageTarget::Work(work.id),
        ImageKind::Primary,
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
async fn only_what_is_missing_is_generated_and_real_art_outranks_it() {
    let f = Fixture::new().await;
    let work = f.film("Half", settled()).await;
    let provider_poster = Image {
        target: ImageTarget::Work(work.id),
        kind: ImageKind::Primary,
        source: ImageSource::Provider,
        path: f.root.join("poster.jpg").to_string_lossy().into_owned(),
        width: None,
        height: None,
        language: None,
    };
    f.storage
        .upsert_image(&provider_poster)
        .await
        .expect("poster");

    f.worker(fake_ffmpeg(f.temp.path(), &bright()))
        .run_sweep()
        .await;

    let poster = f.art(&work, ImageKind::Primary).await.expect("poster");
    assert_eq!(poster.source, ImageSource::Provider);
    let backdrop = f.art(&work, ImageKind::Backdrop).await.expect("backdrop");
    assert_eq!(backdrop.source, ImageSource::Generated);

    let provider_backdrop = Image {
        kind: ImageKind::Backdrop,
        source: ImageSource::Provider,
        ..provider_poster
    };
    f.storage
        .upsert_image(&provider_backdrop)
        .await
        .expect("backdrop");
    let effective = f.art(&work, ImageKind::Backdrop).await.expect("backdrop");
    assert_eq!(effective.source, ImageSource::Provider);
}

#[tokio::test]
async fn a_new_film_waits_for_the_metadata_worker() {
    let f = Fixture::new().await;
    let work = f.film("Fresh", Duration::minutes(1)).await;

    f.worker(fake_ffmpeg(f.temp.path(), &bright()))
        .run_sweep()
        .await;

    assert!(f.art(&work, ImageKind::Primary).await.is_none());
    assert_eq!(runs(f.temp.path()), 0);
}

#[tokio::test]
async fn a_dark_frame_gets_more_looks_and_is_used_if_nothing_is_better() {
    let f = Fixture::new().await;
    let work = f.film("Night", settled()).await;
    let black = RgbImage::from_pixel(1280, 720, Rgb([3, 3, 3]));

    f.worker(fake_ffmpeg(f.temp.path(), &black))
        .run_sweep()
        .await;

    assert_eq!(runs(f.temp.path()), 4);
    assert!(f.art(&work, ImageKind::Primary).await.is_some());
}

#[tokio::test]
async fn a_failed_grab_leaves_nothing_behind_and_is_not_retried_at_once() {
    let f = Fixture::new().await;
    let work = f.film("Broken", settled()).await;
    let ffmpeg = fake_ffmpeg(f.temp.path(), &bright());
    // Writes a partial file, counts the run, then fails.
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\necho run >> \"$(dirname \"$0\")/runs\"\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf partial > \"$out\"\nexit 1\n",
    )
    .expect("script");

    let worker = f.worker(ffmpeg);
    worker.run_sweep().await;
    worker.run_sweep().await;

    assert!(f.art(&work, ImageKind::Primary).await.is_none());
    assert!(f.cached_files().is_empty());
    assert_eq!(runs(f.temp.path()), 1);
}

#[tokio::test]
async fn generate_refuses_what_it_cannot_grab_from() {
    let f = Fixture::new().await;
    let worker = f.worker(fake_ffmpeg(f.temp.path(), &bright()));

    let mut show = movie(f.library, "Show", settled());
    show.kind = WorkKind::Series;
    assert!(matches!(
        worker.generate(&show).await,
        Err(MovieArtError::NotMovie)
    ));

    let empty = movie(f.library, "Empty", settled());
    f.storage.upsert_work(&empty).await.expect("work");
    assert!(matches!(
        worker.generate(&empty).await,
        Err(MovieArtError::NoMedia)
    ));
}
