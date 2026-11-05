// SPDX-License-Identifier: AGPL-3.0-only
//
// What a user is likely to watch next, in priority order. Smart transcoding
// prepares these titles and smart downloads carry them, so both agree on what
// matters most.

use std::collections::HashSet;

use noflex_core::{WorkId, WorkKind};
use noflex_storage::{
    RequestUser, Storage,
    domain::{MediaFile, Work},
    error::StorageResult,
};
use noflex_transcoder::probe::ProbeReport;
use time::{Duration, OffsetDateTime};

use crate::recommend;

/// How long a "Continue Watching" entry survives without activity.
pub(crate) const CONTINUE_WATCHING_DECAY: Duration = Duration::days(90);

/// How many continue-watching entries are worth considering.
const CONTINUE_LIMIT: u32 = 50;

/// How far ahead of the user to look.
#[derive(Debug, Clone, Copy)]
pub struct LikelyOptions {
    /// Episodes to include after each next episode.
    pub lookahead_episodes: u32,
    /// Top recommendations to include.
    pub recommendations: u32,
}

/// Playable works `user` can access that they're likely to watch next, most
/// likely first: continue-watching entries, each followed by the episodes after
/// it, then top recommendations. A recommended series stands for its first
/// episode. Works appear once, at their earliest position.
pub async fn likely_watched(
    storage: &dyn Storage,
    embedder_namespace: Option<&str>,
    user: RequestUser,
    options: LikelyOptions,
    now: OffsetDateTime,
) -> StorageResult<Vec<Work>> {
    let continuing = storage
        .continue_watching(user.user_id, now - CONTINUE_WATCHING_DECAY, CONTINUE_LIMIT)
        .await?;
    let accessible = if user.is_admin {
        None
    } else {
        let ids = continuing.iter().map(|work| work.id).collect::<Vec<_>>();
        Some(
            storage
                .accessible_work_ids(user.user_id, &ids)
                .await?
                .into_iter()
                .collect::<HashSet<_>>(),
        )
    };

    let mut seen: HashSet<WorkId> = HashSet::new();
    let mut likely = Vec::new();
    for work in continuing {
        if accessible
            .as_ref()
            .is_some_and(|set| !set.contains(&work.id))
        {
            continue;
        }
        let following = following_episodes(storage, &work, options.lookahead_episodes).await?;
        for candidate in std::iter::once(work).chain(following) {
            if seen.insert(candidate.id) {
                likely.push(candidate);
            }
        }
    }

    if options.recommendations > 0 {
        let recommended = recommend::recommendations(
            storage,
            embedder_namespace,
            user,
            None,
            options.recommendations,
            now,
        )
        .await?;
        for work in recommended {
            let playable = if work.kind == WorkKind::Series {
                storage.episodes_after(work.id, 0, 0, 1).await?.pop()
            } else {
                Some(work)
            };
            if let Some(playable) = playable
                && seen.insert(playable.id)
            {
                likely.push(playable);
            }
        }
    }
    Ok(likely)
}

/// The `limit` episodes after `episode`; empty for anything but an episode.
async fn following_episodes(
    storage: &dyn Storage,
    episode: &Work,
    limit: u32,
) -> StorageResult<Vec<Work>> {
    if episode.kind != WorkKind::Episode || limit == 0 {
        return Ok(Vec::new());
    }
    let Some(season) = episode.parent_id else {
        return Ok(Vec::new());
    };
    let Some(series) = storage.get_work(season).await?.and_then(|s| s.parent_id) else {
        return Ok(Vec::new());
    };
    let number = episode
        .index_end
        .or(episode.index_number)
        .unwrap_or_default();
    storage
        .episodes_after(
            series,
            episode.parent_index.unwrap_or_default(),
            number,
            limit,
        )
        .await
}

/// The files a viewer of `work` would play, in part order: its first edition's
/// parts, each as its tallest version when a part has several.
pub async fn playable_sources(storage: &dyn Storage, work: &Work) -> StorageResult<Vec<MediaFile>> {
    let Some(edition) = storage.editions_for_work(work.id).await?.into_iter().next() else {
        return Ok(Vec::new());
    };
    let height = |media: &MediaFile| {
        serde_json::from_value::<ProbeReport>(media.probe.0.clone())
            .ok()
            .and_then(|probe| probe.video.map(|video| video.height))
            .unwrap_or(0)
    };
    let mut parts: Vec<MediaFile> = Vec::new();
    for media in storage.media_files_for_edition(edition.id).await? {
        match parts
            .iter_mut()
            .find(|part| part.part_index == media.part_index)
        {
            Some(existing) if height(&media) > height(existing) => *existing = media,
            Some(_) => {}
            None => parts.push(media),
        }
    }
    parts.sort_by_key(|part| part.part_index);
    Ok(parts)
}
