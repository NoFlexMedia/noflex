// SPDX-License-Identifier: AGPL-3.0-only
//
// Shows-library integration tests.
//
// Seasons and episodes are keyed by show name and number, not by folder:
// `Season 1 (1080p)` / `Season 1 (4K)` folders for the same show produce ONE
// Season, and each episode's files become versions of ONE Episode.

use noflex_core::{ExternalId, ExternalIdKind, MetadataStatus, WorkKind};
use noflex_scanner::{FilesystemScanner, ScanSummary, slugify};
use noflex_storage::{
    domain::{LibraryKind, MediaFile, Work},
    query::ListWorksQuery,
};

mod common;

fn series_key(lib: uuid::Uuid, show: &str) -> String {
    format!("fs:series:{lib}:{}", slugify(show))
}

fn season_key(lib: uuid::Uuid, show: &str, num: u32) -> String {
    format!("fs:season:{lib}:{}:{num}", slugify(show))
}

fn episode_key(lib: uuid::Uuid, show: &str, s: u32, e: u32) -> String {
    format!("fs:episode:{lib}:{}:{s}:{e}", slugify(show))
}

/// Range form, for multi-episode files like `s01e01-e02`.
fn episode_range_key(lib: uuid::Uuid, show: &str, s: u32, e_start: u32, e_end: u32) -> String {
    format!("fs:episode:{lib}:{}:{s}:{e_start}-{e_end}", slugify(show))
}

// ---- 1. Plex-strict show layout, multi-season, multi-episode -----------

#[tokio::test]
async fn plex_strict_layout_multi_season() {
    let h = common::build(LibraryKind::Shows).await;

    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e01 - Pilot.mkv",
        b"bb-101",
    );
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e02 - Cat in the Bag.mkv",
        b"bb-102",
    );
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 02/Breaking Bad - s02e01 - Seven Thirty-Seven.mkv",
        b"bb-201",
    );
    // Multi-episode + show without year.
    common::touch(
        &h.lib_root,
        "Severance/Season 01/Severance - s01e01-e02 - Good News.mkv",
        b"sev-101-102",
    );
    common::touch(&h.lib_root, "Severance/Season 01/poster.png", b"png");

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(
        summary,
        ScanSummary {
            added: 4,
            updated: 0,
            removed: 0,
            errored: 0,
        }
    );

    let lib_uuid = h.library_id.into_uuid();

    // Breaking Bad: one Series, two Seasons, three Episodes.
    let bb = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &series_key(lib_uuid, "Breaking Bad"),
        )
        .await
        .unwrap();
    assert_eq!(bb.len(), 1);
    assert_eq!(bb[0].kind, WorkKind::Series);
    assert_eq!(bb[0].year, Some(2008));

    for season_num in [1u32, 2u32] {
        let key = season_key(lib_uuid, "Breaking Bad", season_num);
        let s = h
            .storage
            .works_by_external_id(ExternalIdKind::Custom, &key)
            .await
            .unwrap();
        assert_eq!(s.len(), 1, "season {season_num}");
        assert_eq!(s[0].parent_id, Some(bb[0].id));
        assert_eq!(s[0].index_number, Some(season_num as i32));
    }

    for (s, e) in [(1u32, 1u32), (1, 2), (2, 1)] {
        let key = episode_key(lib_uuid, "Breaking Bad", s, e);
        let ep = h
            .storage
            .works_by_external_id(ExternalIdKind::Custom, &key)
            .await
            .unwrap();
        assert_eq!(ep.len(), 1, "s{s:02}e{e:02}");
        assert_eq!(ep[0].kind, WorkKind::Episode);
        assert_eq!(ep[0].index_number, Some(e as i32));
        assert_eq!(ep[0].parent_index, Some(s as i32));
    }

    // Severance: show without year, multi-episode is one Episode candidate.
    let sev = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &series_key(lib_uuid, "Severance"))
        .await
        .unwrap();
    assert_eq!(sev.len(), 1);
    assert_eq!(sev[0].year, None);
}

#[tokio::test]
async fn plex_strict_idempotent_and_remove() {
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e01 - Pilot.mkv",
        b"bb-101",
    );
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e02 - Cat in the Bag.mkv",
        b"bb-102",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner.scan_library(h.library_id).await.expect("first");

    // Idempotent.
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(summary, ScanSummary::default());

    // Remove one episode.
    common::unlink(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e02 - Cat in the Bag.mkv",
    );
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(
        summary,
        ScanSummary {
            removed: 1,
            ..Default::default()
        }
    );
}

