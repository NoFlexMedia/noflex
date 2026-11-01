// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{ExternalIdKind, WorkKind};
use noflex_scanner::{FilesystemScanner, ScanSummary, slugify};
use noflex_storage::domain::LibraryKind;

mod common;

fn key(kind: &str, library: uuid::Uuid, relative: &str) -> String {
    format!("fs:{kind}:{library}:{}", slugify(relative))
}

#[tokio::test]
async fn music_builds_artist_album_track_hierarchy() {
    let h = common::build(LibraryKind::Music).await;
    common::touch(
        &h.lib_root,
        "Miles Davis/Kind of Blue/01 - So What.flac",
        b"audio-one",
    );
    common::touch(
        &h.lib_root,
        "Miles Davis/Kind of Blue/02 - Freddie Freeloader.MP3",
        b"audio-two",
    );
    common::touch(&h.lib_root, "Miles Davis/Kind of Blue/cover.jpg", b"cover");

    let scanner = FilesystemScanner::new(h.storage.clone());
    assert_eq!(
        scanner.scan_library(h.library_id).await.expect("scan"),
        ScanSummary {
            added: 2,
            updated: 0,
            removed: 0,
            errored: 0,
        }
    );

    let library = h.library_id.into_uuid();
    let artist = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &key("artist", library, "Miles Davis"),
        )
        .await
        .unwrap();
    assert_eq!(artist.len(), 1);
    assert_eq!(artist[0].kind, WorkKind::Artist);

    let album = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &key("album", library, "Miles Davis/Kind of Blue"),
        )
        .await
        .unwrap();
    assert_eq!(album.len(), 1);
    assert_eq!(album[0].kind, WorkKind::Album);
    assert_eq!(album[0].parent_id, Some(artist[0].id));

    let track = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &key("track", library, "Miles Davis/Kind of Blue/01 - So What"),
        )
        .await
        .unwrap();
    assert_eq!(track.len(), 1);
    assert_eq!(track[0].kind, WorkKind::Track);
    assert_eq!(track[0].parent_id, Some(album[0].id));
    assert_eq!(track[0].index_number, Some(1));
    let editions = h.storage.editions_for_work(track[0].id).await.unwrap();
    assert_eq!(editions.len(), 1);
    assert_eq!(editions[0].name, "Default");
    assert!(
        h.storage
            .editions_for_work(album[0].id)
            .await
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        scanner.scan_library(h.library_id).await.expect("rescan"),
        ScanSummary::default()
    );
    common::unlink(
        &h.lib_root,
        "Miles Davis/Kind of Blue/02 - Freddie Freeloader.MP3",
    );
    assert_eq!(
        scanner.scan_library(h.library_id).await.expect("prune"),
        ScanSummary {
            removed: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn photos_create_one_playable_work_per_supported_file() {
    let h = common::build(LibraryKind::Photos).await;
    common::touch(
        &h.lib_root,
        "Trips/2024-06-12_Beach_Sunset.HEIC",
        b"photo-one",
    );
    common::touch(&h.lib_root, "Trips/Portrait.jpeg", b"photo-two");
    common::touch(&h.lib_root, "Trips/movie.mp4", b"decoy");

    let scanner = FilesystemScanner::new(h.storage.clone());
    let summary = scanner.scan_library(h.library_id).await.expect("scan");
    assert_eq!(summary.added, 2);
    assert_eq!(summary.errored, 0);

    let photo = h
        .storage
        .works_by_external_id(
            ExternalIdKind::Custom,
            &key(
                "photo",
                h.library_id.into_uuid(),
                "Trips/2024-06-12_Beach_Sunset",
            ),
        )
        .await
        .unwrap();
    assert_eq!(photo.len(), 1);
    assert_eq!(photo[0].kind, WorkKind::Photo);
    assert_eq!(photo[0].title, "2024-06-12 Beach Sunset");
    assert_eq!(photo[0].year, Some(2024));
    assert_eq!(photo[0].release_date.as_deref(), Some("2024-06-12"));
    let editions = h.storage.editions_for_work(photo[0].id).await.unwrap();
    assert_eq!(editions.len(), 1);
    let files = h
        .storage
        .media_files_for_edition(editions[0].id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].container.as_deref(), Some("heic"));
}
