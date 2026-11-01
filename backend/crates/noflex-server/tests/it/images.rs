// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use noflex_server::images::{ImageQuery, serve_image};
use noflex_storage::store::Storage;
use noflex_storage_turso::TursoStore;

fn no_resize() -> ImageQuery {
    ImageQuery {
        w: None,
        h: None,
        fill: None,
        source: None,
        token: None,
    }
}

/// Temp dir pair (source + cache) plus a store with one image row
/// pointing at a real 200x100 PNG fixture.
async fn setup(
    tag: &str,
) -> (
    Arc<dyn Storage>,
    noflex_core::WorkId,
    std::path::PathBuf,
    std::path::PathBuf,
    Vec<u8>,
) {
    let dir = std::env::temp_dir().join(format!("noflex-img-{tag}-{}", uuid::Uuid::now_v7()));
    let cache_dir = dir.join("cache");
    std::fs::create_dir_all(&cache_dir).expect("dirs");

    let png_path = cache_dir.join("poster.png");
    let fixture = image::RgbImage::from_fn(200, 100, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
    });
    fixture.save(&png_path).expect("write fixture");
    let original_bytes = std::fs::read(&png_path).expect("read fixture");

    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let store: Arc<dyn Storage> = Arc::new(store);

    let work_id = noflex_core::WorkId::new();
    let image = noflex_storage::domain::Image {
        target: noflex_storage::domain::ImageTarget::Work(work_id),
        kind: noflex_core::ImageKind::Primary,
        source: noflex_storage::domain::ImageSource::Local,
        path: png_path.to_string_lossy().to_string(),
        width: Some(200),
        height: Some(100),
        language: None,
    };
    store.upsert_image(&image).await.expect("upsert image");

    (store, work_id, png_path, cache_dir, original_bytes)
}

fn target(work_id: noflex_core::WorkId) -> noflex_storage::domain::ImageTarget {
    noflex_storage::domain::ImageTarget::Work(work_id)
}

async fn body_bytes(resp: axum::response::Response) -> axum::body::Bytes {
    axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body")
}

