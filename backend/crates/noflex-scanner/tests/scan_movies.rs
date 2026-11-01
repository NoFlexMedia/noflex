// SPDX-License-Identifier: AGPL-3.0-only
//
// Movies-library integration tests.
//
// Scanner identity is filesystem-location-based: same folder = same Movie
// (multi-part / multi-edition); different folder OR bare-vs-folder = SEPARATE
// candidate Works. Merging across candidates is the metadata provider's job,
// which is intentionally not exercised here.

use noflex_core::{ExternalIdKind, MetadataStatus};
use noflex_scanner::{FilesystemScanner, ScanSummary, slugify};
use noflex_storage::domain::LibraryKind;

mod common;

fn folder_key(lib: uuid::Uuid, folder_rel: &str) -> String {
    format!("fs:movie:{lib}:folder:{}", slugify(folder_rel))
}

fn bare_key(lib: uuid::Uuid, stem_rel: &str) -> String {
    format!("fs:movie:{lib}:bare:{}", slugify(stem_rel))
}

// ---- 1. Plex-strict layout ----------------------------------------------

#[tokio::test]
async fn plex_strict_layout_basic() {
    let h = common::build(LibraryKind::Movies).await;

    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"matrix-v1",
    );
    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception (2010).mkv",
        b"inception-default",
    );
    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception (2010) {edition-Director's Cut}.mkv",
        b"inception-directors-cut",
    );
    common::touch(
        &h.lib_root,
        "Blade Runner (1982)/Blade Runner (1982) - cd1.mkv",
        b"br-cd1",
    );
    common::touch(
        &h.lib_root,
        "Blade Runner (1982)/Blade Runner (1982) - cd2.mkv",
        b"br-cd2",
    );
    // Decoy non-video file.
    common::touch(&h.lib_root, "Blade Runner (1982)/cover.jpg", b"jpg");

    let scanner = FilesystemScanner::new(h.storage.clone());

    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(
        summary,
        ScanSummary {
            added: 5,
            updated: 0,
            removed: 0,
            errored: 0,
        }
    );

    let lib_uuid = h.library_id.into_uuid();

    // Matrix: one Work, one Default edition.
    let matrix = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(lib_uuid, "The Matrix (1999)"),
        )
        .await
        .expect("matrix lookup");
    assert_eq!(matrix.len(), 1);
    let editions = h.storage.editions_for_work(matrix[0].id).await.unwrap();
    assert_eq!(editions.len(), 1);
    assert_eq!(editions[0].name, "Default");

    // Inception: same Work (folder identity), two editions.
    let inception = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(lib_uuid, "Inception (2010)"),
        )
        .await
        .expect("inception lookup");
    assert_eq!(inception.len(), 1);
    let inception_eds = h.storage.editions_for_work(inception[0].id).await.unwrap();
    let mut names: Vec<_> = inception_eds.iter().map(|e| e.name.clone()).collect();
    names.sort();
    assert_eq!(names, vec!["Default", "Director's Cut"]);

    // Blade Runner: one Work, one Default edition, two parts.
    let br = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(lib_uuid, "Blade Runner (1982)"),
        )
        .await
        .expect("br lookup");
    assert_eq!(br.len(), 1);
    let br_eds = h.storage.editions_for_work(br[0].id).await.unwrap();
    assert_eq!(br_eds.len(), 1);
    let mut br_parts: Vec<u32> = h
        .storage
        .media_files_for_edition(br_eds[0].id)
        .await
        .unwrap()
        .iter()
        .map(|m| m.part_index)
        .collect();
    br_parts.sort();
    assert_eq!(br_parts, vec![1, 2]);
}

#[tokio::test]
async fn concurrent_scans_for_one_library_are_serialized() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"matrix",
    );
    let first = FilesystemScanner::new(h.storage.clone());
    let second = FilesystemScanner::new(h.storage.clone());
    let (first, second) = tokio::join!(
        first.scan_library(h.library_id),
        second.scan_library(h.library_id)
    );
    let mut summaries = [first.unwrap(), second.unwrap()];
    summaries.sort_by_key(|summary| summary.added);
    assert_eq!(summaries[0], ScanSummary::default());
    assert_eq!(summaries[1].added, 1);
    let works = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(h.library_id.into_uuid(), "The Matrix (1999)"),
        )
        .await
        .unwrap();
    assert_eq!(works.len(), 1);
}

