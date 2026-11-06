// SPDX-License-Identifier: AGPL-3.0-only
//
// Viewers marking titles they're going to watch: a hard signal to smart
// transcoding, ahead of anything it guesses at. A mark is for all their devices
// with smart transcoding on, or for one device whatever its setting.

use std::sync::Arc;

use async_graphql::{Context, Enum, Object, SimpleObject};
use noflex_core::{DeviceId, WorkId, WorkKind};
use noflex_storage::{
    RequestUser, Storage,
    domain::{Device, OptimizedVersionStatus, SmartTranscodeMark, SmartTranscodeOutcome, Work},
};
use time::OffsetDateTime;
use tokio::sync::Notify;

use crate::{devices::SmartTranscodingDefaults, users::authed_principal};

const GIB: u64 = 1024 * 1024 * 1024;

/// How far along a marked title is. The planner works these out for the devices
/// the mark is for; `READY` means no device it's for still needs anything.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartTranscodeState {
    /// In the queue. The server prepares things while nobody is watching.
    Waiting,
    Preparing,
    Ready,
    /// The devices it's for already play the original.
    NotNeeded,
    /// Larger than the administrator allows a version to be.
    TooLarge,
    /// Marked for the viewer's devices, and none has smart transcoding on.
    NoDevices,
    /// Waiting, with the server's space for versions used up by other marks.
    NoSpace,
    Failed,
    /// Smart transcoding is off in the server's configuration.
    Disabled,
}

/// A title the viewer marked for smart transcoding.
#[derive(SimpleObject, Debug, Clone)]
#[graphql(name = "SmartTranscodeMark")]
pub struct SmartMark {
    pub work: Work,
    /// The one device this is for. Null is all of the viewer's devices with
    /// smart transcoding on.
    pub device: Option<Device>,
    pub marked_at: OffsetDateTime,
    pub state: SmartTranscodeState,
    /// 0.0 to 1.0 across the versions it stands for.
    pub progress: f32,
}

fn storage<'a>(ctx: &'a Context<'_>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(error: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(error.to_string())
}

/// `true` when the server's configuration has turned smart transcoding off. A
/// schema without those defaults is a test's, and treated as on.
fn disabled(ctx: &Context<'_>) -> bool {
    ctx.data_opt::<SmartTranscodingDefaults>()
        .is_some_and(|defaults| !defaults.enabled)
}

/// Whether versions already fill the space the server allows them.
async fn out_of_space(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
) -> async_graphql::Result<bool> {
    let Some(defaults) = ctx.data_opt::<SmartTranscodingDefaults>() else {
        return Ok(false);
    };
    let quota = storage
        .smart_transcoding_settings()
        .await
        .map_err(gql_err)?
        .max_storage_gb
        .unwrap_or(defaults.max_storage_gb)
        .saturating_mul(GIB);
    let used = storage
        .optimized_version_stats()
        .await
        .map_err(gql_err)?
        .ready_bytes;
    Ok(used >= quota)
}

async fn describe(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    mark: SmartTranscodeMark,
    work: Work,
) -> async_graphql::Result<SmartMark> {
    let device = match mark.device_id {
        Some(id) => storage.get_device(id).await.map_err(gql_err)?,
        None => None,
    };
    let (state, progress) = state_of(ctx, storage, &mark).await?;
    Ok(SmartMark {
        work,
        device,
        marked_at: mark.created_at,
        state,
        progress,
    })
}

async fn state_of(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    mark: &SmartTranscodeMark,
) -> async_graphql::Result<(SmartTranscodeState, f32)> {
    if disabled(ctx) {
        return Ok((SmartTranscodeState::Disabled, 0.0));
    }
    match mark.outcome {
        None => return Ok((SmartTranscodeState::Waiting, 0.0)),
        Some(SmartTranscodeOutcome::NotNeeded) => {
            return Ok((SmartTranscodeState::NotNeeded, 1.0));
        }
        Some(SmartTranscodeOutcome::TooLarge) => return Ok((SmartTranscodeState::TooLarge, 0.0)),
        Some(SmartTranscodeOutcome::NoDevices) => return Ok((SmartTranscodeState::NoDevices, 0.0)),
        Some(SmartTranscodeOutcome::Planned) => {}
    }
    let mut versions = Vec::new();
    for media in &mark.media_file_ids {
        versions.extend(
            storage
                .optimized_versions_for_media(*media)
                .await
                .map_err(gql_err)?,
        );
    }
    if versions.is_empty() {
        return Ok((SmartTranscodeState::Waiting, 0.0));
    }
    let done = |status| versions.iter().any(|version| version.status == status);
    let progress = versions
        .iter()
        .map(|version| match version.status {
            OptimizedVersionStatus::Ready => 1.0,
            OptimizedVersionStatus::Running => version.progress,
            _ => 0.0,
        })
        .sum::<f32>()
        / versions.len() as f32;
    let state = if done(OptimizedVersionStatus::Running) {
        SmartTranscodeState::Preparing
    } else if done(OptimizedVersionStatus::Queued) {
        if out_of_space(ctx, storage).await? {
            SmartTranscodeState::NoSpace
        } else {
            SmartTranscodeState::Waiting
        }
    } else if done(OptimizedVersionStatus::Failed) {
        SmartTranscodeState::Failed
    } else {
        SmartTranscodeState::Ready
    };
    Ok((state, progress))
}

