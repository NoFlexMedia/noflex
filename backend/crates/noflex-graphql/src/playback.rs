// SPDX-License-Identifier: AGPL-3.0-only
//
// Playback reporting. The Jellyfin shim used to carry these writes;
// with it gone this mutation set is the only path that lands rows in
// `user_data`, so without it continue-watching stays empty.
// Writes use the per-request principal or a playback-scoped progress
// grant and return fresh `UserData` so clients need no follow-up query.

use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use async_graphql::{Context, Enum, ID, InputObject, Object};
use noflex_core::{
    DomainEvent, DomainEventBus, EditionId, MediaFileId, PlayTarget, PlayTargetKind, UserId, WorkId,
};
use noflex_storage::{
    Storage,
    domain::{ActivityKind, UserData},
};
use noflex_transcoder::ClientCapabilities;
use time::OffsetDateTime;

use crate::playback_session::{
    ActiveStream, PlaybackProgress, PlaybackSession, PlaybackSessionService, PlaybackStart,
};

/// Client-reported playback state. `Stopped` closes out the session in the
/// activity log. A session turning from `Playing` to `Paused` or back is
/// passed on to scrobblers once per change, so report the state every
/// heartbeat carries.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

/// What a playback write attaches to. Split into `kind` + `id` because
/// GraphQL has no tagged unions; converts to the canonical
/// [`PlayTarget`] before hitting storage.
#[derive(InputObject)]
pub struct PlayTargetInput {
    pub kind: PlayTargetKind,
    pub id: ID,
}

impl PlayTargetInput {
    pub(crate) fn to_play_target(&self) -> async_graphql::Result<PlayTarget> {
        let uuid = self
            .id
            .parse::<uuid::Uuid>()
            .map_err(|e| async_graphql::Error::new(format!("invalid target id: {e}")))?;
        Ok(match self.kind {
            PlayTargetKind::Work => PlayTarget::Work(WorkId::from_uuid(uuid)),
            PlayTargetKind::Edition => PlayTarget::Edition(EditionId::from_uuid(uuid)),
        })
    }
}

/// The calling user, from the per-request `Principal`. Playback writes
/// are principal-only rather than trusting legacy explicit `userId` args.
fn authed_user(ctx: &Context<'_>) -> async_graphql::Result<UserId> {
    crate::access::user(ctx)
}

async fn authorize_target(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    target: PlayTarget,
) -> async_graphql::Result<()> {
    let user = crate::access::request_user(ctx)?;
    let visible = match target {
        PlayTarget::Work(id) => crate::access::can_work(storage, user, id).await?,
        PlayTarget::Edition(id) => crate::access::can_edition(storage, user, id).await?,
    };
    if !visible {
        return Err(async_graphql::Error::new("target not found"));
    }
    Ok(())
}

/// Append an activity row for a playback event.
async fn log_play_activity(
    storage: &Arc<dyn Storage>,
    kind: ActivityKind,
    user: UserId,
    target: PlayTarget,
) -> async_graphql::Result<()> {
    let actor = user.to_string();
    let subject = target.id_uuid().to_string();
    match storage
        .append_activity(
            kind,
            Some(&actor),
            Some(&subject),
            None,
            OffsetDateTime::now_utc(),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(async_graphql::Error::new(e.to_string())),
    }
}

/// The domain-event bus is optional schema data; absent means silent
/// (tests, minimal embeddings).
fn event_bus(ctx: &Context<'_>) -> Option<Arc<DomainEventBus>> {
    ctx.data::<Arc<DomainEventBus>>().ok().cloned()
}

/// Resolve a play target to the work id + title events carry: work
/// targets read the work directly, edition targets hop edition → work.
/// Best-effort — a dangling target or storage hiccup downgrades to a
/// warn and a skipped event, never a failed mutation.
async fn resolve_event_work(
    storage: &Arc<dyn Storage>,
    target: PlayTarget,
) -> Option<(WorkId, String)> {
    let work_id = match target {
        PlayTarget::Work(id) => id,
        PlayTarget::Edition(id) => match storage.get_edition(id).await {
            Ok(Some(edition)) => edition.work_id,
            Ok(None) => {
                tracing::warn!(edition = %id, "event: edition not found; skipping");
                return None;
            }
            Err(e) => {
                tracing::warn!(edition = %id, error = %e, "event: edition lookup failed; skipping");
                return None;
            }
        },
    };
    match storage.get_work(work_id).await {
        Ok(Some(work)) => Some((work.id, work.title)),
        Ok(None) => {
            tracing::warn!(work = %work_id, "event: work not found; skipping");
            None
        }
        Err(e) => {
            tracing::warn!(work = %work_id, error = %e, "event: work lookup failed; skipping");
            None
        }
    }
}

