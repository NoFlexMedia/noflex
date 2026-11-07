// SPDX-License-Identifier: AGPL-3.0-only
//
// The device registry behind smart transcoding and downloads.

use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions, Json, Object, SimpleObject, connection::Connection};
use noflex_core::{DeviceId, MediaFileId, OptimizedVersionId, TonemapCurve, VideoCodec};
use noflex_plugin_host::PluginRegistry;
use noflex_storage::{
    RequestUser, Storage,
    domain::{
        Device, OptimizedVersion, OptimizedVersionStats, OptimizedVersionStatus,
        SmartTranscodingSettings, Work,
    },
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

/// What smart transcoding does when no administrator has said otherwise.
#[derive(Debug, Clone)]
pub struct SmartTranscodingDefaults {
    pub enabled: bool,
    pub directory: String,
    pub max_storage_gb: u64,
    pub max_version_gb: Option<u64>,
}

/// Where smart transcoding writes and how much it may use.
#[derive(SimpleObject, Debug, Clone)]
pub struct SmartTranscoding {
    /// Off in the server's configuration, which only that can change.
    pub enabled: bool,
    pub directory: String,
    pub max_storage_gb: u64,
    /// Skip versions estimated larger than this; null has no limit.
    pub max_version_gb: Option<u64>,
    /// What each falls back to when cleared.
    pub default_directory: String,
    pub default_max_storage_gb: u64,
    pub default_max_version_gb: Option<u64>,
}

async fn tonemap_curve(ctx: &Context<'_>) -> async_graphql::Result<TonemapCurve> {
    let chosen = storage(ctx)?.tonemap_curve().await.map_err(gql_err)?;
    Ok(chosen
        .or_else(|| ctx.data_opt::<TonemapCurve>().copied())
        .unwrap_or_default())
}

async fn smart_transcoding(ctx: &Context<'_>) -> async_graphql::Result<SmartTranscoding> {
    let defaults = ctx
        .data::<SmartTranscodingDefaults>()
        .map_err(|_| async_graphql::Error::new("smart transcoding not configured"))?;
    let settings = storage(ctx)?
        .smart_transcoding_settings()
        .await
        .map_err(gql_err)?;
    Ok(SmartTranscoding {
        enabled: defaults.enabled,
        directory: settings.dir.unwrap_or_else(|| defaults.directory.clone()),
        max_storage_gb: settings.max_storage_gb.unwrap_or(defaults.max_storage_gb),
        max_version_gb: settings.max_version_gb.or(defaults.max_version_gb),
        default_directory: defaults.directory.clone(),
        default_max_storage_gb: defaults.max_storage_gb,
        default_max_version_gb: defaults.max_version_gb,
    })
}

/// A folder smart transcoding can use: absolute, and writable now rather than
/// failing every job later.
async fn check_directory(directory: &str) -> async_graphql::Result<()> {
    let path = std::path::Path::new(directory);
    if !path.is_absolute() {
        return Err(async_graphql::Error::new(
            "The folder must be an absolute path",
        ));
    }
    let probe = path.join(format!(".noflex-write-check-{}", uuid::Uuid::now_v7()));
    let writable = async {
        tokio::fs::create_dir_all(path).await?;
        tokio::fs::write(&probe, b"").await?;
        tokio::fs::remove_file(&probe).await
    };
    writable.await.map_err(|error| {
        tracing::warn!(%error, directory, "smart transcoding folder rejected");
        async_graphql::Error::new("The server can't write to that folder")
    })
}
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

pub(crate) fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
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

    /// A viewer's registered devices. Administrator only.
    async fn user_devices(
        &self,
        ctx: &Context<'_>,
        user_id: noflex_core::UserId,
    ) -> async_graphql::Result<Vec<Device>> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .devices_for_user(user_id)
            .await
            .map_err(gql_err)
    }

    /// The curve HDR is tone-mapped with, for streams starting from now.
    /// Administrator only.
    async fn tonemap_curve(&self, ctx: &Context<'_>) -> async_graphql::Result<TonemapCurve> {
        access::require_admin(ctx)?;
        tonemap_curve(ctx).await
    }

    /// Where smart transcoding writes and how much it may use. Administrator
    /// only.
    async fn smart_transcoding(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<SmartTranscoding> {
        access::require_admin(ctx)?;
        smart_transcoding(ctx).await
    }

    /// Optimized versions, what needs a look first and then the newest. `first`
    /// defaults to 25, max 100. Administrator only.
    #[graphql(complexity = "crate::paging::cost(first, 25, 100, child_complexity)")]
    async fn optimized_versions(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, OptimizedVersion>> {
        access::require_admin(ctx)?;
        let window = crate::paging::Window::new(first, after, 25, 100)?;
        let resumed = window.after.is_some();
        let page = storage(ctx)?
            .optimized_versions_page(None, window.first, window.after)
            .await
            .map_err(gql_err)?;
        Ok(crate::paging::from_item_page(page, resumed))
    }

    /// One version, for looking closer. Administrator only.
    async fn optimized_version(
        &self,
        ctx: &Context<'_>,
        id: OptimizedVersionId,
    ) -> async_graphql::Result<Option<OptimizedVersion>> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .get_optimized_version(id)
            .await
            .map_err(gql_err)
    }

    /// How many versions there are of each kind. Administrator only.
    async fn optimized_version_stats(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<OptimizedVersionStats> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .optimized_version_stats()
            .await
            .map_err(gql_err)
    }

    /// What's ready to play on this server of the titles the caller can see,
    /// the newest 50. A version serves any device that can play it, whoever or
    /// whatever asked for it.
    async fn prepared_versions(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<OptimizedVersion>> {
        const SHOWN: usize = 50;
        const WALK: u32 = 100;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let mut prepared = Vec::new();
        let mut after = None;
        loop {
            let page = storage
                .optimized_versions_page(Some(OptimizedVersionStatus::Ready), WALK, after)
                .await
                .map_err(gql_err)?;
            for version in page.items {
                let Some(work) = version
                    .source_work(storage.as_ref())
                    .await
                    .map_err(gql_err)?
                else {
                    continue;
                };
                if noflex_storage::access::user_can_access_work(storage.as_ref(), user, work.id)
                    .await
                    .map_err(gql_err)?
                {
                    prepared.push(version);
                    if prepared.len() == SHOWN {
                        return Ok(prepared);
                    }
                }
            }
            if !page.has_next {
                return Ok(prepared);
            }
            after = page.end_cursor;
        }
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
        // Boxed because every resolver's future is laid out inside the merged
        // root's, and this one is large enough to overflow a worker's stack.
        Box::pin(smart_downloads(ctx, device_id)).await
    }

    /// One media file as the caller's device should hold it, for a title the
    /// viewer chose to keep: the file itself when it plays directly, otherwise
    /// a ready optimized version. Null while neither exists; an export job can
    /// stand in. Answers regardless of the device's smart downloads setting.
    async fn device_download(
        &self,
        ctx: &Context<'_>,
        device_id: DeviceId,
        media_file_id: MediaFileId,
    ) -> async_graphql::Result<Option<SmartDownload>> {
        Box::pin(device_download(ctx, device_id, media_file_id)).await
    }
}

async fn smart_downloads(
    ctx: &Context<'_>,
    device_id: DeviceId,
) -> async_graphql::Result<Vec<SmartDownload>> {
    let principal = authed_principal(ctx)?;
    let storage = storage(ctx)?;
    let device = owned_device(storage, principal.user_id, device_id).await?;
    // Empty rather than an error: this is polled, and the answer is that the
    // server wants nothing kept here.
    if !device.smart_downloads || !storage.downloads_allowed().await.map_err(gql_err)? {
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
    let likely = likely_watched(
        storage.as_ref(),
        namespace.as_deref(),
        user,
        SMART_DOWNLOAD_CANDIDATES,
        OffsetDateTime::now_utc(),
    )
    .await
    .map_err(gql_err)?;

    let mut remaining = device.download_budget_bytes.unwrap_or(u64::MAX);
    let mut out = Vec::new();
    'works: for work in likely {
        for media in playable_sources(storage.as_ref(), &work)
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

async fn device_download(
    ctx: &Context<'_>,
    device_id: DeviceId,
    media_file_id: MediaFileId,
) -> async_graphql::Result<Option<SmartDownload>> {
    let principal = authed_principal(ctx)?;
    let storage = storage(ctx)?;
    let device = owned_device(storage, principal.user_id, device_id).await?;
    require_downloads_allowed(storage).await?;
    let Ok(service) = ctx.data::<Arc<dyn PlaybackSessionService>>() else {
        return Err(async_graphql::Error::new("playback is not available"));
    };
    let user = RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    };
    let not_found = || async_graphql::Error::new("media file not found");
    if !noflex_storage::access::user_can_access_media_file(storage.as_ref(), user, media_file_id)
        .await
        .map_err(gql_err)?
    {
        return Err(not_found());
    }
    let media = storage
        .media_file_by_id(media_file_id)
        .await
        .map_err(gql_err)?
        .ok_or_else(not_found)?;
    let work = storage
        .work_for_media_file(media_file_id)
        .await
        .map_err(gql_err)?
        .ok_or_else(not_found)?;
    let capabilities = serde_json::from_value::<ClientCapabilities>(device.capabilities.0)
        .map_err(|_| async_graphql::Error::new("device capabilities are unreadable"))?;
    let file = service
        .smart_download(
            principal.user_id,
            principal.session_id,
            &media,
            &capabilities,
        )
        .await
        .map_err(gql_err)?;
    Ok(file.map(|file| SmartDownload {
        work,
        part_index: media.part_index,
        url: file.url,
        size_bytes: file.size_bytes,
        optimized: file.optimized,
        subtitles: file.subtitles,
    }))
}

/// Refuses a new download on a server whose administrator has turned them off.
pub(crate) async fn require_downloads_allowed(
    storage: &Arc<dyn Storage>,
) -> async_graphql::Result<()> {
    if storage.downloads_allowed().await.map_err(gql_err)? {
        return Ok(());
    }
    Err(
        async_graphql::Error::new("This server doesn’t allow downloads.")
            .extend_with(|_, ext| ext.set("code", "DOWNLOADS_NOT_ALLOWED")),
    )
}

pub(crate) async fn owned_device(
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
    /// `client` is the app it's running, as at `login`; omitted, it stays.
    async fn register_device(
        &self,
        ctx: &Context<'_>,
        id: DeviceId,
        name: String,
        client: Option<String>,
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
                client: None,
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
        if let Some(client) = client.filter(|client| !client.trim().is_empty()) {
            if client.chars().count() > MAX_DEVICE_NAME_CHARS {
                return Err(async_graphql::Error::new(
                    "client name must be at most 100 characters",
                ));
            }
            device.client = Some(client);
        }
        device.capabilities = Json(capabilities);
        device.last_seen_at = now;
        store_device(storage, device).await
    }

    /// Choose the curve HDR is tone-mapped with; null leaves it to the server's
    /// configuration. Streams already running keep theirs. Administrator only.
    async fn set_tonemap_curve(
        &self,
        ctx: &Context<'_>,
        curve: Option<TonemapCurve>,
    ) -> async_graphql::Result<TonemapCurve> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .set_tonemap_curve(curve)
            .await
            .map_err(gql_err)?;
        tonemap_curve(ctx).await
    }

    /// Remove a prepared copy, or stop one being prepared, and free its disk. It's
    /// prepared again if something still wants it: its `marks` can be taken back
    /// first with `unmarkViewerForSmartTranscoding`. Administrator only.
    async fn remove_optimized_version(
        &self,
        ctx: &Context<'_>,
        id: OptimizedVersionId,
    ) -> async_graphql::Result<Option<OptimizedVersion>> {
        access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let Some(version) = storage.get_optimized_version(id).await.map_err(gql_err)? else {
            return Ok(None);
        };
        // Here and not left to the worker's orphan sweep, which only looks in the
        // current folder: a version can outlive a change of folder.
        if let Some(path) = &version.path
            && let Err(error) = tokio::fs::remove_file(path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, version = %id, "optimized file not removed");
        }
        storage
            .delete_optimized_version(id)
            .await
            .map_err(gql_err)?;
        // A running job finds its row gone and stops.
        if let Some(wake) = ctx.data_opt::<Arc<tokio::sync::Notify>>() {
            wake.notify_one();
        }
        Ok(Some(version))
    }

    /// Whether the server prepares things for a viewer's device on its own, for an
    /// administrator clearing what keeps a version wanted. Off also ends smart
    /// downloads, which ask for the same predictions; a viewer can turn either back
    /// on, and marks naming the device stand. Administrator only.
    async fn set_viewer_device_smart_transcode(
        &self,
        ctx: &Context<'_>,
        id: DeviceId,
        enabled: bool,
    ) -> async_graphql::Result<Device> {
        access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let mut device = storage
            .get_device(id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("device not found"))?;
        device.smart_transcode = enabled;
        if !enabled {
            device.smart_downloads = false;
        }
        let device = store_device(storage, device).await?;
        if let Some(wake) = ctx.data_opt::<Arc<tokio::sync::Notify>>() {
            wake.notify_one();
        }
        Ok(device)
    }

    /// Set where smart transcoding writes and how much it may use. Null leaves
    /// a setting to the server's configuration. Versions already prepared stay
    /// where they are and keep playing. Administrator only.
    async fn update_smart_transcoding(
        &self,
        ctx: &Context<'_>,
        directory: Option<String>,
        max_storage_gb: Option<u64>,
        max_version_gb: Option<u64>,
    ) -> async_graphql::Result<SmartTranscoding> {
        access::require_admin(ctx)?;
        let directory = directory.filter(|directory| !directory.trim().is_empty());
        if let Some(directory) = &directory {
            check_directory(directory).await?;
        }
        if max_version_gb == Some(0) {
            return Err(async_graphql::Error::new(
                "A version can't be limited to 0 GB; clear the limit instead",
            ));
        }
        storage(ctx)?
            .set_smart_transcoding_settings(&SmartTranscodingSettings {
                dir: directory,
                max_storage_gb,
                max_version_gb,
            })
            .await
            .map_err(gql_err)?;
        smart_transcoding(ctx).await
    }

    /// Whether this server offers its media for keeping on a device, through
    /// `deviceDownload`, `smartDownloads` and export jobs. Streaming is unaffected,
    /// and copies already on a device stay there. Administrator only.
    async fn set_downloads_allowed(
        &self,
        ctx: &Context<'_>,
        allowed: bool,
    ) -> async_graphql::Result<bool> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .set_downloads_allowed(allowed)
            .await
            .map_err(gql_err)?;
        Ok(allowed)
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

    /// Forget one of the caller's devices. Returns it, or null when they had
    /// no such device.
    async fn remove_device(
        &self,
        ctx: &Context<'_>,
        id: DeviceId,
    ) -> async_graphql::Result<Option<Device>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let device = storage
            .get_device(id)
            .await
            .map_err(gql_err)?
            .filter(|device| device.user_id == principal.user_id);
        let Some(device) = device else {
            return Ok(None);
        };
        let removed = storage
            .delete_device(principal.user_id, id)
            .await
            .map_err(gql_err)?;
        Ok(removed.then_some(device))
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
