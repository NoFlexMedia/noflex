// SPDX-License-Identifier: AGPL-3.0-only
//
// `SubtitleService` end to end over the subtitle fixture: the query it
// builds from storage and the file, downloads kept as UTF-8 files in its
// own directory and shared by everyone, formats the pipeline can't read
// turned away, and the sweep of unused and orphaned files.

use std::{sync::Arc, time::Duration};

use noflex_core::{
    EditionId, LibraryId, MediaFileId, MetadataStatus, SubtitleCodec, UserId, WorkId,
    model::{ExternalId, ExternalIdKind, WorkKind},
};
use noflex_plugin_host::{
    EngineConfig, PluginRegistry, SubtitleError, SubtitleService, build_engine,
};
use noflex_storage::{
    Storage,
    domain::{Edition, Library, LibraryKind, MediaFile, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

struct Harness {
    service: SubtitleService,
    storage: Arc<dyn Storage>,
    media: MediaFile,
    cache: tempfile::TempDir,
    _library: tempfile::TempDir,
}

fn work(
    library: LibraryId,
    kind: WorkKind,
    title: &str,
    parent: Option<WorkId>,
    ids: Vec<ExternalId>,
) -> Work {
    let now = OffsetDateTime::now_utc();
    Work {
        id: WorkId::new(),
        kind,
        parent_id: parent,
        library_id: library,
        title: title.into(),
        sort_title: title.to_lowercase(),
        year: None,
        release_date: None,
        overview: None,
        external_ids: ids,
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
    }
}

fn id(kind: ExternalIdKind, value: &str) -> ExternalId {
    ExternalId {
        kind,
        value: value.into(),
    }
}

/// An episode, S02E06 of a series with a TMDb id, with a 128 KiB file.
async fn harness(episode_title: &str) -> Option<Harness> {
    let wasm = crate::fixtures::wasm("subtitle")?;
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry.load_from(&wasm).expect("load subtitle fixture");

    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let library_dir = tempfile::tempdir().expect("library dir");
    let library = Library {
        id: LibraryId::new(),
        name: "Shows".into(),
        kind: LibraryKind::Shows,
        paths: vec![library_dir.path().to_string_lossy().into_owned()],
        ..Default::default()
    };
    storage.upsert_library(&library).await.expect("library");
    let mut series = work(
        library.id,
        WorkKind::Series,
        "Show",
        None,
        vec![id(ExternalIdKind::Tmdb, "1403")],
    );
    series.year = Some(2013);
    let mut season = work(
        library.id,
        WorkKind::Season,
        "Season 2",
        Some(series.id),
        vec![],
    );
    season.index_number = Some(2);
    let mut episode = work(
        library.id,
        WorkKind::Episode,
        episode_title,
        Some(season.id),
        vec![
            id(ExternalIdKind::Imdb, "tt4078580"),
            id(ExternalIdKind::Custom, "/shows/Show/Season 2/e6"),
        ],
    );
    episode.index_number = Some(6);
    for w in [&series, &season, &episode] {
        storage.upsert_work(w).await.expect("work");
    }
    let edition = Edition {
        id: EditionId::new(),
        work_id: episode.id,
        name: "Default".into(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    storage.upsert_edition(&edition).await.expect("edition");
    let path = library_dir.path().join("Show.S02E06.mkv");
    std::fs::write(&path, vec![0_u8; 128 * 1024]).expect("media file");
    let media = MediaFile {
        id: MediaFileId::new(),
        edition_id: edition.id,
        path: path.to_string_lossy().into_owned(),
        part_index: 0,
        size_bytes: 128 * 1024,
        mtime: OffsetDateTime::now_utc(),
        container: Some("matroska".into()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: vec![],
    };
    storage.upsert_media_file(&media).await.expect("media");

    let cache = tempfile::tempdir().expect("cache");
    let service = SubtitleService::new(
        Arc::new(registry),
        storage.clone(),
        cache.path().join("subtitles"),
    );
    Some(Harness {
        service,
        storage,
        media,
        cache,
        _library: library_dir,
    })
}

fn cached_files(h: &Harness) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(h.cache.path().join("subtitles")) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn the_query_carries_the_file_the_episode_and_its_series() {
    let Some(h) = harness("A Fractured House").await else {
        eprintln!("skipping: could not build the subtitle fixture");
        return;
    };
    let search = h
        .service
        .search(&h.media, &["en".into()])
        .await
        .expect("search");
    let echoed = search
        .matches
        .iter()
        .find(|m| m.handle == "srt:eng:ids")
        .and_then(|m| m.release.clone())
        .expect("the id match spells out the query");
    // 128 KiB of zeros hashes to its size alone.
    assert_eq!(
        echoed,
        "WorkKind::Episode|A Fractured House|Some(2013)|ExternalIdKind::Imdb=tt4078580|\
         ExternalIdKind::Tmdb=1403|Some(2)|Some(6)|Show.S02E06.mkv|131072|\
         Some(\"0000000000020000\")"
    );
}

#[tokio::test]
async fn a_download_is_kept_once_in_the_cache_as_utf8() {
    let Some(h) = harness("Pilot").await else {
        eprintln!("skipping: could not build the subtitle fixture");
        return;
    };
    let unsearched = h
        .service
        .download(&h.media, "fixture-subtitle", "srt:eng:hash", None)
        .await;
    assert!(matches!(unsearched, Err(SubtitleError::NotOffered)));

    h.service
        .search(&h.media, &["eng".into()])
        .await
        .expect("search");
    let viewer = noflex_storage::domain::User {
        id: UserId::new(),
        username: "viewer".into(),
        display_name: None,
        password_hash: String::new(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    };
    h.storage.upsert_user(&viewer).await.expect("viewer");
    let viewer = viewer.id;
    let kept = h
        .service
        .download(&h.media, "fixture-subtitle", "srt:eng:hash", Some(viewer))
        .await
        .expect("download");
    assert_eq!(kept.media_file_id, h.media.id);
    assert_eq!(kept.format, SubtitleCodec::SubRip);
    assert_eq!(kept.language, "eng");
    assert!(kept.hash_match, "flags come from the offer");
    assert_eq!(kept.downloaded_by, Some(viewer));
    assert_eq!(kept.file_name, format!("{}.srt", kept.id));
    let text = std::fs::read_to_string(h.service.path(&kept)).expect("cached file");
    assert_eq!(text, "1\n00:00:01,000 --> 00:00:02,500\nCafé\n");
    assert!(
        h.service.path(&kept).starts_with(h.cache.path()),
        "never beside the media"
    );

    let again = h
        .service
        .download(&h.media, "fixture-subtitle", "srt:eng:hash", None)
        .await
        .expect("again");
    assert_eq!(again.id, kept.id);
    assert_eq!(cached_files(&h), std::slice::from_ref(&kept.file_name));

    let removed = h.service.delete(kept.id).await.expect("delete");
    assert_eq!(removed.map(|s| s.id), Some(kept.id));
    assert!(cached_files(&h).is_empty());
}

#[tokio::test]
async fn unreadable_downloads_leave_nothing_behind() {
    let Some(h) = harness("Formats").await else {
        eprintln!("skipping: could not build the subtitle fixture");
        return;
    };
    h.service.search(&h.media, &[]).await.expect("search");
    let ass = h
        .service
        .download(&h.media, "fixture-subtitle", "ass:eng", None)
        .await
        .expect("ASS is kept as ASS");
    assert_eq!(ass.format, SubtitleCodec::Ass);
    assert!(ass.file_name.ends_with(".ass"));

    for (handle, expected) in [("txt:eng", "unsupported"), ("empty:eng", "no subtitles")] {
        let error = h
            .service
            .download(&h.media, "fixture-subtitle", handle, None)
            .await
            .expect_err(handle);
        assert!(error.to_string().contains(expected), "{handle}: {error}");
    }
    assert_eq!(cached_files(&h), std::slice::from_ref(&ass.file_name));
    assert_eq!(
        h.storage
            .downloaded_subtitles_for_media(h.media.id)
            .await
            .expect("list")
            .len(),
        1
    );
}

#[tokio::test]
async fn the_sweep_takes_unused_subtitles_and_orphaned_files() {
    let Some(h) = harness("Pilot").await else {
        eprintln!("skipping: could not build the subtitle fixture");
        return;
    };
    h.service
        .search(&h.media, &["eng".into()])
        .await
        .expect("search");
    let kept = h
        .service
        .download(&h.media, "fixture-subtitle", "srt:eng:ids", None)
        .await
        .expect("download");

    let dir = h.cache.path().join("subtitles");
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    let orphan = dir.join(format!("{}.srt", uuid::Uuid::now_v7()));
    let fresh_orphan = dir.join(format!("{}.srt", uuid::Uuid::now_v7()));
    for path in [&orphan, &fresh_orphan] {
        std::fs::write(path, "1\n00:00:01,000 --> 00:00:02,000\nx\n").expect("orphan");
    }
    std::fs::File::options()
        .write(true)
        .open(&orphan)
        .and_then(|f| f.set_modified(old))
        .expect("age the orphan");
    std::fs::File::options()
        .write(true)
        .open(h.service.path(&kept))
        .and_then(|f| f.set_modified(old))
        .expect("age the kept file");

    let removed = h
        .service
        .sweep(Duration::from_secs(24 * 60 * 60))
        .await
        .expect("sweep");
    assert_eq!(removed, 0, "used just now");
    let mut left = cached_files(&h);
    left.retain(|name| name != &kept.file_name);
    assert_eq!(
        left,
        [fresh_orphan
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()],
        "an old file without a record goes; one being written stays"
    );

    let removed = h.service.sweep(Duration::ZERO).await.expect("sweep");
    assert_eq!(removed, 1);
    assert!(!h.service.path(&kept).exists());
    assert_eq!(
        h.storage
            .get_downloaded_subtitle(kept.id)
            .await
            .expect("get"),
        None
    );
}
