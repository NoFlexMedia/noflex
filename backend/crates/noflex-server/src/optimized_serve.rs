// SPDX-License-Identifier: AGPL-3.0-only
//
// HTTP serve path for optimized versions: HLS manifests rendered from the
// stored index, plus byte ranges of the one file behind them. The same file
// route serves smart downloads.

use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use noflex_core::OptimizedVersionId;
use noflex_storage::{
    Storage,
    domain::{OptimizedVersion, OptimizedVersionStatus},
};
use noflex_transcoder::{
    optimize::version_master_playlist,
    package::{PackageIndex, TrackIndex, media_playlist, single_track_init},
    probe::ProbeReport,
};
use time::OffsetDateTime;

use crate::{
    file_serve::{RootCapability, ServeOpts, serve_file},
    stream::propagate_token_to_playlist,
};

const PLAYLIST_TYPE: &str = "application/vnd.apple.mpegurl";

/// A ready version, or `404` for anything else: missing, unfinished, or broken.
pub async fn ready_version(
    storage: &Arc<dyn Storage>,
    id: OptimizedVersionId,
) -> Result<OptimizedVersion, StatusCode> {
    match storage.get_optimized_version(id).await {
        Ok(Some(version)) if version.status == OptimizedVersionStatus::Ready => Ok(version),
        Ok(_) => Err(StatusCode::NOT_FOUND),
        Err(error) => {
            tracing::error!(%error, version = %id, "optimized version lookup failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn index_of(version: &OptimizedVersion) -> Result<PackageIndex, StatusCode> {
    version
        .index
        .clone()
        .and_then(|index| serde_json::from_value(index).ok())
        .ok_or_else(|| {
            tracing::error!(version = %version.id, "optimized version has no readable index");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn playlist(body: String, token: Option<&str>) -> Response {
    let body = match token {
        Some(token) => propagate_token_to_playlist(&body, token),
        None => body,
    };
    ([(header::CONTENT_TYPE, PLAYLIST_TYPE)], body).into_response()
}

/// Serve the master playlist. Playing a version counts as using it, which is
/// what keeps it from eviction.
pub async fn serve_master(
    storage: &Arc<dyn Storage>,
    mut version: OptimizedVersion,
    token: Option<&str>,
) -> Response {
    let index = match index_of(&version) {
        Ok(index) => index,
        Err(status) => return status.into_response(),
    };
    let Some(probe) = version
        .probe
        .clone()
        .and_then(|probe| serde_json::from_value::<ProbeReport>(probe).ok())
    else {
        tracing::error!(version = %version.id, "optimized version has no readable probe");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let body = version_master_playlist(&index, &probe);
    version.last_used_at = Some(OffsetDateTime::now_utc());
    if let Err(error) = storage.update_optimized_version(&version).await {
        tracing::warn!(%error, version = %version.id, "optimized version: last use not recorded");
    }
    playlist(body, token)
}

fn track_named<'a>(
    index: &'a PackageIndex,
    name: &str,
    prefix: &str,
    suffix: &str,
) -> Option<&'a TrackIndex> {
    let id = name
        .strip_prefix(prefix)?
        .strip_suffix(suffix)?
        .parse::<u32>()
        .ok()?;
    index.tracks.iter().find(|track| track.track_id == id)
}

/// Serve one of the files a master playlist leads to: `video.m3u8`,
/// `audio_{id}.m3u8`, `init_{id}.mp4`, or the `file` itself.
pub async fn serve_part(
    version: &OptimizedVersion,
    root: &RootCapability,
    name: &str,
    token: Option<&str>,
    range: Option<&str>,
) -> Response {
    let Some(path) = version.path.as_deref().map(std::path::Path::new) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if name == "file" {
        return match root.open(path).await {
            Ok(opened) => serve_file(opened, range, ServeOpts::default()).await,
            Err(error) => {
                tracing::warn!(%error, version = %version.id, "optimized file missing");
                StatusCode::NOT_FOUND.into_response()
            }
        };
    }
    let index = match index_of(version) {
        Ok(index) => index,
        Err(status) => return status.into_response(),
    };
    let init_name = |track: &TrackIndex| format!("init_{}.mp4", track.track_id);
    if name == "video.m3u8" {
        let Some(track) = index
            .tracks
            .iter()
            .find(|track| track.kind == noflex_transcoder::package::TrackKind::Video)
        else {
            return StatusCode::NOT_FOUND.into_response();
        };
        return playlist(media_playlist(track, &init_name(track), "file"), token);
    }
    if let Some(track) = track_named(&index, name, "audio_", ".m3u8") {
        return playlist(media_playlist(track, &init_name(track), "file"), token);
    }
    if let Some(track) = track_named(&index, name, "init_", ".mp4") {
        let id = track.track_id;
        let head_size = index.head_size;
        let path = path.to_path_buf();
        let init = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let mut file = std::fs::File::open(path)?;
            file.seek(SeekFrom::Start(0))?;
            let mut head = vec![0_u8; usize::try_from(head_size)?];
            file.read_exact(&mut head)?;
            Ok(single_track_init(&head, id)?)
        })
        .await;
        return match init {
            Ok(Ok(bytes)) => ([(header::CONTENT_TYPE, "video/mp4")], bytes).into_response(),
            Ok(Err(error)) => {
                tracing::error!(%error, version = %version.id, "init segment failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            Err(error) => {
                tracing::error!(%error, version = %version.id, "init segment task failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        };
    }
    StatusCode::NOT_FOUND.into_response()
}
