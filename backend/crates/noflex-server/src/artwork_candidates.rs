// SPDX-License-Identifier: AGPL-3.0-only
//
// Previews of the artwork candidates `artworkCandidates` offers an
// administrator. The bytes come from the candidate's provider through its
// image hook: the host never fetches a provider URL, and the route names a
// candidate only by the id the service registered when it offered it, so a
// client can't have a plugin fetch anything that wasn't offered.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use noflex_auth::{JwtConfig, Principal, auth_middleware};
use noflex_plugin_host::{MetadataError, MetadataService};
use noflex_storage::Storage;

use crate::stream;

/// What the preview route reads.
#[derive(Clone)]
pub struct ArtworkCandidateState {
    pub service: Arc<MetadataService>,
    pub storage: Arc<dyn Storage>,
    pub jwt: Arc<JwtConfig>,
}

/// `GET /artwork-candidates/{id}/preview`, for an administrator by header
/// credential or the `?token=` in `ArtworkCandidate.previewUrl`.
pub fn routes(state: ArtworkCandidateState) -> Router {
    let jwt = state.jwt.clone();
    let storage = state.storage.clone();
    Router::new()
        .route("/artwork-candidates/{id}/preview", get(preview))
        .with_state(state)
        .layer(middleware::from_fn(auth_middleware))
        .layer(axum::Extension(jwt.as_ref().clone()))
        .layer(axum::Extension(storage))
}

async fn preview(
    State(state): State<ArtworkCandidateState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(id): Path<String>,
    Query(query): Query<stream::TokenQuery>,
) -> Response {
    let principal = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &stream::artwork_candidate_token_target(&id),
    )
    .await
    {
        Ok((principal, _)) => principal,
        Err(status) => return status.into_response(),
    };
    if !principal.is_admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    match state.service.candidate_preview(&id).await {
        Ok(image) => (
            [
                (header::CONTENT_TYPE, image.mime),
                (header::CACHE_CONTROL, "private, max-age=1800"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            image.bytes,
        )
            .into_response(),
        Err(MetadataError::UnknownCandidate) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(candidate = %id, %error, "artwork candidate preview failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}