// ---- 2. Specials folder (season 0) -------------------------------------

#[tokio::test]
async fn specials_folder_treated_as_season_zero() {
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Show (2020)/Season 01/Show - s01e01 - Pilot.mkv",
        b"e1",
    );
    common::touch(
        &h.lib_root,
        "Show (2020)/Specials/Show - s00e01 - Behind the Scenes.mkv",
        b"sp1",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 2);
    assert_eq!(summary.errored, 0);

    let lib_uuid = h.library_id.into_uuid();
    let special = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &season_key(lib_uuid, "Show", 0))
        .await
        .unwrap();
    assert_eq!(special.len(), 1, "Specials registered as season 0");
    assert_eq!(special[0].index_number, Some(0));
}

// ---- 3. Torrent-flat layout (combined season folder) -------------------

#[tokio::test]
async fn torrent_flat_show_layout() {
    // Single top-level dir contains both show name and season number,
    // with episode files directly inside (no Season subdirectory).
    let h = common::build(LibraryKind::Shows).await;
    let folder = "Game of Thrones Season 1 (1080p BluRay x265 HEVC 10bit AAC 5 1 Tigole)";
    common::touch(
        &h.lib_root,
        &format!("{folder}/Game of Thrones - S01E01 - Winter Is Coming.mkv"),
        b"got-101",
    );
    common::touch(
        &h.lib_root,
        &format!("{folder}/Game of Thrones - S01E02 - The Kingsroad.mkv"),
        b"got-102",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(
        summary,
        ScanSummary {
            added: 2,
            ..Default::default()
        }
    );

    let lib_uuid = h.library_id.into_uuid();

    // Show name was extracted from the combined folder name (with the
    // " Season 1 (junk)" suffix stripped).
    let series = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &series_key(lib_uuid, "Game of Thrones"),
        )
        .await
        .unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].title, "Game of Thrones");

    // Season identified by the torrent folder path.
    let season = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &season_key(lib_uuid, "Game of Thrones", 1),
        )
        .await
        .unwrap();
    assert_eq!(season.len(), 1);
    assert_eq!(season[0].parent_id, Some(series[0].id));

    // Two episodes, both children of that season.
    for ep_num in [1u32, 2u32] {
        let key = episode_key(lib_uuid, "Game of Thrones", 1, ep_num);
        let ep = h
            .storage
            .works_by_external_id(ExternalIdKind::Custom, &key)
            .await
            .unwrap();
        assert_eq!(ep.len(), 1);
        assert_eq!(ep[0].parent_id, Some(season[0].id));
    }
}

// ---- 4. Multiple versions of the same season ---------------------------

/// The one work under `key`, with the media files of its single edition.
async fn versions(h: &common::Harness, key: &str) -> (Work, Vec<MediaFile>) {
    let mut works = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, key)
        .await
        .unwrap();
    assert_eq!(works.len(), 1, "{key}");
    let work = works.remove(0);
    let editions = h.storage.editions_for_work(work.id).await.unwrap();
    assert_eq!(editions.len(), 1, "versions share one edition: {key}");
    assert_eq!(editions[0].name, "Default");
    let files = h
        .storage
        .media_files_for_edition(editions[0].id)
        .await
        .unwrap();
    (work, files)
}

async fn child_ids(h: &common::Harness, parent: &Work) -> Vec<uuid::Uuid> {
    let page = h
        .storage
        .list_works(&ListWorksQuery {
            parent: Some(parent.id),
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    let mut ids: Vec<_> = page.items.iter().map(|w| w.id.into_uuid()).collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn quality_split_season_folders_merge() {
    // `Season 01 (1080p)` and `Season 01 (4K)` hold versions of the same
    // episodes: one Season, one Episode per number, a file per version.
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Game of Thrones (2011)/Season 01 (1080p)/Game of Thrones - s01e01 - Winter Is Coming.mkv",
        b"got-101-1080p",
    );
    common::touch(
        &h.lib_root,
        "Game of Thrones (2011)/Season 01 (4K)/Game of Thrones - s01e01 - Winter Is Coming.mkv",
        b"got-101-4k",
    );

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!((summary.added, summary.errored), (2, 0));

    let lib_uuid = h.library_id.into_uuid();
    let series = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &series_key(lib_uuid, "Game of Thrones"),
        )
        .await
        .unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].year, Some(2011));

    let season = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &season_key(lib_uuid, "Game of Thrones", 1),
        )
        .await
        .unwrap();
    assert_eq!(season.len(), 1, "one Season for both folders");
    assert_eq!(season[0].parent_id, Some(series[0].id));
    assert_eq!(season[0].index_number, Some(1));
    assert_eq!(child_ids(&h, &series[0]).await.len(), 1);

    let (episode, files) = versions(&h, &episode_key(lib_uuid, "Game of Thrones", 1, 1)).await;
    assert_eq!(episode.parent_id, Some(season[0].id));
    assert_eq!(episode.title, "Winter Is Coming");
    assert_eq!(files.len(), 2);
    assert_eq!(child_ids(&h, &season[0]).await.len(), 1);

    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(summary, ScanSummary::default());
}

