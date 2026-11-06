// SPDX-License-Identifier: AGPL-3.0-only
//
// What a user is likely to watch next, in priority order. Smart transcoding
// prepares these titles and smart downloads carry them, so both agree on what
// matters most.

use std::collections::HashSet;

use noflex_core::{PlayTarget, UserId, WorkId, WorkKind};
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

/// What a mark on `work` stands for now: the title itself until it's been watched
/// since being marked, or for a series or season, the next `lookahead_episodes + 1`
/// episodes like that. Empty once there's nothing left to prepare. Comparing with
/// `marked_at` is what lets someone mark a title they've seen, to watch it again.
pub async fn marked_works(
    storage: &dyn Storage,
    user: UserId,
    work: &Work,
    marked_at: OffsetDateTime,
    lookahead_episodes: u32,
) -> StorageResult<Vec<Work>> {
    const PAGE: u32 = 50;
    let unwatched = async |candidate: &Work| -> StorageResult<bool> {
        let data = storage
            .user_data(user, PlayTarget::Work(candidate.id))
            .await?;
        Ok(!data.played || data.last_played.is_none_or(|at| at < marked_at))
    };
    let (series, season) = match work.kind {
        WorkKind::Series => (work.id, None),
        WorkKind::Season => match work.parent_id {
            Some(series) => (series, Some(work)),
            None => return Ok(Vec::new()),
        },
        _ if unwatched(work).await? => return Ok(vec![work.clone()]),
        _ => return Ok(Vec::new()),
    };
    let wanted = lookahead_episodes as usize + 1;
    let mut found = Vec::new();
    let mut after = (
        season.map_or(0, |season| season.index_number.unwrap_or(0)),
        0,
    );
    loop {
        let page = storage
            .episodes_after(series, after.0, after.1, PAGE)
            .await?;
        let Some(last) = page.last() else {
            return Ok(found);
        };
        after = (
            last.parent_index.unwrap_or(after.0),
            last.index_end.or(last.index_number).unwrap_or(after.1),
        );
        let full_page = page.len() == PAGE as usize;
        for episode in page {
            if season.is_some_and(|season| episode.parent_id != Some(season.id)) {
                return Ok(found);
            }
            if unwatched(&episode).await? {
                found.push(episode);
                if found.len() == wanted {
                    return Ok(found);
                }
            }
        }
        if !full_page {
            return Ok(found);
        }
    }
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
