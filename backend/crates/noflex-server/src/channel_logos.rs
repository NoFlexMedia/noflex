// SPDX-License-Identifier: AGPL-3.0-only
//
// Same-origin proxy for Live TV channel logos. M3U `tvg-logo` URLs point
// at arbitrary hosts, which the web app's `img-src 'self'` CSP blocks, so
// each logo is fetched once with the Live TV source client policy (http(s)
// only, no redirects, capped body, finite timeouts) and cached on disk,
// keyed by channel and URL so an updated playlist refetches. Only raster
// formats identified by magic bytes are kept: an SVG or HTML body served
// from this origin could script it on direct navigation.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use noflex_core::ChannelId;
use noflex_storage::Storage;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{
    file_serve::{RootCapability, ServeOpts, serve_file},
    live_tv::fetch_capped,
};

/// Logos are small; anything bigger is not a logo.
const LOGO_CAP: usize = 2 * 1024 * 1024;

pub struct ChannelLogos {
    storage: Arc<dyn Storage>,
    client: reqwest::Client,
    cache_dir: PathBuf,
    fetches: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl ChannelLogos {
    /// `cache_dir` is the image cache root; logos live under
    /// `channel-logos/` inside it.
    pub fn new(storage: Arc<dyn Storage>, cache_dir: &Path) -> reqwest::Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            storage,
            client,
            cache_dir: cache_dir.join("channel-logos"),
            fetches: Mutex::new(HashMap::new()),
        })
    }

    /// Serve `channel`'s logo: `404` when it has none or the upstream
    /// body isn't a supported raster image, `502` when the fetch fails.
    /// Concurrent first requests for one logo share a single fetch.
    pub async fn serve(&self, channel: ChannelId) -> Response {
        let url = match self.storage.get_live_tv_channel(channel).await {
            Ok(Some(row)) => match row.logo_url {
                Some(url) => url,
                None => return StatusCode::NOT_FOUND.into_response(),
            },
            Ok(None) => return StatusCode::NOT_FOUND.into_response(),
            Err(error) => {
                tracing::error!(%error, "storage error reading channel");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        let digest = hex::encode(&Sha256::digest(url.as_bytes())[..12]);
        let stem = self.cache_dir.join(format!("{channel}-{digest}"));

        if let Some(response) = self.serve_cached(&stem).await {
            return response;
        }
        let lock = {
            let mut fetches = self.fetches.lock().await;
            fetches.entry(stem.clone()).or_default().clone()
        };
        let _guard = lock.lock().await;
        // Another request may have fetched it while we waited.
        if let Some(response) = self.serve_cached(&stem).await {
            return response;
        }
        let result = self.fetch(&url, &stem).await;
        self.fetches.lock().await.remove(&stem);
        match result {
            Ok(()) => self
                .serve_cached(&stem)
                .await
                .unwrap_or_else(|| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            Err(status) => status.into_response(),
        }
    }

    async fn fetch(&self, url: &str, stem: &Path) -> Result<(), StatusCode> {
        let bytes = fetch_capped(&self.client, url, LOGO_CAP)
            .await
            .map_err(|error| {
                tracing::debug!(%error, url, "channel logo fetch failed");
                StatusCode::BAD_GATEWAY
            })?;
        let ext = raster_extension(&bytes).ok_or(StatusCode::NOT_FOUND)?;
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let path = stem.with_extension(ext);
        let partial = stem.with_extension(format!("{ext}.part"));
        tokio::fs::write(&partial, &bytes)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        tokio::fs::rename(&partial, &path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }

    async fn serve_cached(&self, stem: &Path) -> Option<Response> {
        let root = RootCapability::new(&self.cache_dir).await.ok()?;
        for ext in RASTER_EXTENSIONS {
            if let Ok(opened) = root.open(&stem.with_extension(ext)).await {
                return Some(
                    serve_file(
                        opened,
                        None,
                        ServeOpts {
                            content_disposition: None,
                            cache_control: Some("private, max-age=86400"),
                        },
                    )
                    .await,
                );
            }
        }
        None
    }
}

const RASTER_EXTENSIONS: [&str; 4] = ["png", "jpg", "gif", "webp"];

/// The file extension for a PNG/JPEG/GIF/WebP body, by magic bytes.
fn raster_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_raster_magic_is_accepted() {
        assert_eq!(raster_extension(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(raster_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(raster_extension(b"GIF89a..."), Some("gif"));
        assert_eq!(raster_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(
            raster_extension(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            None
        );
        assert_eq!(raster_extension(b"<html><script>"), None);
        assert_eq!(raster_extension(b""), None);
    }
}