#[tokio::test]
async fn mixed_season_layouts_converge() {
    // Some episodes exist in both a plain and a quality-tagged folder,
    // others in only one of them.
    let h = common::build(LibraryKind::Shows).await;
    for (rel, body) in [
        ("Show/Season 01/Show - S01E01 - 1080p.mkv", &b"e1-1080"[..]),
        ("Show/Season 01 (4K)/Show - S01E01 - 2160p.mkv", b"e1-2160"),
        ("Show/Season 01/Show - S01E02.mkv", b"e2"),
        (
            "Show/Season 01 (4K)/Show - S01E03 - Finale - 2160p.mkv",
            b"e3",
        ),
        ("Show/Season 02/Show - S02E01.mkv", b"s2e1"),
    ] {
        common::touch(&h.lib_root, rel, body);
    }

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!((summary.added, summary.errored), (5, 0));

    let lib_uuid = h.library_id.into_uuid();
    let series = &h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &series_key(lib_uuid, "Show"))
        .await
        .unwrap()[0];
    assert_eq!(child_ids(&h, series).await.len(), 2, "seasons 1 and 2");

    let s1 = &h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &season_key(lib_uuid, "Show", 1))
        .await
        .unwrap()[0];
    assert_eq!(child_ids(&h, s1).await.len(), 3);
    for (e, title, count) in [(1, "Episode 1", 2), (2, "Episode 2", 1), (3, "Finale", 1)] {
        let (episode, files) = versions(&h, &episode_key(lib_uuid, "Show", 1, e)).await;
        assert_eq!(episode.parent_id, Some(s1.id));
        assert_eq!((episode.title.as_str(), files.len()), (title, count));
    }
}

#[tokio::test]
async fn multi_episode_versions_stay_apart_from_single_episodes() {
    let h = common::build(LibraryKind::Shows).await;
    for (rel, body) in [
        (
            "Show/Season 01 (1080p)/Show - S01E01-E02.mkv",
            &b"r-1080"[..],
        ),
        ("Show/Season 01 (4K)/Show - S01E01-E02.mkv", b"r-4k"),
        ("Show/Season 01 (4K)/Show - S01E01.mkv", b"e1-4k"),
    ] {
        common::touch(&h.lib_root, rel, body);
    }
    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!((summary.added, summary.errored), (3, 0));

    let lib_uuid = h.library_id.into_uuid();
    let (range, range_files) = versions(&h, &episode_range_key(lib_uuid, "Show", 1, 1, 2)).await;
    let (single, single_files) = versions(&h, &episode_key(lib_uuid, "Show", 1, 1)).await;
    assert_ne!(range.id, single.id);
    assert_eq!((range.index_end, range_files.len()), (Some(2), 2));
    assert_eq!((single.index_end, single_files.len()), (None, 1));
    assert_eq!(range.parent_id, single.parent_id, "one Season");
}