/// Mark `target` played and tell everyone who cares: the activity log and
/// the domain-event subscribers (scrobblers, webhooks).
async fn complete_play(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    user: UserId,
    target: PlayTarget,
) -> async_graphql::Result<()> {
    storage
        .mark_played(user, target, true)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    log_play_activity(storage, ActivityKind::PlayCompleted, user, target).await?;
    if let Some(bus) = event_bus(ctx)
        && let Some((work, title)) = resolve_event_work(storage, target).await
    {
        bus.emit(DomainEvent::WorkPlayed { user, work, title });
    }
    Ok(())
}

/// Whether `position_ms` counts as having finished `target`. A work with
/// several editions is judged by its longest, because the session's edition
/// isn't known here and finishing early would lose a resume position. A
/// lookup failure counts as not finished: the next heartbeat tries again.
async fn past_completion_point(
    storage: &Arc<dyn Storage>,
    target: PlayTarget,
    position_ms: i64,
) -> bool {
    let editions = match target {
        PlayTarget::Work(work) => storage.editions_for_work(work).await,
        PlayTarget::Edition(edition) => storage
            .get_edition(edition)
            .await
            .map(|edition| edition.into_iter().collect()),
    };
    let editions = match editions {
        Ok(editions) => editions,
        Err(error) => {
            tracing::warn!(%error, "play completion: edition lookup failed");
            return false;
        }
    };
    editions
        .iter()
        .max_by_key(|edition| edition.runtime_ms)
        .and_then(|edition| completion_point(edition.runtime_ms?, edition.credits_start_ms))
        .is_some_and(|point| position_ms >= point)
}

/// Where a title counts as finished: the start of the credits, else 90% of
/// the runtime. Credits detected in the first half are a false positive and
/// are ignored. `None` without a usable runtime.
fn completion_point(runtime_ms: i64, credits_start_ms: Option<i64>) -> Option<i64> {
    if runtime_ms <= 0 {
        return None;
    }
    Some(
        credits_start_ms
            .filter(|credits| *credits >= runtime_ms / 2 && *credits <= runtime_ms)
            .unwrap_or(runtime_ms / 10 * 9),
    )
}

