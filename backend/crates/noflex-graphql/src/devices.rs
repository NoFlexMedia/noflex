// SPDX-License-Identifier: AGPL-3.0-only
//
// The device registry behind smart transcoding and downloads.

use std::sync::Arc;

use async_graphql::{Context, Json, Object, SimpleObject};
use noflex_core::{DeviceId, VideoCodec};
use noflex_plugin_host::PluginRegistry;
use noflex_storage::{
    RequestUser, Storage,
    domain::{Device, OptimizedVersion, Work},
    error::StorageError,
};
use noflex_transcoder::ClientCapabilities;
use time::OffsetDateTime;

use crate::{
    access,
    playback_session::{PlaybackSessionService, SubtitleDelivery},
    smart::{LikelyOptions, likely_watched, playable_sources},
    users::authed_principal,
};

const MAX_DEVICE_NAME_CHARS: usize = 100;
/// Bounds the work of one request: titles past this are never the next thing
/// a budget would reach.
const SMART_DOWNLOAD_CANDIDATES: LikelyOptions = LikelyOptions {
    lookahead_episodes: 3,
    recommendations: 10,
};

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

/// One file a device should hold.
#[derive(SimpleObject, Debug, Clone)]
pub struct SmartDownload {
    pub work: Work,
    /// Parts of a multi-file title each get their own entry.
    pub part_index: u32,
    /// HTTP range requests work, so an interrupted download can resume, and a
    /// partial file already plays.
    pub url: String,
    pub size_bytes: u64,
    /// Whether this is a version prepared for the device rather than the
    /// original file.
    pub optimized: bool,
    pub subtitles: Vec<SubtitleDelivery>,
}

#[derive(Default)]
pub struct DevicesQuery;

#[Object]
impl DevicesQuery {
    /// The caller's registered devices.
    async fn my_devices(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Device>> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .devices_for_user(principal.user_id)
            .await
            .map_err(gql_err)
    }

    /// Every optimized version the server knows about, in or out of the queue.
    /// Administrator only.
    async fn optimized_versions(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<OptimizedVersion>> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .list_optimized_versions()
            .await
            .map_err(gql_err)
    }

    /// What the caller's device should hold, most important first and within
    /// its download budget. Empty unless the device has smart downloads on.
    /// Titles still being prepared appear once they're ready, so poll this.
    /// Whether to fetch now, such as only on Wi-Fi, is the client's call.
    async fn smart_downloads(
        &self,
        ctx: &Context<'_>,
        device_id: DeviceId,
    ) -> async_graphql::Result<Vec<SmartDownload>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let device = owned_device(storage, principal.user_id, device_id).await?;
        if !device.smart_downloads {
            return Ok(Vec::new());
        }
        let Ok(service) = ctx.data::<Arc<dyn PlaybackSessionService>>() else {
            return Err(async_graphql::Error::new("playback is not available"));
        };
        let capabilities = serde_json::from_value::<ClientCapabilities>(device.capabilities.0)
            .map_err(|_| async_graphql::Error::new("device capabilities are unreadable"))?;
        let namespace = ctx
            .data::<Arc<PluginRegistry>>()
            .ok()
            .and_then(|registry| registry.embedder_claim())
            .map(|claim| claim.namespace);
        let user = RequestUser {
            user_id: principal.user_id,
            is_admin: principal.is_admin,
        };
        // Boxed for the same reason as the recommendations resolver.
        let likely = Box::pin(likely_watched(
            storage.as_ref(),
            namespace.as_deref(),
            user,
            SMART_DOWNLOAD_CANDIDATES,
            OffsetDateTime::now_utc(),
        ))
        .await
        .map_err(gql_err)?;

        let mut remaining = device.download_budget_bytes.unwrap_or(u64::MAX);
        let mut out = Vec::new();
        'works: for work in likely {
            for media in Box::pin(playable_sources(storage.as_ref(), &work))
                .await
                .map_err(gql_err)?
            {
                let Some(file) = service
                    .smart_download(
                        principal.user_id,
                        principal.session_id,
                        &media,
                        &capabilities,
                    )
                    .await
                    .map_err(gql_err)?
                else {
                    continue;
                };
                if file.size_bytes > remaining {
                    break 'works;
                }
                remaining -= file.size_bytes;
                out.push(SmartDownload {
                    work: work.clone(),
                    part_index: media.part_index,
                    url: file.url,
                    size_bytes: file.size_bytes,
                    optimized: file.optimized,
                    subtitles: file.subtitles,
                });
            }
        }
        Ok(out)
    }
}

