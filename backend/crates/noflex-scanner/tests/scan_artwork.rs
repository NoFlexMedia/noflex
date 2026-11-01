// SPDX-License-Identifier: AGPL-3.0-only
//
// Local artwork discovery: Plex-convention image files next to media
// become `images` rows on the works they decorate, and follow the disk
// across rescans even when the media files themselves are unchanged.

use std::collections::BTreeMap;

use noflex_core::{ExternalIdKind, ImageKind};
use noflex_scanner::{FilesystemScanner, slugify};
use noflex_storage::domain::{Image, ImageSource, ImageTarget, LibraryKind};

mod common;

use common::Harness;

fn key(kind: &str, h: &Harness, rest: &str) -> String {
    format!("fs:{kind}:{}:{rest}", h.library_id.into_uuid())
}

/// `kind → path relative to the library root` for the work under `key`.
async fn art(h: &Harness, key: &str) -> BTreeMap<&'static str, String> {
    let works = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, key)
        .await
        .expect("lookup");
    assert_eq!(works.len(), 1, "exactly one work for {key}");
    h.storage
        .list_images(ImageTarget::Work(works[0].id), ImageSource::ALL)
        .await
        .expect("list images")
        .into_iter()
        .map(|i| {
            let rel = std::path::Path::new(&i.path)
                .strip_prefix(&h.lib_root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(i.path.clone());
            (i.kind.as_str(), rel)
        })
        .collect()
}

async fn scan(h: &Harness) {
    let summary = FilesystemScanner::new(h.storage.clone())
        .scan_library(h.library_id)
        .await
        .expect("scan");
    assert_eq!(summary.errored, 0);
}

