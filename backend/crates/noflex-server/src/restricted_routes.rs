// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use noflex_auth::{JwtConfig, Principal, auth_middleware};
use noflex_storage::{Storage, domain::ImageTarget};
use noflex_transcoder::FfmpegJobManager;

use crate::{
    channel_logos::ChannelLogos,
    file_serve::{RootCapability, ServeOpts, serve_file},
    images, optimized_serve, stream, trickplay_serve,
};

#[derive(Clone)]
pub struct RestrictedRouteState {
    pub storage: Arc<dyn Storage>,
    pub jwt: Arc<JwtConfig>,
    pub transcoder: Arc<FfmpegJobManager>,
    pub playback_job_authorizer: Arc<dyn stream::PlaybackJobAuthorizer>,
    pub transcode_cache_dir: PathBuf,
    pub image_cache_dir: PathBuf,
    pub recordings_dir: PathBuf,
    /// Downloaded subtitles; without it their tracks are `404`.
    pub subtitles: Option<Arc<noflex_plugin_host::SubtitleService>>,
}

pub fn routes(state: RestrictedRouteState) -> Router {
    let jwt = state.jwt.clone();
    let storage = state.storage.clone();
    let logos = match ChannelLogos::new(storage.clone(), &state.image_cache_dir) {
        Ok(logos) => Some(Arc::new(logos)),
        Err(error) => {
            tracing::error!(%error, "channel logo proxy unavailable");
            None
        }
    };
    Router::new()
        .route("/live-tv/channels/{channel_id}/logo", get(channel_logo))
        .route("/avatars/{user_id}", get(avatar))
        .route("/stream/{media_file_id}/file", get(stream_file))
        .route("/recordings/{recording_id}/file", get(recording_file))
        .route(
            "/stream/{media_file_id}/subtitles/{index}",
            get(stream_subtitle),
        )
        .route("/stream/job/{job_id}/master.m3u8", get(stream_master))
        .route("/stream/job/{job_id}/{filename}", get(stream_job_file))
        .route("/optimized/{version_id}/master.m3u8", get(optimized_master))
        .route("/optimized/{version_id}/{name}", get(optimized_part))
        .route("/images/{target_kind}/{target_id}/{kind}", get(serve_image))
        .route("/trickplay/{edition_id}/index.vtt", get(trickplay_vtt))
        .route(
            "/trickplay/{edition_id}/sheets/{index}",
            get(trickplay_sheet),
        )
        .with_state(state)
        .layer(middleware::from_fn(auth_middleware))
        .layer(axum::Extension(logos))
        .layer(axum::Extension(jwt.as_ref().clone()))
        .layer(axum::Extension(storage))
}

async fn channel_logo(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    axum::Extension(logos): axum::Extension<Option<Arc<ChannelLogos>>>,
    Path(channel_id): Path<String>,
    Query(query): Query<stream::TokenQuery>,
) -> Response {
    if let Err(status) = stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &stream::channel_logo_token_target(&channel_id),
    )
    .await
    {
        return status.into_response();
    }
    let Some(channel) = parse_path_id(&channel_id, noflex_core::ChannelId::from_uuid) else {
        return invalid_id_response(&channel_id);
    };
    match logos {
        Some(logos) => logos.serve(channel).await,
        None => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// A user's picture, to anyone signed in. It's JPEG by way of `setProfilePicture`,
/// and `?v=` in its URL changes with it.
async fn avatar(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(user_id): Path<String>,
    Query(query): Query<stream::TokenQuery>,
) -> Response {
    if let Err(status) = stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &stream::avatar_token_target(&user_id),
    )
    .await
    {
        return status.into_response();
    }
    let Some(user) = parse_path_id(&user_id, noflex_core::UserId::from_uuid) else {
        return invalid_id_response(&user_id);
    };
    match state.storage.user_avatar(user).await {
        Ok(Some(image)) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (
                    header::CACHE_CONTROL,
                    "private, max-age=31536000, immutable",
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            image,
        )
            .into_response(),
        Ok(None) => axum::http::StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(%error, "avatar not read");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn parse_path_id<T>(raw: &str, wrap: fn(uuid::Uuid) -> T) -> Option<T> {
    uuid::Uuid::parse_str(raw).map(wrap).ok()
}

fn invalid_id_response(raw: &str) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        format!("invalid id: {raw}"),
    )
        .into_response()
}

