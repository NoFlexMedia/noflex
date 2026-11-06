// SPDX-License-Identifier: AGPL-3.0-only
//
// HTTP serve path for images attached to works, editions, and tags.
// Originals stream straight from the path recorded in the `images`
// table; `?w=`/`?h=` resizes are rendered on the fly and cached under
// the image cache dir, keyed by target/kind/geometry so each
// geometry is rendered once. The `serve_image` axum handler in
// `main.rs` is a thin wrapper that parses the route segments; the
// integration tests in `tests/it/images.rs` call this directly.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use noflex_core::ImageKind;
use noflex_storage::{
    Storage,
    domain::{ImageSource, ImageTarget},
};

use crate::file_serve::{OpenedFile, RootCapability, ServeOpts, open_from_roots, serve_file};

/// Hard cap on requested resize dimensions; beyond this the Lanczos
/// render gets silly-expensive for a poster fetch.
const MAX_DIMENSION: u32 = 4096;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ImageQuery {
    pub w: Option<u32>,
    pub h: Option<u32>,
    pub fill: Option<bool>,
    /// Pins one image source (`local`, `provider`, `generated`); absent,
    /// the target's effective image is served.
    pub source: Option<String>,
    /// Scoped media token, checked by the route handler against
    /// `img:{target_kind}:{target_id}`; ignored here.
    pub token: Option<String>,
}

pub async fn serve_image(
    storage: &Arc<dyn Storage>,
    cache_dir: &Path,
    target: ImageTarget,
    kind: ImageKind,
    query: ImageQuery,
) -> Response {
    let pinned;
    let order = match query.source.as_deref() {
        Some(source) => match ImageSource::from_str(source) {
            Some(source) => {
                pinned = [source];
                &pinned[..]
            }
            None => return (StatusCode::NOT_FOUND, "image not found").into_response(),
        },
        None => match target.source_order(storage.as_ref()).await {
            Ok(order) => order,
            Err(e) => {
                tracing::error!(error = %e, "storage error reading artwork preference");
                return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
            }
        },
    };
    let image = match storage.image(target, kind, order).await {
        Ok(Some(image)) => image,
        Ok(None) => return (StatusCode::NOT_FOUND, "image not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "storage error reading image");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };

    let cache_root = match RootCapability::new(cache_dir).await {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(%error, path = %cache_dir.display(), "image cache unavailable");
            return (StatusCode::INTERNAL_SERVER_ERROR, "image cache unavailable").into_response();
        }
    };
    let source_path = PathBuf::from(&image.path);
    let source_roots = match image_source_roots(storage, target, cache_root.path()).await {
        Ok(roots) => roots,
        Err(status) => return status.into_response(),
    };
    let source = match open_from_roots(&source_roots, &source_path).await {
        Ok(source) => source,
        Err(error) => {
            tracing::warn!(%error, path = %source_path.display(), "rejected image source");
            return (StatusCode::NOT_FOUND, "image not found").into_response();
        }
    };
    let source_mtime = source.metadata().modified().ok();

    let w = query.w.map(|v| v.clamp(1, MAX_DIMENSION));
    let h = query.h.map(|v| v.clamp(1, MAX_DIMENSION));
    // Center-crop fill only makes sense with a full target box; a
    // one-sided request falls back to aspect-fit.
    let fill = query.fill.unwrap_or(false) && w.is_some() && h.is_some();

    let opened = if w.is_none() && h.is_none() {
        source
    } else {
        // The source is part of the key: switching the effective source
        // must not serve the other source's render.
        let geometry = format!(
            "{}-{}-{}x{}{}",
            kind.as_str(),
            image.source.as_str(),
            w.map_or_else(|| "auto".to_string(), |v| v.to_string()),
            h.map_or_else(|| "auto".to_string(), |v| v.to_string()),
            if fill { "-fill" } else { "" },
        );
        let ext = source_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_string();
        let cached = cache_dir
            .join(target.kind_str())
            .join(target.id().to_string())
            .join(format!("{geometry}.{ext}"));
        if let Some(opened) = cache_fresh(&cache_root, &cached, source_mtime).await {
            opened
        } else {
            if let Err(resp) = render_resize(source, &source_path, &cached, w, h, fill).await {
                return *resp;
            }
            match cache_root.open(&cached).await {
                Ok(opened) => opened,
                Err(error) => {
                    tracing::error!(%error, path = %cached.display(), "rejected rendered image");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "cache read failed")
                        .into_response();
                }
            }
        }
    };

    serve_file(
        opened,
        None,
        ServeOpts {
            content_disposition: None,
            cache_control: Some("private, max-age=86400"),
        },
    )
    .await
}