async fn owned_device(
    storage: &Arc<dyn Storage>,
    user: noflex_core::UserId,
    id: DeviceId,
) -> async_graphql::Result<Device> {
    storage
        .get_device(id)
        .await
        .map_err(gql_err)?
        .filter(|device| device.user_id == user)
        .ok_or_else(|| async_graphql::Error::new("device not found"))
}

#[derive(Default)]
pub struct DevicesMutation;

#[Object]
impl DevicesMutation {
    /// Register this client, or refresh what it can play. Clients choose
    /// `id` once and keep it across sign-ins; re-registering keeps the
    /// device's settings. An id already registered to someone else fails.
    async fn register_device(
        &self,
        ctx: &Context<'_>,
        id: DeviceId,
        name: String,
        capabilities: ClientCapabilities,
    ) -> async_graphql::Result<Device> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_DEVICE_NAME_CHARS {
            return Err(async_graphql::Error::new(
                "device name must be 1 to 100 characters",
            ));
        }
        let capabilities = serde_json::to_value(capabilities).map_err(gql_err)?;
        let now = OffsetDateTime::now_utc();
        let mut device = match storage.get_device(id).await.map_err(gql_err)? {
            Some(existing) => existing,
            None => Device {
                id,
                user_id: principal.user_id,
                name: String::new(),
                capabilities: Json(serde_json::Value::Null),
                smart_transcode: false,
                max_bitrate_bps: None,
                allowed_video_codecs: Vec::new(),
                smart_downloads: false,
                download_budget_bytes: None,
                downloads_wifi_only: true,
                created_at: now,
                last_seen_at: now,
            },
        };
        device.user_id = principal.user_id;
        device.name = name.to_string();
        device.capabilities = Json(capabilities);
        device.last_seen_at = now;
        store_device(storage, device).await
    }

    /// Change one of the caller's devices; omitted settings stay. Pass
    /// `clearMaxBitrate` or `clearDownloadBudget` to remove a limit.
    #[allow(clippy::too_many_arguments)] // Each parameter is one GraphQL field argument.
    async fn update_device(
        &self,
        ctx: &Context<'_>,
        id: DeviceId,
        smart_transcode: Option<bool>,
        max_bitrate_bps: Option<u64>,
        clear_max_bitrate: Option<bool>,
        allowed_video_codecs: Option<Vec<VideoCodec>>,
        smart_downloads: Option<bool>,
        download_budget_bytes: Option<u64>,
        clear_download_budget: Option<bool>,
        downloads_wifi_only: Option<bool>,
    ) -> async_graphql::Result<Device> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let mut device = owned_device(storage, principal.user_id, id).await?;
        if let Some(value) = smart_transcode {
            device.smart_transcode = value;
        }
        if clear_max_bitrate == Some(true) {
            device.max_bitrate_bps = None;
        }
        if let Some(value) = max_bitrate_bps {
            device.max_bitrate_bps = Some(value);
        }
        if let Some(value) = allowed_video_codecs {
            device.allowed_video_codecs = value;
        }
        if let Some(value) = smart_downloads {
            device.smart_downloads = value;
        }
        if clear_download_budget == Some(true) {
            device.download_budget_bytes = None;
        }
        if let Some(value) = download_budget_bytes {
            device.download_budget_bytes = Some(value);
        }
        if let Some(value) = downloads_wifi_only {
            device.downloads_wifi_only = value;
        }
        store_device(storage, device).await
    }

    /// Forget one of the caller's devices. Returns whether there was one.
    async fn remove_device(&self, ctx: &Context<'_>, id: DeviceId) -> async_graphql::Result<bool> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .delete_device(principal.user_id, id)
            .await
            .map_err(gql_err)
    }
}

async fn store_device(storage: &Arc<dyn Storage>, device: Device) -> async_graphql::Result<Device> {
    match storage.upsert_device(&device).await {
        Ok(()) => Ok(device),
        // Another user's id; indistinguishable from any other refusal.
        Err(StorageError::NotFound) => Err(async_graphql::Error::new("device not found")),
        Err(error) => Err(gql_err(error)),
    }
}