#[tokio::test]
async fn plex_strict_idempotent_rescan() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"matrix",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner
        .scan_library(h.library_id)
        .await
        .expect("first scan");
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(summary, ScanSummary::default());
}

#[tokio::test]
async fn plex_strict_remove_detected() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"matrix",
    );
    common::touch(&h.lib_root, "Inception (2010)/Inception (2010).mkv", b"i");
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner
        .scan_library(h.library_id)
        .await
        .expect("first scan");
    let inception = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(h.library_id.into_uuid(), "Inception (2010)"),
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
    let edition_id = h.storage.editions_for_work(inception.id).await.unwrap()[0].id;

    common::unlink(&h.lib_root, "Inception (2010)/Inception (2010).mkv");
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(
        summary,
        ScanSummary {
            added: 0,
            updated: 0,
            removed: 1,
            errored: 0,
        }
    );
    assert!(h.storage.get_work(inception.id).await.unwrap().is_none());
    assert!(h.storage.get_edition(edition_id).await.unwrap().is_none());
}

#[tokio::test]
async fn excluded_folders_use_root_relative_component_boundaries() {
    let h = common::build(LibraryKind::Movies).await;
    let mut library = h
        .storage
        .list_libraries()
        .await
        .unwrap()
        .into_iter()
        .find(|library| library.id == h.library_id)
        .unwrap();
    library.config.excluded_folders = vec!["incoming/./failed".to_string()];
    h.storage.upsert_library(&library).await.unwrap();

    common::touch(&h.lib_root, "incoming/failed/Hidden/Hidden.mkv", b"hidden");
    common::touch(
        &h.lib_root,
        "incoming/failed-extra/Visible/Visible.mkv",
        b"visible",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.unwrap();
    assert_eq!(summary.added, 1);

    library.config.excluded_folders = vec!["../outside".to_string()];
    h.storage.upsert_library(&library).await.unwrap();
    assert!(scanner.scan_library(h.library_id).await.is_err());
}

#[tokio::test]
async fn plex_strict_update_detected() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"original",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner
        .scan_library(h.library_id)
        .await
        .expect("first scan");

    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"resized-larger-content",
    );
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(
        summary,
        ScanSummary {
            added: 0,
            updated: 1,
            removed: 0,
            errored: 0,
        }
    );
}

// ---- 2. Bare-vs-folder same title — must NOT merge ----------------------

#[tokio::test]
async fn bare_file_and_folder_candidate_stay_separate() {
    let h = common::build(LibraryKind::Movies).await;

    // Two candidates that resolve to the same canonical movie. They MUST
    // remain separate Works at scan time; provider integration is what
    // merges them later via TMDb/IMDb ID matching.
    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception.mkv",
        b"folder-version",
    );
    common::touch(&h.lib_root, "Inception.mkv", b"bare-version");

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(
        summary,
        ScanSummary {
            added: 2,
            updated: 0,
            removed: 0,
            errored: 0,
        }
    );

    let lib_uuid = h.library_id.into_uuid();
    let folder = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(lib_uuid, "Inception (2010)"),
        )
        .await
        .unwrap();
    let bare = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &bare_key(lib_uuid, "Inception"))
        .await
        .unwrap();
    assert_eq!(folder.len(), 1, "folder candidate exists");
    assert_eq!(bare.len(), 1, "bare candidate exists");
    assert_ne!(
        folder[0].id, bare[0].id,
        "the two candidates are SEPARATE Works (merge belongs to the provider)"
    );

    // Both folder candidate and bare candidate parsed correctly:
    assert_eq!(folder[0].title, "Inception");
    assert_eq!(folder[0].year, Some(2010));
    assert_eq!(bare[0].title, "Inception");
    assert_eq!(
        bare[0].year, None,
        "bare file has no year signal in the name"
    );

    // Both candidates are scanner-derived, so both land Unconfirmed.
    assert_eq!(folder[0].metadata_status, MetadataStatus::Unconfirmed);
    assert_eq!(bare[0].metadata_status, MetadataStatus::Unconfirmed);
}

