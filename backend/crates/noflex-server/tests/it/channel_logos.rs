// SPDX-License-Identifier: AGPL-3.0-only
//
// The Live TV logo proxy: fetch once, serve from cache, refuse anything
// that isn't a raster image.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use noflex_server::channel_logos::ChannelLogos;
use noflex_storage::{
    Storage,
    domain::{LiveTvChannel, LiveTvSource},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

/// An HTTP server answering every request with `body`; returns its URL
/// and a request counter.
async fn upstream(body: &'static [u8]) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body).await;
        }
    });
    (format!("http://{address}/logo"), hits)
}

async fn channel_with_logo(logo: Option<String>) -> (Arc<dyn Storage>, noflex_core::ChannelId) {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store: Arc<dyn Storage> = Arc::new(store);
    let source = LiveTvSource {
        id: noflex_core::LiveTvSourceId::new(),
        name: "Tuner".into(),
        enabled: true,
        last_refresh_at: None,
        last_error: None,
        playlist_url: "http://127.0.0.1/playlist.m3u".into(),
        epg_url: None,
    };
    store.upsert_live_tv_source(&source).await.expect("source");
    let channel = LiveTvChannel {
        id: noflex_core::ChannelId::new(),
        source_id: source.id,
        external_id: "news".into(),
        name: "News".into(),
        number: None,
        logo_url: logo,
        group: None,
        enabled: true,
        stream_url: "http://127.0.0.1/live/news".into(),
    };
    store
        .apply_live_tv_refresh(
            source.id,
            std::slice::from_ref(&channel),
            &[],
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("channel");
    (store, channel.id)
}

async fn body(response: axum::response::Response) -> (axum::http::StatusCode, Vec<u8>) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, bytes.to_vec())
}

#[tokio::test]
async fn raster_logo_is_fetched_once_then_served_from_cache() {
    let (url, hits) = upstream(PNG).await;
    let (store, channel) = channel_with_logo(Some(url)).await;
    let cache = tempfile::tempdir().expect("cache");
    let logos = ChannelLogos::new(store, cache.path()).expect("logos");

    let first = logos.serve(channel).await;
    assert_eq!(
        first.headers()[axum::http::header::CONTENT_TYPE],
        "image/png"
    );
    assert_eq!(
        body(first).await,
        (axum::http::StatusCode::OK, PNG.to_vec())
    );
    let (status, _) = body(logos.serve(channel).await).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "second request is cached");
}

#[tokio::test]
async fn svg_and_html_logos_are_refused() {
    for payload in [
        &b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>"[..],
        &b"<html><script>alert(1)</script></html>"[..],
    ] {
        let payload: &'static [u8] = Box::leak(payload.to_vec().into_boxed_slice());
        let (url, _) = upstream(payload).await;
        let (store, channel) = channel_with_logo(Some(url)).await;
        let cache = tempfile::tempdir().expect("cache");
        let logos = ChannelLogos::new(store, cache.path()).expect("logos");
        let (status, _) = body(logos.serve(channel).await).await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn channel_without_logo_or_unreachable_logo() {
    let cache = tempfile::tempdir().expect("cache");
    let (store, channel) = channel_with_logo(None).await;
    let logos = ChannelLogos::new(store, cache.path()).expect("logos");
    assert_eq!(
        logos.serve(channel).await.status(),
        axum::http::StatusCode::NOT_FOUND
    );

    let (store, channel) = channel_with_logo(Some("http://127.0.0.1:9/nothing".into())).await;
    let logos = ChannelLogos::new(store, cache.path()).expect("logos");
    assert_eq!(
        logos.serve(channel).await.status(),
        axum::http::StatusCode::BAD_GATEWAY
    );
}
