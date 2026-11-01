// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::{Context, InputObject, Object, connection::Connection};
use noflex_core::{
    ChannelId, EpgProgramId, LiveTvSourceId, RecordingId, RecordingScheduleId, UserId,
};
use noflex_storage::{
    EpgProgram, LiveTvChannel, LiveTvSource, Recording, RecordingSchedule, RecordingScheduleStatus,
    Storage,
    query::{ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX},
};
use time::OffsetDateTime;

use crate::{PlaybackSession, paging};

/// Failure from a [`LiveTvService`] call. The `Display` text is the
/// stable client-facing message; [`LiveTvServiceError::Internal`] carries its
/// cause for server-side logging only.
#[derive(Debug, thiserror::Error)]
pub enum LiveTvServiceError {
    /// The named resource doesn't exist or isn't visible to the caller.
    #[error("{0} not found")]
    NotFound(&'static str),
    /// The recording is scheduled or running and can't be deleted.
    #[error("recording is active")]
    RecordingActive,
    /// The caller's concurrent-stream allowance is exhausted.
    #[error("concurrent stream limit reached")]
    LimitReached,
    /// Anything else; never shown to clients verbatim.
    #[error("live TV operation failed")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl LiveTvServiceError {
    /// Wrap an arbitrary cause as [`LiveTvServiceError::Internal`].
    pub fn internal(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Internal(error.into())
    }
}

/// Live TV runtime operations the GraphQL layer can't perform with
/// storage alone. `owner` is always the authenticated caller; the
/// implementor decides whether it may act on another user's rows.
#[async_trait::async_trait]
pub trait LiveTvService: Send + Sync {
    async fn refresh_source(&self, source: LiveTvSourceId) -> Result<(), LiveTvServiceError>;
    async fn start_channel(
        &self,
        owner: UserId,
        channel: ChannelId,
    ) -> Result<PlaybackSession, LiveTvServiceError>;
    async fn cancel_schedule(
        &self,
        owner: UserId,
        schedule: RecordingScheduleId,
    ) -> Result<RecordingSchedule, LiveTvServiceError>;
    async fn delete_recording(
        &self,
        owner: UserId,
        recording: RecordingId,
        delete_file: bool,
    ) -> Result<Recording, LiveTvServiceError>;
}

/// Most channels one guide request may name.
const MAX_GUIDE_CHANNELS: usize = 200;
/// Widest guide window one request may span.
const MAX_GUIDE_SPAN: time::Duration = time::Duration::days(14);
/// Longest recording window (excluding padding).
const MAX_RECORDING_SPAN: time::Duration = time::Duration::hours(12);
/// Largest padding on either side of a recording, in seconds.
const MAX_PADDING_SECS: u32 = 60 * 60;
/// Most schedules one user may hold in `Scheduled` state.
const MAX_ACTIVE_SCHEDULES: usize = 100;

/// Default guide page: a few hours across a screenful of channels.
const GUIDE_PAGE_DEFAULT: u32 = 500;
/// Largest guide page.
const GUIDE_PAGE_MAX: u32 = 2000;

#[derive(InputObject)]
pub struct CreateLiveTvSourceInput {
    pub name: String,
    pub playlist_url: String,
    pub epg_url: Option<String>,
    #[graphql(default = true)]
    pub enabled: bool,
}
#[derive(InputObject)]
pub struct UpdateLiveTvSourceInput {
    pub name: Option<String>,
    pub playlist_url: Option<String>,
    pub epg_url: Option<String>,
    pub clear_epg_url: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Default)]