// ---- 3. Multi-dash torrent title without a year -------------------------

#[tokio::test]
async fn multi_dash_torrent_title_preserved() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "Mission - Impossible - Ghost Protocol - 1080p AV1 BluRay.mp4",
        b"mi4",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 1);
    assert_eq!(summary.errored, 0);

    let lib_uuid = h.library_id.into_uuid();
    let key = bare_key(
        lib_uuid,
        "Mission - Impossible - Ghost Protocol - 1080p AV1 BluRay",
    );
    let hit = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &key)
        .await
        .unwrap();
    assert_eq!(hit.len(), 1, "bare candidate registered");
    assert_eq!(
        hit[0].title, "Mission - Impossible - Ghost Protocol",
        "internal dashes survive, but quality/codec/source tags are stripped"
    );
    assert_eq!(hit[0].year, None);
}

// ---- 4. Common torrent / scene filename patterns ------------------------

#[tokio::test]
async fn dot_separated_scene_release() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "Inception.2010.1080p.BluRay.x264-SPARKS.mkv",
        b"inception-scene",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 1);

    let lib_uuid = h.library_id.into_uuid();
    let candidates = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &bare_key(lib_uuid, "Inception.2010.1080p.BluRay.x264-SPARKS"),
        )
        .await
        .unwrap();

    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.title, "Inception");
    assert_eq!(candidate.year, Some(2010));
}

#[tokio::test]
async fn group_bracket_prefix_stripped() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "[Tigole] Inception (2010) 4K HDR Atmos.mkv",
        b"inception-tigole",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 1);

    let lib_uuid = h.library_id.into_uuid();
    let key = bare_key(lib_uuid, "[Tigole] Inception (2010) 4K HDR Atmos");
    let hit = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &key)
        .await
        .unwrap();
    assert_eq!(hit.len(), 1);
    assert_eq!(
        hit[0].title, "Inception",
        "release-group bracket prefix stripped; 4K HDR cut from suffix"
    );
    assert_eq!(hit[0].year, Some(2010));
}

#[tokio::test]
async fn movie_folder_with_torrent_junk_inside() {
    // Plex folder identity, torrent-style filename inside it. Folder gives
    // us the canonical title/year; filename junk is stripped without losing
    // identity.
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "Dune (2021)/Dune.2021.2160p.UHD.BluRay.x265.HDR.Atmos-FraMeSToR.mkv",
        b"dune",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 1);
    assert_eq!(summary.errored, 0);

    let lib_uuid = h.library_id.into_uuid();
    let hit = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &folder_key(lib_uuid, "Dune (2021)"))
        .await
        .unwrap();
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].title, "Dune");
    assert_eq!(hit[0].year, Some(2021));
}

// ---- 5. Mixed library: bare + folder + torrent all coexist -------------