#[tokio::test]
async fn serve_original_returns_source_bytes() {
    let (store, work_id, _png, cache_dir, original_bytes) = setup("orig").await;

    let resp = serve_image(
        &store,
        &cache_dir,
        target(work_id),
        noflex_core::ImageKind::Primary,
        no_resize(),
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
    let body = body_bytes(resp).await;
    assert_eq!(&body[..], &original_bytes[..]);
}

#[tokio::test]
async fn source_outside_image_cache_is_rejected() {
    let (store, work_id, _, cache_dir, _) = setup("outside").await;
    let outside = cache_dir.parent().unwrap().join("outside.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
        .save(&outside)
        .expect("outside image");
    store
        .upsert_image(&noflex_storage::domain::Image {
            target: target(work_id),
            kind: noflex_core::ImageKind::Primary,
            source: noflex_storage::domain::ImageSource::Local,
            path: outside.to_string_lossy().into_owned(),
            width: Some(2),
            height: Some(2),
            language: None,
        })
        .await
        .expect("replace image");

    let response = serve_image(
        &store,
        &cache_dir,
        target(work_id),
        noflex_core::ImageKind::Primary,
        no_resize(),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serve_unknown_target_returns_404() {
    let (store, _work_id, _png, cache_dir, _bytes) = setup("missing").await;

    let resp = serve_image(
        &store,
        &cache_dir,
        target(noflex_core::WorkId::new()),
        noflex_core::ImageKind::Primary,
        no_resize(),
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn resize_by_width_keeps_aspect_and_caches() {
    let (store, work_id, _png, cache_dir, _bytes) = setup("resize").await;

    let resp = serve_image(
        &store,
        &cache_dir,
        target(work_id),
        noflex_core::ImageKind::Primary,
        ImageQuery {
            w: Some(50),
            h: None,
            fill: None,
            source: None,
            token: None,
        },
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(resp).await;
    let decoded = image::load_from_memory(&body).expect("decode resized");
    assert_eq!(decoded.width(), 50);
    assert_eq!(decoded.height(), 25, "aspect-fit from 200x100 at w=50");

    let cached = cache_dir
        .join("work")
        .join(work_id.to_string())
        .join("primary-local-50xauto.png");
    assert!(cached.exists(), "render must land in the cache dir");
}

#[tokio::test]
async fn second_identical_request_serves_from_cache() {
    let (store, work_id, _png, cache_dir, _bytes) = setup("cached").await;
    let query = ImageQuery {
        w: Some(50),
        h: None,
        fill: None,
        source: None,
        token: None,
    };

    let resp = serve_image(
        &store,
        &cache_dir,
        target(work_id),
        noflex_core::ImageKind::Primary,
        query.clone(),
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    drop(body_bytes(resp).await);

    // Plant a different image at the cache path (fresh mtime): if the
    // next request re-rendered, the planted bytes would be replaced.
    let cached = cache_dir
        .join("work")
        .join(work_id.to_string())
        .join("primary-local-50xauto.png");
    let plant = image::RgbImage::from_pixel(10, 10, image::Rgb([0, 0, 0]));
    plant.save(&cached).expect("plant cache");

    let resp = serve_image(
        &store,
        &cache_dir,
        target(work_id),
        noflex_core::ImageKind::Primary,
        query,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(resp).await;
    let decoded = image::load_from_memory(&body).expect("decode cached");
    assert_eq!(
        (decoded.width(), decoded.height()),
        (10, 10),
        "response must be the planted cache file, not a re-render"
    );
}

#[tokio::test]
async fn fill_resize_crops_to_exact_box() {
    let (store, work_id, _png, cache_dir, _bytes) = setup("fill").await;

    let resp = serve_image(
        &store,
        &cache_dir,
        target(work_id),
        noflex_core::ImageKind::Primary,
        ImageQuery {
            w: Some(50),
            h: Some(50),
            fill: Some(true),
            source: None,
            token: None,
        },
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(resp).await;
    let decoded = image::load_from_memory(&body).expect("decode fill");
    assert_eq!((decoded.width(), decoded.height()), (50, 50));
}

/// Without `?source=` the route serves the library's preferred source,
/// re-reading the preference per request; a pinned source wins, and
/// resized renders of the two sources never collide in the cache.
#[tokio::test]
async fn effective_source_follows_the_library_preference() {
    use noflex_storage::domain::{ArtworkPreference, Image, ImageSource, Library, LibraryKind};

    let dir = std::env::temp_dir().join(format!("noflex-img-pref-{}", uuid::Uuid::now_v7()));
    let cache_dir = dir.join("cache");
    let lib_root = dir.join("media");
    std::fs::create_dir_all(&cache_dir).expect("cache dir");
    std::fs::create_dir_all(lib_root.join("Dune")).expect("media dir");
    let local_path = lib_root.join("Dune/poster.png");
    let provider_path = cache_dir.join("provider.png");
    image::RgbImage::from_pixel(40, 60, image::Rgb([200, 0, 0]))
        .save(&local_path)
        .expect("local");
    image::RgbImage::from_pixel(40, 60, image::Rgb([0, 0, 200]))
        .save(&provider_path)
        .expect("provider");

    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let store: Arc<dyn Storage> = Arc::new(store);
    let mut library = Library {
        id: noflex_core::LibraryId::new(),
        name: "Films".into(),
        kind: LibraryKind::Movies,
        paths: vec![lib_root.to_string_lossy().into_owned()],
        config: Default::default(),
    };
    store.upsert_library(&library).await.expect("library");
    let now = time::OffsetDateTime::now_utc();
    let work = noflex_storage::domain::Work {
        id: noflex_core::WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Dune".into(),
        sort_title: "dune".into(),
        year: None,
        release_date: None,
        overview: None,
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: noflex_core::MetadataStatus::Unconfirmed,
        locked_fields: vec![],
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&work).await.expect("work");
    for (source, path) in [
        (ImageSource::Local, &local_path),
        (ImageSource::Provider, &provider_path),
    ] {
        store
            .upsert_image(&Image {
                target: target(work.id),
                kind: noflex_core::ImageKind::Primary,
                source,
                path: path.to_string_lossy().into_owned(),
                width: None,
                height: None,
                language: None,
            })
            .await
            .expect("image");
    }

    let blue_or_red = |source: Option<&str>, w: Option<u32>| {
        let store = store.clone();
        let cache_dir = cache_dir.clone();
        let source = source.map(str::to_string);
        async move {
            let resp = serve_image(
                &store,
                &cache_dir,
                target(work.id),
                noflex_core::ImageKind::Primary,
                ImageQuery {
                    w,
                    h: None,
                    fill: None,
                    source,
                    token: None,
                },
            )
            .await;
            assert_eq!(resp.status(), axum::http::StatusCode::OK);
            let pixel = image::load_from_memory(&body_bytes(resp).await)
                .expect("decode")
                .to_rgb8()
                .get_pixel(0, 0)
                .0;
            if pixel[2] > pixel[0] {
                "provider"
            } else {
                "local"
            }
        }
    };

    assert_eq!(blue_or_red(None, None).await, "provider");
    assert_eq!(blue_or_red(None, Some(20)).await, "provider");
    library.config.artwork_preference = ArtworkPreference::PreferLocal;
    store.upsert_library(&library).await.expect("prefer local");
    assert_eq!(blue_or_red(None, None).await, "local");
    assert_eq!(
        blue_or_red(None, Some(20)).await,
        "local",
        "the provider's render is not reused"
    );
    assert_eq!(blue_or_red(Some("provider"), None).await, "provider");

    let resp = serve_image(
        &store,
        &cache_dir,
        target(work.id),
        noflex_core::ImageKind::Primary,
        ImageQuery {
            source: Some("generated".into()),
            ..no_resize()
        },
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}