pub struct LiveTvQuery;
#[Object]
impl LiveTvQuery {
    async fn live_tv_sources(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<LiveTvSource>> {
        crate::access::require_admin(ctx)?;
        storage(ctx)?.list_live_tv_sources().await.map_err(gql)
    }
    /// Enabled channels, optionally of one source, in id order.
    /// `first` defaults to 50, max 100.
    #[graphql(
        complexity = "paging::cost(first, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn live_tv_channels(
        &self,
        ctx: &Context<'_>,
        source_id: Option<LiveTvSourceId>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, LiveTvChannel>> {
        let window = paging::Window::new(first, after, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX)?;
        crate::access::request_user(ctx)?;
        let resumed = window.after.is_some();
        let page = storage(ctx)?
            .list_live_tv_channels(source_id, window.first, window.after)
            .await
            .map_err(gql)?;
        Ok(paging::from_item_page(page, resumed))
    }
    /// Programmes on `channelIds` overlapping `[start, end)`, ordered by
    /// start time. At most 200 channels and a 14-day window per request;
    /// `first` defaults to 500, max 2000.
    // Programmes are leaf rows from one indexed query, already bounded by
    // the channel and window caps, so they're priced per ten rows.
    #[graphql(
        complexity = "paging::cost(first, GUIDE_PAGE_DEFAULT, GUIDE_PAGE_MAX, child_complexity).div_ceil(10)"
    )]
    async fn live_tv_guide(
        &self,
        ctx: &Context<'_>,
        channel_ids: Vec<ChannelId>,
        start: OffsetDateTime,
        end: OffsetDateTime,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, EpgProgram>> {
        let window = paging::Window::new(first, after, GUIDE_PAGE_DEFAULT, GUIDE_PAGE_MAX)?;
        crate::access::request_user(ctx)?;
        if end <= start {
            return Err(async_graphql::Error::new("end must be after start"));
        }
        if end - start > MAX_GUIDE_SPAN {
            return Err(async_graphql::Error::new("guide window exceeds 14 days"));
        }
        if channel_ids.len() > MAX_GUIDE_CHANNELS {
            return Err(async_graphql::Error::new(format!(
                "at most {MAX_GUIDE_CHANNELS} channels per guide request"
            )));
        }
        if channel_ids.is_empty() {
            return Ok(paging::empty());
        }
        let resumed = window.after.is_some();
        let page = storage(ctx)?
            .list_live_tv_guide(&channel_ids, start, end, window.first, window.after)
            .await
            .map_err(gql)?;
        Ok(paging::from_item_page(page, resumed))
    }
    /// A user's recording schedules, newest first. `owner` defaults to
    /// the caller; naming anyone else requires admin. `first` defaults
    /// to 50, max 100.
    #[graphql(
        complexity = "paging::cost(first, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn recording_schedules(
        &self,
        ctx: &Context<'_>,
        owner: Option<UserId>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, RecordingSchedule>> {
        let window = paging::Window::new(first, after, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX)?;
        let owner = effective_owner(ctx, owner)?;
        let resumed = window.after.is_some();
        let page = storage(ctx)?
            .list_recording_schedules(owner, window.first, window.after)
            .await
            .map_err(gql)?;
        Ok(paging::from_item_page(page, resumed))
    }
    /// A user's recordings, newest first. `owner` defaults to the
    /// caller; naming anyone else requires admin. `first` defaults to
    /// 50, max 100.
    #[graphql(
        complexity = "paging::cost(first, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn recordings(
        &self,
        ctx: &Context<'_>,
        owner: Option<UserId>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Recording>> {
        let window = paging::Window::new(first, after, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX)?;
        let owner = effective_owner(ctx, owner)?;
        let resumed = window.after.is_some();
        let page = storage(ctx)?
            .list_recordings(owner, window.first, window.after)
            .await
            .map_err(gql)?;
        Ok(paging::from_item_page(page, resumed))
    }
}