#[tokio::test]
async fn three_distinct_candidates_for_same_title() {
    // Stress-test the "preserve candidates" rule: three filesystem
    // representations of (notionally) the same movie produce three separate
    // Works. Provider integration is the only thing that should merge them.
    let h = common::build(LibraryKind::Movies).await;

    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception (2010).mkv",
        b"folder",
    );
    common::touch(&h.lib_root, "Inception.mkv", b"bare-noyear");
    common::touch(
        &h.lib_root,
        "Inception.2010.1080p.BluRay.x264.mkv",
        b"bare-torrent",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 3);
    assert_eq!(summary.errored, 0);

    let lib_uuid = h.library_id.into_uuid();
    assert_eq!(
        h.storage
            .works_by_external_id(
                ExternalIdKind::Custom,
                &folder_key(lib_uuid, "Inception (2010)")
            )
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.storage
            .works_by_external_id(ExternalIdKind::Custom, &bare_key(lib_uuid, "Inception"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.storage
            .works_by_external_id(
                ExternalIdKind::Custom,
                &bare_key(lib_uuid, "Inception.2010.1080p.BluRay.x264")
            )
            .await
            .unwrap()
            .len(),
        1
    );
}

// ---- 6. External subtitle sidecars ---------------------------------------

#[tokio::test]
async fn external_sidecar_persists_as_subtitle_track() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception (2010).mkv",
        b"inception",
    );
    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception (2010).en.srt",
        b"1\n00:00:01,000 --> 00:00:02,000\nHello\n",
    );
    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception (2010).en.sdh.vtt",
        b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n[music]\n",
    );
    // A quality-flagged junk sidecar must be dropped, not turned into a
    // track with `None` language.
    common::touch(
        &h.lib_root,
        "Inception (2010)/Inception (2010).720p.srt",
        b"junk",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner.scan_library(h.library_id).await.expect("scan");

    let lib_uuid = h.library_id.into_uuid();
    let work = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(lib_uuid, "Inception (2010)"),
        )
        .await
        .unwrap();
    assert_eq!(work.len(), 1);
    let editions = h.storage.editions_for_work(work[0].id).await.unwrap();
    let mfs = h
        .storage
        .media_files_for_edition(editions[0].id)
        .await
        .unwrap();
    assert_eq!(mfs.len(), 1);
    let tracks = &mfs[0].subtitle_tracks;
    assert_eq!(
        tracks.len(),
        2,
        "two sidecars parsed (en.srt, en.sdh.vtt); 720p junk dropped"
    );

    // Sorted by absolute path for determinism — `.en.sdh.vtt` lexicographically
    // precedes `.en.srt` only because the SRT sidecar was created first.
    let codecs: Vec<&str> = tracks.iter().map(|t| t.codec.as_str()).collect();
    assert!(codecs.contains(&"sub_rip"));
    assert!(codecs.contains(&"web_vtt"));

    let srt = tracks.iter().find(|t| t.codec == "sub_rip").unwrap();
    assert_eq!(srt.language.as_deref(), Some("en"));
    assert!(srt.external);
    assert!(srt.srt_compatible);
    assert!(!srt.forced);
    assert!(!srt.default);
    assert_eq!(srt.codec_profile, None);
    assert_eq!(srt.image_count, None);
    assert_eq!(srt.title, None);
    assert!(srt.path.is_some(), "external sidecar path stored");

    let sdh = tracks.iter().find(|t| t.codec == "web_vtt").unwrap();
    assert_eq!(sdh.language.as_deref(), Some("en"));
    assert!(sdh.srt_compatible);
    assert_eq!(sdh.title.as_deref(), Some("SDH"));
    assert!(sdh.path.is_some());
}

// ---- domain events -------------------------------------------------------