fn expect(pairs: &[(&'static str, &str)]) -> BTreeMap<&'static str, String> {
    pairs.iter().map(|(k, v)| (*k, (*v).to_string())).collect()
}

#[tokio::test]
async fn movie_folder_and_bare_movie_artwork() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(&h.lib_root, "The Matrix (1999)/The Matrix (1999).mkv", b"m");
    common::touch(&h.lib_root, "The Matrix (1999)/poster.jpg", b"p");
    common::touch(&h.lib_root, "The Matrix (1999)/folder.png", b"f");
    common::touch(&h.lib_root, "The Matrix (1999)/fanart.jpg", b"b");
    common::touch(&h.lib_root, "The Matrix (1999)/clearlogo.png", b"l");
    common::touch(&h.lib_root, "Heat (1995).mkv", b"h");
    common::touch(&h.lib_root, "Heat (1995)-poster.jpg", b"hp");
    // Root-level generic names belong to no single bare movie.
    common::touch(&h.lib_root, "poster.jpg", b"stray");
    common::touch(&h.lib_root, "fanart.jpg", b"stray");

    scan(&h).await;

    let matrix = key(
        "movie",
        &h,
        &format!("folder:{}", slugify("The Matrix (1999)")),
    );
    assert_eq!(
        art(&h, &matrix).await,
        expect(&[
            ("primary", "The Matrix (1999)/poster.jpg"),
            ("backdrop", "The Matrix (1999)/fanart.jpg"),
            ("logo", "The Matrix (1999)/clearlogo.png"),
        ])
    );
    let heat = key("movie", &h, &format!("bare:{}", slugify("Heat (1995)")));
    assert_eq!(
        art(&h, &heat).await,
        expect(&[("primary", "Heat (1995)-poster.jpg")])
    );
}

#[tokio::test]
async fn show_season_and_episode_artwork() {
    let h = common::build(LibraryKind::Shows).await;
    let show = "Breaking Bad (2008)";
    common::touch(
        &h.lib_root,
        &format!("{show}/Season 01/Breaking Bad - s01e01 - Pilot.mkv"),
        b"e1",
    );
    common::touch(
        &h.lib_root,
        &format!("{show}/Season 01/Breaking Bad - s01e01 - Pilot-thumb.jpg"),
        b"t1",
    );
    common::touch(
        &h.lib_root,
        &format!("{show}/Season 01/Breaking Bad - s01e02.mkv"),
        b"e2",
    );
    common::touch(
        &h.lib_root,
        &format!("{show}/Season 02/Breaking Bad - s02e01.mkv"),
        b"e3",
    );
    common::touch(&h.lib_root, &format!("{show}/poster.jpg"), b"sp");
    common::touch(&h.lib_root, &format!("{show}/fanart.jpg"), b"sb");
    common::touch(&h.lib_root, &format!("{show}/banner.jpg"), b"sbn");
    common::touch(&h.lib_root, &format!("{show}/season01-poster.jpg"), b"s1");
    // Season-numbered show-folder art beats the season folder's own poster.
    common::touch(&h.lib_root, &format!("{show}/Season 01/poster.jpg"), b"s1f");
    common::touch(&h.lib_root, &format!("{show}/Season 02/folder.png"), b"s2");
    // A second Season 02 folder feeds the same Season; the first folder
    // by path supplies its in-folder art.
    common::touch(
        &h.lib_root,
        &format!("{show}/Season 02 (4K)/Breaking Bad - s02e01.mkv"),
        b"e3-4k",
    );
    common::touch(
        &h.lib_root,
        &format!("{show}/Season 02 (4K)/poster.jpg"),
        b"s2k",
    );
    common::touch(&h.lib_root, &format!("{show}/season02-fanart.jpg"), b"s2b");

    scan(&h).await;

    let series = key("series", &h, &slugify("Breaking Bad"));
    assert_eq!(
        art(&h, &series).await,
        expect(&[
            ("primary", &format!("{show}/poster.jpg")),
            ("backdrop", &format!("{show}/fanart.jpg")),
            ("banner", &format!("{show}/banner.jpg")),
        ])
    );
    let season = |n: u32| key("season", &h, &format!("{}:{n}", slugify("Breaking Bad")));
    assert_eq!(
        art(&h, &season(1)).await,
        expect(&[("primary", &format!("{show}/season01-poster.jpg"))])
    );
    assert_eq!(
        art(&h, &season(2)).await,
        expect(&[
            ("primary", &format!("{show}/Season 02/folder.png")),
            ("backdrop", &format!("{show}/season02-fanart.jpg")),
        ])
    );
    let episode = |e: u32| key("episode", &h, &format!("{}:1:{e}", slugify("Breaking Bad")));
    assert_eq!(
        art(&h, &episode(1)).await,
        expect(&[(
            "thumb",
            &format!("{show}/Season 01/Breaking Bad - s01e01 - Pilot-thumb.jpg")
        )])
    );
    assert!(art(&h, &episode(2)).await.is_empty());
}

#[tokio::test]
async fn album_and_artist_artwork() {
    let h = common::build(LibraryKind::Music).await;
    common::touch(
        &h.lib_root,
        "Miles Davis/Kind of Blue/01 - So What.flac",
        b"t",
    );
    common::touch(&h.lib_root, "Miles Davis/Kind of Blue/front.jpg", b"f");
    common::touch(&h.lib_root, "Miles Davis/Kind of Blue/cover.jpg", b"c");
    common::touch(&h.lib_root, "Miles Davis/artist.jpg", b"a");
    common::touch(&h.lib_root, "Miles Davis/fanart.jpg", b"b");

    scan(&h).await;

    assert_eq!(
        art(&h, &key("album", &h, &slugify("Miles Davis/Kind of Blue"))).await,
        expect(&[("primary", "Miles Davis/Kind of Blue/cover.jpg")])
    );
    assert_eq!(
        art(&h, &key("artist", &h, &slugify("Miles Davis"))).await,
        expect(&[
            ("primary", "Miles Davis/artist.jpg"),
            ("backdrop", "Miles Davis/fanart.jpg"),
        ])
    );
}

#[tokio::test]
async fn rescan_tracks_added_and_removed_artwork_for_unchanged_media() {
    let h = common::build(LibraryKind::Movies).await;
    common::touch(&h.lib_root, "Heat (1995)/Heat (1995).mkv", b"h");
    let heat = key("movie", &h, &format!("folder:{}", slugify("Heat (1995)")));

    scan(&h).await;
    assert!(art(&h, &heat).await.is_empty());

    // Provider rows (art in the image cache) coexist with local ones of
    // the same kind and are never touched by reconcile.
    let work = h
        .storage
        .works_by_external_id(ExternalIdKind::Custom, &heat)
        .await
        .expect("lookup")
        .remove(0);
    for (kind, path) in [
        (
            ImageKind::Primary,
            "/var/cache/noflex/images/heat-poster.jpg",
        ),
        (
            ImageKind::Banner,
            "/var/cache/noflex/images/heat-banner.jpg",
        ),
    ] {
        h.storage
            .upsert_image(&Image {
                target: ImageTarget::Work(work.id),
                kind,
                source: ImageSource::Provider,
                path: path.into(),
                width: None,
                height: None,
                language: None,
            })
            .await
            .expect("seed provider image");
    }

    common::touch(&h.lib_root, "Heat (1995)/poster.jpg", b"p");
    common::touch(&h.lib_root, "Heat (1995)/fanart.jpg", b"b");
    scan(&h).await;
    assert_eq!(
        art(&h, &heat).await,
        expect(&[
            ("primary", "Heat (1995)/poster.jpg"),
            ("backdrop", "Heat (1995)/fanart.jpg"),
            ("banner", "/var/cache/noflex/images/heat-banner.jpg"),
        ])
    );

    // A more specific name replaces the generic poster; removing the
    // fanart drops its row.
    common::touch(&h.lib_root, "Heat (1995)/Heat (1995).jpg", b"s");
    common::unlink(&h.lib_root, "Heat (1995)/fanart.jpg");
    scan(&h).await;
    assert_eq!(
        art(&h, &heat).await,
        expect(&[
            ("primary", "Heat (1995)/Heat (1995).jpg"),
            ("banner", "/var/cache/noflex/images/heat-banner.jpg"),
        ])
    );

    // With every local poster gone the provider poster shows through.
    common::unlink(&h.lib_root, "Heat (1995)/Heat (1995).jpg");
    common::unlink(&h.lib_root, "Heat (1995)/poster.jpg");
    scan(&h).await;
    assert_eq!(
        art(&h, &heat).await,
        expect(&[
            ("primary", "/var/cache/noflex/images/heat-poster.jpg"),
            ("banner", "/var/cache/noflex/images/heat-banner.jpg"),
        ])
    );
}