fn wake(ctx: &Context<'_>) {
    if let Some(wake) = ctx.data_opt::<Arc<Notify>>() {
        wake.notify_one();
    }
}

/// `device`, when it's one of the caller's. The same answer for a device of
/// someone else's as for one that doesn't exist.
async fn check_device(
    storage: &Arc<dyn Storage>,
    user: noflex_core::UserId,
    device: Option<DeviceId>,
) -> async_graphql::Result<()> {
    let Some(id) = device else {
        return Ok(());
    };
    match storage.get_device(id).await.map_err(gql_err)? {
        Some(found) if found.user_id == user => Ok(()),
        _ => Err(async_graphql::Error::new("device not found")),
    }
}

#[derive(Default)]
pub struct SmartMarksQuery;

#[Object]
impl SmartMarksQuery {
    /// The caller's marks, the next to be prepared first; only `workId`'s if
    /// given. Titles they can no longer see are left out.
    async fn my_smart_transcode_marks(
        &self,
        ctx: &Context<'_>,
        work_id: Option<WorkId>,
    ) -> async_graphql::Result<Vec<SmartMark>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let user = RequestUser {
            user_id: principal.user_id,
            is_admin: principal.is_admin,
        };
        let mut out = Vec::new();
        for mark in storage
            .smart_transcode_marks_for_user(principal.user_id)
            .await
            .map_err(gql_err)?
        {
            if work_id.is_some_and(|id| id != mark.work_id) {
                continue;
            }
            if !noflex_storage::access::user_can_access_work(storage.as_ref(), user, mark.work_id)
                .await
                .map_err(gql_err)?
            {
                continue;
            }
            let Some(work) = storage.get_work(mark.work_id).await.map_err(gql_err)? else {
                continue;
            };
            out.push(describe(ctx, storage, mark, work).await?);
        }
        Ok(out)
    }
}

#[derive(Default)]
pub struct SmartMarksMutation;

#[Object]
impl SmartMarksMutation {
    /// Say the caller's going to watch `workId`, so the server prepares it next:
    /// for `deviceId` alone if given, whatever its smart transcoding setting, or
    /// otherwise for all their devices with it on. A series or season stands for
    /// its next unwatched episodes. Lasts until it's unmarked or watched, and
    /// marking again keeps its place in the queue.
    async fn mark_for_smart_transcoding(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        device_id: Option<DeviceId>,
    ) -> async_graphql::Result<SmartMark> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let user = RequestUser {
            user_id: principal.user_id,
            is_admin: principal.is_admin,
        };
        let not_found = || async_graphql::Error::new("work not found");
        if !noflex_storage::access::user_can_access_work(storage.as_ref(), user, work_id)
            .await
            .map_err(gql_err)?
        {
            return Err(not_found());
        }
        let work = storage
            .get_work(work_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(not_found)?;
        if !matches!(
            work.kind,
            WorkKind::Movie | WorkKind::Series | WorkKind::Season | WorkKind::Episode
        ) {
            return Err(async_graphql::Error::new(
                "Only films and shows can be smart transcoded",
            ));
        }
        check_device(storage, principal.user_id, device_id).await?;
        let mark = storage
            .add_smart_transcode_mark(&SmartTranscodeMark {
                user_id: principal.user_id,
                work_id,
                device_id,
                created_at: OffsetDateTime::now_utc(),
                outcome: None,
                media_file_ids: Vec::new(),
            })
            .await
            .map_err(gql_err)?;
        wake(ctx);
        describe(ctx, storage, mark, work).await
    }

    /// Take a mark back: what's waiting leaves the queue, and what's been prepared
    /// can be evicted like anything else. Returns the mark, or null when there was
    /// none.
    async fn unmark_for_smart_transcoding(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        device_id: Option<DeviceId>,
    ) -> async_graphql::Result<Option<SmartMark>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let marks = storage
            .smart_transcode_marks_for_user(principal.user_id)
            .await
            .map_err(gql_err)?;
        let Some(mark) = marks
            .into_iter()
            .find(|mark| mark.work_id == work_id && mark.device_id == device_id)
        else {
            return Ok(None);
        };
        let work = storage.get_work(work_id).await.map_err(gql_err)?;
        let described = match work {
            Some(work) => Some(describe(ctx, storage, mark, work).await?),
            None => None,
        };
        storage
            .remove_smart_transcode_mark(principal.user_id, work_id, device_id)
            .await
            .map_err(gql_err)?;
        wake(ctx);
        Ok(described)
    }
}