async fn recording_file(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !principal.is_authenticated() {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(id) = parse_path_id(&id, noflex_core::RecordingId::from_uuid) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let recording = match state.storage.get_recording(id).await {
        Ok(Some(recording)) if principal.is_admin || recording.owner_id == principal.user_id => {
            recording
        }
        Ok(_) => return axum::http::StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(%error, "recording lookup failed");
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if recording.status != noflex_storage::RecordingStatus::Completed {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    let Some(path) = recording.path else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let root = match RootCapability::new(&state.recordings_dir).await {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(%error, "recordings directory unavailable");
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let opened = match root.open(path).await {
        Ok(opened) => opened,
        Err(error) => {
            tracing::warn!(%error, "rejected recording file");
            return axum::http::StatusCode::NOT_FOUND.into_response();
        }
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    serve_file(opened, range, ServeOpts::default()).await
}

async fn stream_file(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(id): Path<String>,
    Query(query): Query<stream::TokenQuery>,
    headers: HeaderMap,
) -> Response {
    let id = match parse_path_id(&id, noflex_core::MediaFileId::from_uuid) {
        Some(id) => id,
        None => return invalid_id_response(&id),
    };
    let (principal, _) = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &format!("file:{id}"),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = stream::media_access(state.storage.as_ref(), &principal, id).await {
        return status.into_response();
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    stream::serve_media_file(&state.storage, id, range).await
}

/// The caller of an optimized-version route and the version, once they're
/// allowed to read it: its media token, and access to its source.
async fn authorize_optimized(
    state: &RestrictedRouteState,
    principal: &Principal,
    id: &str,
    token: Option<&str>,
) -> Result<(noflex_storage::domain::OptimizedVersion, Option<String>), Box<Response>> {
    let id = parse_path_id(id, noflex_core::OptimizedVersionId::from_uuid)
        .ok_or_else(|| Box::new(invalid_id_response(id)))?;
    let (principal, token) = stream::byte_route_principal(
        state.storage.as_ref(),
        principal,
        &state.jwt,
        token,
        &format!("optimized:{id}"),
    )
    .await
    .map_err(|error| Box::new(error.into_response()))?;
    let version = optimized_serve::ready_version(&state.storage, id)
        .await
        .map_err(|error| Box::new(error.into_response()))?;
    stream::media_access(state.storage.as_ref(), &principal, version.media_file_id)
        .await
        .map_err(|error| Box::new(error.into_response()))?;
    Ok((version, token))
}

async fn optimized_master(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(id): Path<String>,
    Query(query): Query<stream::TokenQuery>,
) -> Response {
    match authorize_optimized(&state, &principal, &id, query.token.as_deref()).await {
        Ok((version, token)) => {
            optimized_serve::serve_master(&state.storage, version, token.as_deref()).await
        }
        Err(response) => *response,
    }
}

async fn optimized_part(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path((id, name)): Path<(String, String)>,
    Query(query): Query<stream::TokenQuery>,
    headers: HeaderMap,
) -> Response {
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let (version, token) =
        match authorize_optimized(&state, &principal, &id, query.token.as_deref()).await {
            Ok(authorized) => authorized,
            Err(response) => return *response,
        };
    // Versions stay where they were written when the folder setting changes.
    let Some(dir) = version
        .path
        .as_deref()
        .and_then(|path| std::path::Path::new(path).parent())
    else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let root = match RootCapability::new(dir).await {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(%error, "optimized directory unavailable");
            return axum::http::StatusCode::NOT_FOUND.into_response();
        }
    };
    optimized_serve::serve_part(&version, &root, &name, token.as_deref(), range).await
}

async fn stream_subtitle(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path((id, index)): Path<(String, String)>,
    Query(query): Query<stream::TokenQuery>,
) -> Response {
    let id = match parse_path_id(&id, noflex_core::MediaFileId::from_uuid) {
        Some(id) => id,
        None => return invalid_id_response(&id),
    };
    let (principal, _) = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &format!("file:{id}"),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = stream::media_access(state.storage.as_ref(), &principal, id).await {
        return status.into_response();
    }
    let index = match index.parse::<usize>() {
        Ok(index) => index,
        Err(_) => return invalid_id_response(&index),
    };
    stream::serve_subtitle(
        &state.storage,
        state.transcoder.ffmpeg_path(),
        &state.transcode_cache_dir,
        state.subtitles.as_deref(),
        id,
        index,
    )
    .await
}

async fn stream_master(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(job_id): Path<String>,
    Query(query): Query<stream::TokenQuery>,
) -> Response {
    let (principal, token) = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &format!("job:{job_id}"),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = state
        .playback_job_authorizer
        .authorize_job(&principal, &job_id)
        .await
    {
        return status.into_response();
    }
    stream::master_playlist(state.transcoder.registry(), &job_id, token.as_deref()).await
}

async fn stream_job_file(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path((job_id, filename)): Path<(String, String)>,
    Query(query): Query<stream::TokenQuery>,
) -> Response {
    let (principal, token) = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &format!("job:{job_id}"),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = state
        .playback_job_authorizer
        .authorize_job(&principal, &job_id)
        .await
    {
        return status.into_response();
    }
    stream::serve_job_file(
        &state.transcoder,
        &state.transcode_cache_dir,
        &job_id,
        &filename,
        token.as_deref(),
    )
    .await
}

async fn serve_image(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path((target_kind, target_id, kind)): Path<(String, String, String)>,
    Query(query): Query<images::ImageQuery>,
) -> Response {
    let (principal, _) = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &format!("img:{target_kind}:{target_id}"),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(status) => return status.into_response(),
    };
    let target = match target_kind.as_str() {
        "work" => parse_path_id(&target_id, noflex_core::WorkId::from_uuid).map(ImageTarget::Work),
        "edition" => {
            parse_path_id(&target_id, noflex_core::EditionId::from_uuid).map(ImageTarget::Edition)
        }
        "tag" => parse_path_id(&target_id, noflex_core::TagId::from_uuid).map(ImageTarget::Tag),
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                format!("invalid image target kind: {target_kind}"),
            )
                .into_response();
        }
    };
    let Some(target) = target else {
        return invalid_id_response(&target_id);
    };
    let access = match target {
        ImageTarget::Work(id) => stream::work_access(state.storage.as_ref(), &principal, id).await,
        ImageTarget::Edition(id) => {
            stream::edition_access(state.storage.as_ref(), &principal, id).await
        }
        ImageTarget::Tag(_) => Ok(()),
    };
    if let Err(status) = access {
        return status.into_response();
    }
    let Some(kind) = noflex_core::ImageKind::from_str(&kind) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!("invalid image kind: {kind}"),
        )
            .into_response();
    };
    images::serve_image(&state.storage, &state.image_cache_dir, target, kind, query).await
}