#[tokio::test]
async fn scan_emits_domain_events() {
    use noflex_core::{DomainEvent, DomainEventBus};

    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"matrix-v1",
    );

    let bus = DomainEventBus::default();
    let mut rx = bus.subscribe();
    let scanner = FilesystemScanner::new(h.storage.clone()).with_events(bus);

    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 1);

    // Deterministic order: started → per-work creates → completed.
    match rx.recv().await.expect("scan-started") {
        DomainEvent::ScanStarted {
            library,
            library_name,
        } => {
            assert_eq!(library, h.library_id);
            assert_eq!(library_name, "Test Movies");
        }
        other => panic!("expected ScanStarted, got {other:?}"),
    }
    let matrix_work = match rx.recv().await.expect("work-added") {
        DomainEvent::WorkAdded {
            library,
            title,
            work,
        } => {
            assert_eq!(library, h.library_id);
            assert_eq!(title, "The Matrix");
            work
        }
        other => panic!("expected WorkAdded, got {other:?}"),
    };
    match rx.recv().await.expect("scan-completed") {
        DomainEvent::ScanCompleted {
            library,
            added,
            updated,
            removed,
            added_works,
            updated_works,
            removed_works,
            ..
        } => {
            assert_eq!(library, h.library_id);
            assert_eq!((added, updated, removed), (1, 0, 0));
            assert_eq!(added_works, vec![matrix_work]);
            assert!(updated_works.is_empty());
            assert!(removed_works.is_empty());
        }
        other => panic!("expected ScanCompleted, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "no further events on first scan");

    // An unchanged rescan emits the lifecycle pair but no WorkAdded.
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(summary.added, 0);
    assert!(matches!(
        rx.recv().await.expect("rescan started"),
        DomainEvent::ScanStarted { .. }
    ));
    match rx.recv().await.expect("rescan completed") {
        DomainEvent::ScanCompleted {
            added,
            updated,
            removed,
            added_works,
            updated_works,
            ..
        } => {
            assert_eq!((added, updated, removed), (0, 0, 0));
            assert!(added_works.is_empty());
            assert!(updated_works.is_empty());
        }
        other => panic!("expected ScanCompleted, got {other:?}"),
    }
    assert!(
        rx.try_recv().is_err(),
        "unchanged rescan must not re-add works"
    );

    // A changed file reports its target work as updated.
    common::touch(
        &h.lib_root,
        "The Matrix (1999)/The Matrix (1999).mkv",
        b"matrix-v2-with-different-length",
    );
    let summary = scanner
        .scan_library(h.library_id)
        .await
        .expect("update rescan");
    assert_eq!(summary.updated, 1);
    assert!(matches!(
        rx.recv().await.expect("update rescan started"),
        DomainEvent::ScanStarted { .. }
    ));
    match rx.recv().await.expect("update rescan completed") {
        DomainEvent::ScanCompleted {
            added,
            updated,
            added_works,
            updated_works,
            ..
        } => {
            assert_eq!((added, updated), (0, 1));
            assert!(added_works.is_empty());
            assert_eq!(updated_works, vec![matrix_work]);
        }
        other => panic!("expected ScanCompleted, got {other:?}"),
    }

    common::unlink(&h.lib_root, "The Matrix (1999)/The Matrix (1999).mkv");
    scanner
        .scan_library(h.library_id)
        .await
        .expect("remove scan");
    assert!(matches!(
        rx.recv().await.expect("remove scan started"),
        DomainEvent::ScanStarted { .. }
    ));
    match rx.recv().await.expect("remove scan completed") {
        DomainEvent::ScanCompleted {
            removed,
            removed_works,
            ..
        } => {
            assert_eq!(removed, 1);
            assert_eq!(removed_works, vec![matrix_work]);
        }
        other => panic!("expected ScanCompleted, got {other:?}"),
    }
}

// ---- Versions ------------------------------------------------------------

#[tokio::test]
async fn quality_versions_share_one_edition() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(
        &h.lib_root,
        "Zodiac (2007)/Zodiac (2007) - 1080p AV1 BluRay.mkv",
        b"zodiac-1080",
    );
    common::touch(
        &h.lib_root,
        "Zodiac (2007)/Zodiac (2007) - 2160p HEVC HDR.mkv",
        b"zodiac-2160",
    );
    common::touch(
        &h.lib_root,
        "Blade Runner 2049 (2017)/Blade Runner 2049 (2017).mkv",
        b"br2049",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!((summary.added, summary.errored), (3, 0));

    let lib_uuid = h.library_id.into_uuid();
    let zodiac = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(lib_uuid, "Zodiac (2007)"),
        )
        .await
        .unwrap();
    assert_eq!(zodiac.len(), 1);
    assert_eq!(
        (zodiac[0].title.as_str(), zodiac[0].year),
        ("Zodiac", Some(2007))
    );
    let editions = h.storage.editions_for_work(zodiac[0].id).await.unwrap();
    assert_eq!(editions.len(), 1, "quality tags are versions, not editions");
    let files = h
        .storage
        .media_files_for_edition(editions[0].id)
        .await
        .unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|m| m.part_index == 0));

    let br = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &folder_key(lib_uuid, "Blade Runner 2049 (2017)"),
        )
        .await
        .unwrap();
    assert_eq!(
        (br[0].title.as_str(), br[0].year),
        ("Blade Runner 2049", Some(2017))
    );
}