/// Persist progress through the same completion, history and event path for
/// authenticated clients and receivers holding a playback-scoped grant.
async fn record_progress(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    progress: PlaybackProgress,
    position_ms: i64,
    state: PlaybackState,
) -> async_graphql::Result<UserData> {
    let PlaybackProgress {
        user,
        target,
        device,
    } = progress;
    storage
        .record_playback_progress(user, target, position_ms)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    // Done here rather than in clients so every client agrees on what
    // "finished" means. Viewers rarely sit through the credits, so
    // waiting for an explicit markPlayed would strand the next episode.
    if past_completion_point(storage, target, position_ms).await
        && !storage
            .user_data(user, target)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .played
    {
        complete_play(ctx, storage, user, target).await?;
    }
    // Play history is analytics: a failure there must not lose the
    // resume position the client is waiting on.
    if let Err(error) = crate::stats::record_heartbeat(
        storage.as_ref(),
        user,
        target,
        position_ms,
        state,
        async || device,
        OffsetDateTime::now_utc(),
    )
    .await
    {
        tracing::warn!(%error, %user, "play history: heartbeat not recorded");
    }
    if state == PlaybackState::Stopped {
        log_play_activity(storage, ActivityKind::PlayProgress, user, target).await?;
        // Heartbeats stay silent; only the terminal stop pays for the
        // title-resolution reads.
        if let Some(bus) = event_bus(ctx)
            && let Some((work, title)) = resolve_event_work(storage, target).await
        {
            bus.emit(DomainEvent::ProgressAtStop {
                user,
                work,
                title,
                position_ms: u64::try_from(position_ms).unwrap_or(0),
            });
        }
    }
    storage
        .user_data(user, target)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

/// Playback half of the mutation root; merged into `Mutation` in
/// `lib.rs` (async-graphql can't split one `#[Object] impl` across
/// modules, so roots compose via `MergedObject`).
#[derive(Default)]
pub struct PlaybackMutation;

#[Object]
impl PlaybackMutation {
    /// Report the current playback position for a work or edition.
    /// `state: STOPPED` additionally logs a `play.progress` activity
    /// row so the activity feed reflects the session end.
    async fn report_progress(
        &self,
        ctx: &Context<'_>,
        target: PlayTargetInput,
        position_ms: i64,
        state: PlaybackState,
    ) -> async_graphql::Result<UserData> {
        let user = authed_user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let target = target.to_play_target()?;
        authorize_target(ctx, storage, target).await?;
        if let Ok(service) = ctx.data::<Arc<dyn PlaybackSessionService>>() {
            service.heartbeat(user, target, position_ms, state).await;
        }
        record_progress(
            ctx,
            storage,
            PlaybackProgress {
                user,
                target,
                device: caller_device(ctx).await,
            },
            position_ms,
            state,
        )
        .await
    }

    /// Report progress using only the grant from `startPlaybackSession`.
    /// The grant fixes the user, target and playback session; STOPPED cleans
    /// up only that playback. No access or refresh credential is needed.
    async fn report_playback_progress(
        &self,
        ctx: &Context<'_>,
        token: String,
        position_ms: i64,
        state: PlaybackState,
    ) -> async_graphql::Result<UserData> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let service = ctx
            .data::<Arc<dyn PlaybackSessionService>>()
            .map_err(|_| async_graphql::Error::new("playback sessions unavailable"))?;
        let progress = service
            .report_granted_progress(&token, position_ms, state)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "playback progress grant refused");
                async_graphql::Error::new("invalid playback progress grant")
            })?;
        record_progress(ctx, storage, progress, position_ms, state).await
    }

    /// Flag a work or edition as fully played.
    async fn mark_played(
        &self,
        ctx: &Context<'_>,
        target: PlayTargetInput,
    ) -> async_graphql::Result<UserData> {
        let user = authed_user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let target = target.to_play_target()?;
        authorize_target(ctx, storage, target).await?;
        complete_play(ctx, storage, user, target).await?;
        storage
            .user_data(user, target)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Clear the played flag on a work or edition.
    async fn mark_unplayed(
        &self,
        ctx: &Context<'_>,
        target: PlayTargetInput,
    ) -> async_graphql::Result<UserData> {
        let user = authed_user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let target = target.to_play_target()?;
        authorize_target(ctx, storage, target).await?;
        storage
            .mark_played(user, target, false)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        storage
            .user_data(user, target)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Start a playback session for a media file. The server decides
    /// direct play vs. HLS from the stored probe and the client's
    /// self-measured `capabilities`, spawning an HLS job when needed.
    /// `burnSubtitle` indexes the media file's subtitle tracks and
    /// forces a video re-encode; text tracks are otherwise delivered
    /// out of band through `subtitles`. `decision` explains the route.
    /// `startPositionMs` starts a transcode at that source position, so
    /// resuming or seeking far ahead doesn't wait for ffmpeg to catch
    /// up; the session's `startPositionMs` is the offset actually used:
    /// `0` for a video encode, whose playlist covers the whole title from
    /// the start and which the player seeks in itself.
    // Each parameter is one GraphQL field argument.
    #[allow(clippy::too_many_arguments)]
    async fn start_playback_session(
        &self,
        ctx: &Context<'_>,
        media_file_id: ID,
        capabilities: ClientCapabilities,
        burn_subtitle: Option<i32>,
        start_position_ms: Option<i64>,
        #[graphql(desc = "Index among the file's audio streams to play instead of its default.")]
        audio_stream: Option<u32>,
        #[graphql(
            default,
            desc = "Transcode to a ladder the player adapts along, from the chosen quality down."
        )]
        adaptive: bool,
    ) -> async_graphql::Result<PlaybackSession> {
        let user = authed_user(ctx)?;
        let service = ctx
            .data::<Arc<dyn PlaybackSessionService>>()
            .map_err(|_| async_graphql::Error::new("playback sessions unavailable"))?;
        let uuid = media_file_id
            .parse::<uuid::Uuid>()
            .map_err(|e| async_graphql::Error::new(format!("invalid media file id: {e}")))?;
        let burn = burn_subtitle
            .map(|v| {
                u32::try_from(v).map_err(|_| async_graphql::Error::new("burnSubtitle must be >= 0"))
            })
            .transpose()?;
        let device = caller_device(ctx).await;
        service
            .start(
                user,
                noflex_storage::access::caller_session(ctx),
                PlaybackStart {
                    media_file_id: MediaFileId::from_uuid(uuid),
                    capabilities,
                    burn_subtitle: burn,
                    device,
                    start_position_ms,
                    audio_stream,
                    adaptive,
                },
            )
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// End a session started by `startPlaybackSession`, stopping its
    /// transcode job if it has one.
    async fn stop_playback_session(
        &self,
        ctx: &Context<'_>,
        session_id: ID,
    ) -> async_graphql::Result<bool> {
        let user = authed_user(ctx)?;
        let service = ctx
            .data::<Arc<dyn PlaybackSessionService>>()
            .map_err(|_| async_graphql::Error::new("playback sessions unavailable"))?;
        service
            .stop(user, &session_id.0)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(true)
    }

    /// Set or clear the favorite flag on a work or edition.
    async fn set_favorite(
        &self,
        ctx: &Context<'_>,
        target: PlayTargetInput,
        favorite: bool,
    ) -> async_graphql::Result<UserData> {
        let user = authed_user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let target = target.to_play_target()?;
        authorize_target(ctx, storage, target).await?;
        storage
            .set_favorite(user, target, favorite)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if let Some(bus) = event_bus(ctx)
            && let Some((work, title)) = resolve_event_work(storage, target).await
        {
            bus.emit(DomainEvent::Favorite {
                user,
                work,
                title,
                favorite,
            });
        }
        storage
            .user_data(user, target)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}

/// The caller's device label from their auth session row. Best-effort:
/// API keys and legacy tokens carry no session.
async fn caller_device(ctx: &Context<'_>) -> Option<String> {
    let session = ctx.data_opt::<noflex_auth::Principal>()?.session_id?;
    let storage = ctx.data_opt::<Arc<dyn Storage>>()?;
    let row = storage.get_session(session).await.ok()??;
    match (row.client, row.device) {
        (Some(client), Some(device)) => Some(format!("{client} · {device}")),
        (client, device) => device.or(client),
    }
}

#[derive(Default)]
pub struct PlaybackQuery;

#[Object]
impl PlaybackQuery {
    /// The calling user's playback state (position, played, favorite,
    /// play count) for a work or edition.
    async fn user_data(
        &self,
        ctx: &Context<'_>,
        target: PlayTargetInput,
    ) -> async_graphql::Result<UserData> {
        let user = authed_user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let target = target.to_play_target()?;
        authorize_target(ctx, storage, target).await?;
        storage
            .user_data(user, target)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Every playback in progress, direct and transcoded, for the admin
    /// dashboard. Administrators only. Streams whose user no longer
    /// exists are omitted.
    async fn active_streams(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<ActiveStream>> {
        crate::access::require_admin(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let service = ctx
            .data::<Arc<dyn PlaybackSessionService>>()
            .map_err(|_| async_graphql::Error::new("playback sessions unavailable"))?;
        let records = service
            .active_streams()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let mut users = HashMap::new();
        let mut works = HashMap::new();
        let mut out = Vec::with_capacity(records.len());
        for record in records {
            if let Entry::Vacant(slot) = users.entry(record.user_id) {
                slot.insert(
                    storage
                        .get_user(record.user_id)
                        .await
                        .map_err(|e| async_graphql::Error::new(e.to_string()))?,
                );
            }
            let Some(user) = users.get(&record.user_id).cloned().flatten() else {
                continue;
            };
            let work = match record.work_id {
                Some(id) => {
                    if let Entry::Vacant(slot) = works.entry(id) {
                        slot.insert(
                            storage
                                .get_work(id)
                                .await
                                .map_err(|e| async_graphql::Error::new(e.to_string()))?,
                        );
                    }
                    works.get(&id).cloned().flatten()
                }
                None => None,
            };
            out.push(ActiveStream {
                id: ID(record.id),
                user,
                work,
                media_file_id: ID(record.media_file_id.to_string()),
                channel_id: record.channel_id.map(|c| ID(c.to_string())),
                device: record.device,
                started_at: record.started_at,
                last_active_at: record.last_active_at,
                position_ms: record.position_ms,
                decision: record.decision,
                transcode: record.transcode,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::completion_point;

    #[test]
    fn credits_mark_completion_unless_they_look_wrong() {
        assert_eq!(completion_point(100_000, Some(95_000)), Some(95_000));
        assert_eq!(completion_point(100_000, Some(80_000)), Some(80_000));
        assert_eq!(
            completion_point(100_000, Some(5_000)),
            Some(90_000),
            "credits in the first half are a false detection"
        );
        assert_eq!(completion_point(100_000, Some(120_000)), Some(90_000));
        assert_eq!(completion_point(100_000, None), Some(90_000));
        assert_eq!(completion_point(0, None), None);
    }
}