async fn trickplay_vtt(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(edition_id): Path<String>,
    Query(query): Query<stream::TokenQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(edition) = parse_path_id(&edition_id, noflex_core::EditionId::from_uuid) else {
        return (axum::http::StatusCode::NOT_FOUND, "trickplay not found").into_response();
    };
    let (principal, token) = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &format!("trickplay:{edition}"),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = stream::edition_access(state.storage.as_ref(), &principal, edition).await {
        return status.into_response();
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    trickplay_serve::serve_vtt(
        &state.storage,
        &state.image_cache_dir.join("trickplay"),
        edition,
        range,
        token.as_deref(),
    )
    .await
}

async fn trickplay_sheet(
    State(state): State<RestrictedRouteState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path((edition_id, index)): Path<(String, String)>,
    Query(query): Query<stream::TokenQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(edition) = parse_path_id(&edition_id, noflex_core::EditionId::from_uuid) else {
        return (axum::http::StatusCode::NOT_FOUND, "trickplay not found").into_response();
    };
    let Some(index) = trickplay_serve::parse_sheet_index(&index) else {
        return (axum::http::StatusCode::NOT_FOUND, "trickplay not found").into_response();
    };
    let (principal, _) = match stream::byte_route_principal(
        state.storage.as_ref(),
        &principal,
        &state.jwt,
        query.token.as_deref(),
        &format!("trickplay:{edition}"),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(status) => return status.into_response(),
    };
    if let Err(status) = stream::edition_access(state.storage.as_ref(), &principal, edition).await {
        return status.into_response();
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    trickplay_serve::serve_sheet(
        &state.storage,
        &state.image_cache_dir.join("trickplay"),
        edition,
        index,
        range,
    )
    .await
}