#[tokio::test]
async fn library_scanned_under_folder_keys_converges() {
    // Before scan version 1, season and episode keys carried the season
    // folder. Rebuild that state: scan the 1080p files, then re-key their
    // works the old way and forget the recorded scan version.
    let h = common::build(LibraryKind::Shows).await;
    let lib_uuid = h.library_id.into_uuid();
    common::touch(
        &h.lib_root,
        "Show/Season 01 (1080p)/Show - S01E01.mkv",
        b"e1-1080",
    );
    common::touch(
        &h.lib_root,
        "Show/Season 01 (1080p)/Show - S01E02.mkv",
        b"e2-1080",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner
        .scan_library(h.library_id)
        .await
        .expect("first scan");

    let folder = slugify("Show/Season 01 (1080p)");
    let mut legacy = Vec::new();
    for (key, old) in [
        (
            season_key(lib_uuid, "Show", 1),
            format!("{}:{folder}", season_key(lib_uuid, "Show", 1)),
        ),
        (
            episode_key(lib_uuid, "Show", 1, 1),
            format!("{}:{folder}", episode_key(lib_uuid, "Show", 1, 1)),
        ),
        (
            episode_key(lib_uuid, "Show", 1, 2),
            format!("{}:{folder}", episode_key(lib_uuid, "Show", 1, 2)),
        ),
    ] {
        let mut work = h
            .storage
            .works_by_external_id(ExternalIdKind::Custom, &key)
            .await
            .unwrap()
            .remove(0);
        work.external_ids = vec![ExternalId {
            kind: ExternalIdKind::Custom,
            value: old,
        }];
        h.storage.upsert_work(&work).await.expect("re-key");
        legacy.push(work.id);
    }
    h.storage
        .set_library_scan_version(h.library_id, 0)
        .await
        .expect("reset scan version");
    common::touch(
        &h.lib_root,
        "Show/Season 01 (4K)/Show - S01E01.mkv",
        b"e1-4k",
    );

    let summary = scanner.scan_library(h.library_id).await.expect("re-plan");
    assert_eq!(
        summary,
        ScanSummary {
            added: 1,
            updated: 2,
            ..Default::default()
        },
        "the unchanged 1080p files moved, the 4K file joined them"
    );
    for id in legacy {
        assert!(
            h.storage.get_work(id).await.unwrap().is_none(),
            "folder-keyed work pruned"
        );
    }
    let series = &h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &series_key(lib_uuid, "Show"))
        .await
        .unwrap()[0];
    assert_eq!(child_ids(&h, series).await.len(), 1);
    let (_, e1) = versions(&h, &episode_key(lib_uuid, "Show", 1, 1)).await;
    let (_, e2) = versions(&h, &episode_key(lib_uuid, "Show", 1, 2)).await;
    assert_eq!((e1.len(), e2.len()), (2, 1));

    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(summary, ScanSummary::default(), "re-plan runs once");
}

// ---- 5. Multi-episode files (index_end is first-class) -----------------

#[tokio::test]
async fn multi_episode_file_records_index_end() {
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Severance/Season 01/Severance - s01e01-e02 - Good News.mkv",
        b"sev-101-102",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 1);
    assert_eq!(summary.errored, 0);

    let lib_uuid = h.library_id.into_uuid();

    // The Episode Work is found under the range scan-key, not the single-ep
    // key. The single-ep key returns no hits — there is no s01e01-only work.
    let single_e1 = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &episode_key(lib_uuid, "Severance", 1, 1),
        )
        .await
        .unwrap();
    assert!(
        single_e1.is_empty(),
        "no single-episode candidate for e01 — this file represents the range"
    );

    let range = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &episode_range_key(lib_uuid, "Severance", 1, 1, 2),
        )
        .await
        .unwrap();
    assert_eq!(range.len(), 1, "range candidate present");
    let ep = &range[0];
    assert_eq!(ep.kind, WorkKind::Episode);
    assert_eq!(ep.index_number, Some(1), "range starts at episode 1");
    assert_eq!(ep.index_end, Some(2), "range ends at episode 2");
    assert_eq!(ep.parent_index, Some(1), "season number on the episode");
}

#[tokio::test]
async fn single_episode_has_no_index_end() {
    // Sanity check: index_end stays None for normal single-episode files,
    // so single-ep scan-keys and the lookups built on them keep working.
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Severance/Season 01/Severance - s01e01 - Good News.mkv",
        b"sev-101",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner.scan_library(h.library_id).await.expect("scan");

    let lib_uuid = h.library_id.into_uuid();
    let hits = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &episode_key(lib_uuid, "Severance", 1, 1),
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].index_number, Some(1));
    assert_eq!(hits[0].index_end, None);
}