#[derive(Default)]
pub struct LiveTvMutation;
#[Object]
impl LiveTvMutation {
    async fn create_live_tv_source(
        &self,
        ctx: &Context<'_>,
        input: CreateLiveTvSourceInput,
    ) -> async_graphql::Result<LiveTvSource> {
        crate::access::require_admin(ctx)?;
        validate_source_url(&input.playlist_url)?;
        if let Some(url) = input.epg_url.as_deref() {
            validate_source_url(url)?
        }
        let source = LiveTvSource {
            id: LiveTvSourceId::new(),
            name: input.name,
            enabled: input.enabled,
            last_refresh_at: None,
            last_error: None,
            playlist_url: input.playlist_url,
            epg_url: input.epg_url,
        };
        storage(ctx)?
            .upsert_live_tv_source(&source)
            .await
            .map_err(gql)?;
        Ok(source)
    }
    async fn update_live_tv_source(
        &self,
        ctx: &Context<'_>,
        id: LiveTvSourceId,
        input: UpdateLiveTvSourceInput,
    ) -> async_graphql::Result<LiveTvSource> {
        crate::access::require_admin(ctx)?;
        let store = storage(ctx)?;
        let mut source = store
            .get_live_tv_source(id)
            .await
            .map_err(gql)?
            .ok_or_else(|| async_graphql::Error::new("source not found"))?;
        if let Some(name) = input.name {
            source.name = name
        }
        if let Some(url) = input.playlist_url {
            validate_source_url(&url)?;
            source.playlist_url = url
        }
        if input.clear_epg_url.unwrap_or(false) {
            source.epg_url = None
        } else if let Some(url) = input.epg_url {
            validate_source_url(&url)?;
            source.epg_url = Some(url)
        }
        if let Some(enabled) = input.enabled {
            source.enabled = enabled
        }
        store.upsert_live_tv_source(&source).await.map_err(gql)?;
        Ok(source)
    }
    async fn delete_live_tv_source(
        &self,
        ctx: &Context<'_>,
        id: LiveTvSourceId,
    ) -> async_graphql::Result<bool> {
        crate::access::require_admin(ctx)?;
        let store = storage(ctx)?;
        if store.get_live_tv_source(id).await.map_err(gql)?.is_none() {
            return Ok(false);
        }
        store.delete_live_tv_source(id).await.map_err(gql)?;
        Ok(true)
    }
    async fn refresh_live_tv_source(
        &self,
        ctx: &Context<'_>,
        id: LiveTvSourceId,
    ) -> async_graphql::Result<LiveTvSource> {
        crate::access::require_admin(ctx)?;
        service(ctx)?
            .refresh_source(id)
            .await
            .map_err(service_error)?;
        storage(ctx)?
            .get_live_tv_source(id)
            .await
            .map_err(gql)?
            .ok_or_else(|| async_graphql::Error::new("source not found"))
    }
    async fn start_live_tv_channel(
        &self,
        ctx: &Context<'_>,
        channel_id: ChannelId,
    ) -> async_graphql::Result<PlaybackSession> {
        let user = crate::access::user(ctx)?;
        service(ctx)?
            .start_channel(user, channel_id)
            .await
            .map_err(service_error)
    }
    // The GraphQL contract keeps schedule fields as named arguments rather than an opaque input.
    #[allow(clippy::too_many_arguments)]
    async fn schedule_recording(
        &self,
        ctx: &Context<'_>,
        channel_id: ChannelId,
        program_id: Option<EpgProgramId>,
        start_at: OffsetDateTime,
        end_at: OffsetDateTime,
        #[graphql(default = 0)] padding_before_secs: i32,
        #[graphql(default = 0)] padding_after_secs: i32,
    ) -> async_graphql::Result<RecordingSchedule> {
        let owner = crate::access::user(ctx)?;
        let now = OffsetDateTime::now_utc();
        if end_at <= start_at {
            return Err(async_graphql::Error::new("end must be after start"));
        }
        if end_at <= now {
            return Err(async_graphql::Error::new("end must be in the future"));
        }
        if end_at - start_at > MAX_RECORDING_SPAN {
            return Err(async_graphql::Error::new("recording exceeds 12 hours"));
        }
        let before = u32::try_from(padding_before_secs)
            .map_err(|_| async_graphql::Error::new("padding must be non-negative"))?;
        let after = u32::try_from(padding_after_secs)
            .map_err(|_| async_graphql::Error::new("padding must be non-negative"))?;
        if before > MAX_PADDING_SECS || after > MAX_PADDING_SECS {
            return Err(async_graphql::Error::new("padding exceeds 1 hour"));
        }
        let store = storage(ctx)?;
        let active = active_schedule_count(&store, owner).await?;
        if active >= MAX_ACTIVE_SCHEDULES {
            return Err(async_graphql::Error::new(format!(
                "at most {MAX_ACTIVE_SCHEDULES} pending recording schedules per user"
            )));
        }
        let channel = store
            .get_live_tv_channel(channel_id)
            .await
            .map_err(gql)?
            .filter(|c| c.enabled)
            .ok_or_else(|| async_graphql::Error::new("channel not found"))?;
        if let Some(program) = program_id {
            let p = store
                .get_epg_program(program)
                .await
                .map_err(gql)?
                .ok_or_else(|| async_graphql::Error::new("program not found"))?;
            if p.channel_id != channel.id {
                return Err(async_graphql::Error::new(
                    "program does not belong to channel",
                ));
            }
        }
        let schedule = RecordingSchedule {
            id: RecordingScheduleId::new(),
            owner_id: owner,
            channel_id,
            program_id,
            start_at,
            end_at,
            padding_before_secs: before,
            padding_after_secs: after,
            status: RecordingScheduleStatus::Scheduled,
            created_at: now,
            updated_at: now,
        };
        store
            .create_recording_schedule(&schedule)
            .await
            .map_err(gql)?;
        Ok(schedule)
    }
    async fn cancel_recording_schedule(
        &self,
        ctx: &Context<'_>,
        id: RecordingScheduleId,
    ) -> async_graphql::Result<RecordingSchedule> {
        let owner = crate::access::user(ctx)?;
        service(ctx)?
            .cancel_schedule(owner, id)
            .await
            .map_err(service_error)
    }
    async fn delete_recording(
        &self,
        ctx: &Context<'_>,
        id: RecordingId,
        delete_file: bool,
    ) -> async_graphql::Result<Recording> {
        let owner = crate::access::user(ctx)?;
        service(ctx)?
            .delete_recording(owner, id, delete_file)
            .await
            .map_err(service_error)
    }
}