async fn image_source_roots(
    storage: &Arc<dyn Storage>,
    target: ImageTarget,
    cache_root: &Path,
) -> Result<Vec<String>, StatusCode> {
    let mut roots = vec![cache_root.to_string_lossy().into_owned()];
    let work = match target {
        ImageTarget::Work(id) => storage
            .get_work(id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        ImageTarget::Edition(id) => {
            let edition = storage
                .get_edition(id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            match edition {
                Some(edition) => storage
                    .get_work(edition.work_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                None => None,
            }
        }
        ImageTarget::Tag(_) => None,
    };
    if let Some(work) = work
        && let Some(library) = storage
            .get_library(work.library_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        roots.extend(library.paths);
    }
    Ok(roots)
}

/// A cached render is reusable only when it's at least as new as the
/// source — a replaced source file must re-render every geometry.
async fn cache_fresh(
    root: &RootCapability,
    cached: &Path,
    source_mtime: Option<std::time::SystemTime>,
) -> Option<OpenedFile> {
    let opened = root.open(cached).await.ok()?;
    match (opened.metadata().modified().ok(), source_mtime) {
        (Some(cached_mtime), Some(source_mtime)) if cached_mtime >= source_mtime => Some(opened),
        (Some(_), None) => Some(opened),
        _ => None,
    }
}

async fn render_resize(
    source: OpenedFile,
    source_path: &Path,
    dest: &Path,
    w: Option<u32>,
    h: Option<u32>,
    fill: bool,
) -> Result<(), Box<Response>> {
    if let Some(parent) = dest.parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        tracing::error!(error = %e, path = %parent.display(), "image cache dir create failed");
        return Err(Box::new(
            (StatusCode::INTERNAL_SERVER_ERROR, "cache write failed").into_response(),
        ));
    }

    let source_path = source_path.to_path_buf();
    let source = source.into_file().into_std().await;
    let dest = dest.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let reader = image::ImageReader::new(std::io::BufReader::new(source))
            .with_guessed_format()
            .map_err(|e| format!("detect {}: {e}", source_path.display()))?;
        let img = reader
            .decode()
            .map_err(|e| format!("decode {}: {e}", source_path.display()))?;
        let resized = match (fill, w, h) {
            (true, Some(w), Some(h)) => {
                img.resize_to_fill(w, h, image::imageops::FilterType::Lanczos3)
            }
            _ => img.resize(
                w.unwrap_or(u32::MAX),
                h.unwrap_or(u32::MAX),
                image::imageops::FilterType::Lanczos3,
            ),
        };
        let format = match source_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("jpg" | "jpeg") => image::ImageFormat::Jpeg,
            Some("webp") => image::ImageFormat::WebP,
            _ => image::ImageFormat::Png,
        };
        // Temp sibling + rename: two concurrent renders of the same
        // geometry must never publish a partial file. The uuid keeps
        // the temp names from colliding with each other.
        let tmp = dest.with_file_name(format!(
            ".{}.{}.tmp",
            dest.file_name().and_then(|n| n.to_str()).unwrap_or("image"),
            uuid::Uuid::now_v7()
        ));
        resized
            .save_with_format(&tmp, format)
            .map_err(|e| format!("encode {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &dest).map_err(|e| format!("rename {}: {e}", dest.display()))?;
        Ok(())
    })
    .await;

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            tracing::error!(error = %e, "image resize failed");
            Err(Box::new(
                (StatusCode::INTERNAL_SERVER_ERROR, "resize failed").into_response(),
            ))
        }
        Err(e) => {
            tracing::error!(error = %e, "image resize task panicked");
            Err(Box::new(
                (StatusCode::INTERNAL_SERVER_ERROR, "resize failed").into_response(),
            ))
        }
    }
}