#[tokio::test]
async fn single_and_multi_episode_files_stay_distinct() {
    // Two files in the same season folder: one represents s01e01 only,
    // the other represents s01e01-e02. They are SEPARATE candidates — the
    // merge belongs to the metadata provider, not the scanner.
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Severance/Season 01/Severance - s01e01 - Good News.mkv",
        b"sev-just-101",
    );
    common::touch(
        &h.lib_root,
        "Severance/Season 01/Severance - s01e01-e02 - Good News.mkv",
        b"sev-101-and-102",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 2);
    assert_eq!(summary.errored, 0);

    let lib_uuid = h.library_id.into_uuid();

    let single = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &episode_key(lib_uuid, "Severance", 1, 1),
        )
        .await
        .unwrap();
    let range = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &episode_range_key(lib_uuid, "Severance", 1, 1, 2),
        )
        .await
        .unwrap();
    assert_eq!(single.len(), 1);
    assert_eq!(range.len(), 1);
    assert_ne!(single[0].id, range[0].id);
    assert_eq!(single[0].index_end, None);
    assert_eq!(range[0].index_end, Some(2));
}

#[tokio::test]
async fn multi_episode_rescan_is_idempotent() {
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Severance/Season 01/Severance - s01e01-e02 - Good News.mkv",
        b"sev-range",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner.scan_library(h.library_id).await.expect("first");
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(summary, ScanSummary::default());
}

#[tokio::test]
async fn three_episode_range() {
    // sNNeNNeNN with three episodes covered. The end of the range is the
    // last episode in the run, regardless of how many segments it has.
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Show/Season 01/Show - s01e05-e07 - Three In One.mkv",
        b"triple",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner.scan_library(h.library_id).await.expect("scan");

    let lib_uuid = h.library_id.into_uuid();
    let hits = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &episode_range_key(lib_uuid, "Show", 1, 5, 7),
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].index_number, Some(5));
    assert_eq!(hits[0].index_end, Some(7));
}

// ---- 6. Show without a year folder -------------------------------------

#[tokio::test]
async fn show_folder_without_year() {
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Severance/Season 01/Severance - s01e01 - Good News.mkv",
        b"sev-101",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner.scan_library(h.library_id).await.expect("scan");

    let lib_uuid = h.library_id.into_uuid();
    let series = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &series_key(lib_uuid, "Severance"))
        .await
        .unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].year, None);
}

// ---- 7. Metadata-status: scanner output is always Unconfirmed -----------

#[tokio::test]
async fn scanner_creates_unconfirmed_works_at_every_level() {
    // Series, Season, and Episode Works all carry filesystem-derived data,
    // so all three must land at Unconfirmed for the frontend to badge them
    // correctly. Providers escalate later.
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e01 - Pilot.mkv",
        b"bb-101",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner.scan_library(h.library_id).await.expect("scan");

    let lib_uuid = h.library_id.into_uuid();
    let series = &h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &series_key(lib_uuid, "Breaking Bad"),
        )
        .await
        .unwrap()[0];
    let season = &h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &season_key(lib_uuid, "Breaking Bad", 1),
        )
        .await
        .unwrap()[0];
    let episode = &h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &episode_key(lib_uuid, "Breaking Bad", 1, 1),
        )
        .await
        .unwrap()[0];

    assert_eq!(series.metadata_status, MetadataStatus::Unconfirmed);
    assert_eq!(season.metadata_status, MetadataStatus::Unconfirmed);
    assert_eq!(episode.metadata_status, MetadataStatus::Unconfirmed);
}