fn storage(ctx: &Context<'_>) -> async_graphql::Result<Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}
fn service(ctx: &Context<'_>) -> async_graphql::Result<Arc<dyn LiveTvService>> {
    ctx.data::<Arc<dyn LiveTvService>>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("live TV unavailable"))
}
fn effective_owner(ctx: &Context<'_>, requested: Option<UserId>) -> async_graphql::Result<UserId> {
    let user = crate::access::request_user(ctx)?;
    match requested {
        Some(owner) if owner != user.user_id && !user.is_admin => {
            Err(async_graphql::Error::new("recordings not found"))
        }
        Some(owner) => Ok(owner),
        None => Ok(user.user_id),
    }
}
fn validate_source_url(value: &str) -> async_graphql::Result<()> {
    let url = url::Url::parse(value)
        .map_err(|_| async_graphql::Error::new("source URL must be absolute"))?;
    if matches!(url.scheme(), "http" | "https") {
        Ok(())
    } else {
        Err(async_graphql::Error::new(
            "source URL must use http or https",
        ))
    }
}
/// Pending schedules owned by `owner`, counted page by page up to the cap.
async fn active_schedule_count(
    store: &Arc<dyn Storage>,
    owner: UserId,
) -> async_graphql::Result<usize> {
    let mut count = 0;
    let mut cursor = None;
    loop {
        let page = store
            .list_recording_schedules(owner, ITEM_PAGE_MAX, cursor)
            .await
            .map_err(gql)?;
        count += page
            .items
            .iter()
            .filter(|schedule| schedule.status == RecordingScheduleStatus::Scheduled)
            .count();
        match page.next_cursor() {
            Some(next) if count < MAX_ACTIVE_SCHEDULES => cursor = Some(next),
            _ => return Ok(count),
        }
    }
}

fn service_error(error: LiveTvServiceError) -> async_graphql::Error {
    if let LiveTvServiceError::Internal(source) = &error {
        tracing::error!(error = %source, "live TV service failure");
    }
    async_graphql::Error::new(error.to_string())
}

fn gql(error: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(error.to_string())
}
