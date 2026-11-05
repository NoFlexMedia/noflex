// SPDX-License-Identifier: AGPL-3.0-only
//
// Provider logos for the credits clients show beside provider data. Public, as
// a brand mark tells nobody anything, and so `<img src>` works from a linked
// server's page without a token. A logo can be an SVG, which would script this
// origin if opened directly, so every response is sandboxed.

use std::sync::Arc;

use axum::{
    Extension, Router,
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use noflex_storage::Storage;

/// Stops anything but the logo's own styles running, should a browser navigate
/// to one rather than load it as an image.
const LOGO_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/// The logo URL carries the logo's digest, so a new logo is a new URL.
const CACHE_FOREVER: &str = "public, max-age=31536000, immutable";

/// `GET /attributions/{provider}/logo`. Needs an [`Extension`] holding the store.
pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/attributions/{provider}/logo", get(logo))
}

async fn logo(
    Extension(storage): Extension<Arc<dyn Storage>>,
    Path(provider): Path<String>,
) -> Response {
    let logo = match storage.provider_attribution_logo(&provider).await {
        Ok(Some(logo)) => logo,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(%error, "attribution logo not read");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Ok(mime) = HeaderValue::from_str(&logo.mime) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(CACHE_FOREVER),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(LOGO_POLICY),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        logo.bytes,
    )
        .into_response()
}