#[tokio::test]
async fn rescan_does_not_downgrade_confirmed_or_user_edited() {
    // The scanner deliberately must NOT touch a Work that already exists,
    // so a provider's `Confirmed` (or a user's `UserEdited`) stays put on a
    // rescan. Without this, every cron rescan would silently revert
    // verified metadata back to filesystem heuristics.
    let h = common::build(LibraryKind::Shows).await;
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e01 - Pilot.mkv",
        b"bb-101",
    );
    let scanner = FilesystemScanner::new(h.storage.clone());
    scanner
        .scan_library(h.library_id)
        .await
        .expect("first scan");

    let lib_uuid = h.library_id.into_uuid();
    let series_id = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &series_key(lib_uuid, "Breaking Bad"),
        )
        .await
        .unwrap()[0]
        .id;

    // Simulate a provider escalating the Series, plus rewriting its title.
    let mut series = h.storage.get_work(series_id).await.unwrap().unwrap();
    series.metadata_status = MetadataStatus::Confirmed;
    series.title = "Breaking Bad (Provider-Verified)".to_string();
    h.storage.upsert_work(&series).await.expect("escalate");

    // Rescan: same files on disk, scanner should find the existing Work via
    // its scan key and leave it alone.
    let summary = scanner.scan_library(h.library_id).await.expect("rescan");
    assert_eq!(summary, ScanSummary::default(), "rescan is a no-op");

    let after = h.storage.get_work(series_id).await.unwrap().unwrap();
    assert_eq!(
        after.metadata_status,
        MetadataStatus::Confirmed,
        "scanner did NOT downgrade the Confirmed Work"
    );
    assert_eq!(
        after.title, "Breaking Bad (Provider-Verified)",
        "scanner also did NOT overwrite the provider's title"
    );

    // Same protection for UserEdited.
    let mut series = after;
    series.metadata_status = MetadataStatus::UserEdited;
    series.title = "Breaking Bad (My Edit)".to_string();
    h.storage.upsert_work(&series).await.expect("user edit");

    scanner.scan_library(h.library_id).await.expect("rescan 2");
    let after = h.storage.get_work(series_id).await.unwrap().unwrap();
    assert_eq!(after.metadata_status, MetadataStatus::UserEdited);
    assert_eq!(after.title, "Breaking Bad (My Edit)");
}

#[tokio::test]
async fn scan_completed_added_works_dedupe_across_files() {
    use noflex_core::{DomainEvent, DomainEventBus};

    let h = common::build(LibraryKind::Shows).await;
    // Two files of the same new show: the series and season works are
    // created once, so the scan's added-work list must carry each work
    // exactly once — series, season, and both episodes, four total.
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e01 - Pilot.mkv",
        b"bb-101",
    );
    common::touch(
        &h.lib_root,
        "Breaking Bad (2008)/Season 01/Breaking Bad - s01e02 - Cat in the Bag.mkv",
        b"bb-102",
    );

    let bus = DomainEventBus::default();
    let mut rx = bus.subscribe();
    let scanner = FilesystemScanner::new(h.storage.clone()).with_events(bus);
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 2);

    loop {
        match rx.recv().await.expect("scan-completed") {
            DomainEvent::ScanCompleted {
                added_works,
                updated_works,
                ..
            } => {
                assert_eq!(added_works.len(), 4, "{added_works:?}");
                assert!(updated_works.is_empty());
                break;
            }
            DomainEvent::ScanStarted { .. } | DomainEvent::WorkAdded { .. } => {}
            other => panic!("unexpected event: {other:?}"),
        }
    }
}

#[tokio::test]
async fn quality_versions_share_one_episode() {
    let h = common::build(LibraryKind::Shows).await;
    for (rel, body) in [
        ("Show/Season 01/Show - S01E01 - 1080p.mkv", &b"e1-1080"[..]),
        (
            "Show/Season 01/Show - S01E01 - 2160p HEVC HDR.mkv",
            b"e1-2160",
        ),
        (
            "Show/Season 01/Show - S01E02 - Pilot - 1080p AV1.mkv",
            b"e2-1080",
        ),
        ("Show/Season 01/Show - S01E02 - Pilot - 720p.mkv", b"e2-720"),
        (
            "Show/Season 01/Show.S01E03.1080p.WEB-DL.x264.mkv",
            b"e3-1080",
        ),
        ("Show/Season 01/Show.S01E03.720p.HDTV.x264.mkv", b"e3-720"),
    ] {
        common::touch(&h.lib_root, rel, body);
    }

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!((summary.added, summary.errored), (6, 0));

    let lib_uuid = h.library_id.into_uuid();
    for (e, title) in [(1, "Episode 1"), (2, "Pilot"), (3, "Episode 3")] {
        let ep = h
            .storage
            .works_by_external_id(ExternalIdKind::Custom, &episode_key(lib_uuid, "Show", 1, e))
            .await
            .unwrap();
        assert_eq!(ep.len(), 1, "s01e0{e}");
        assert_eq!(ep[0].title, title);
        let editions = h.storage.editions_for_work(ep[0].id).await.unwrap();
        assert_eq!(editions.len(), 1, "quality tags are versions, not editions");
        assert_eq!(editions[0].name, "Default");
        let files = h
            .storage
            .media_files_for_edition(editions[0].id)
            .await
            .unwrap();
        assert_eq!(files.len(), 2, "s01e0{e}");
        assert!(files.iter().all(|m| m.part_index == 0));
    }
}
