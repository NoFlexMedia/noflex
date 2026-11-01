// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;

use noflex_core::{
    ids::{TagId, UserId, WorkId},
    model::{TagTarget, WorkKind},
};
use noflex_storage::{
    domain::{Tag, Work},
    query::TagQuery,
    store::Storage,
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub work: Work,
    pub score: u32,
    #[allow(dead_code)]
    pub rationale: String,
}

#[derive(Debug, Clone)]
pub struct MoreLikeThisConfig {
    pub played_lookback: u32,
    pub per_genre_pool: u32,
    pub kinds: Vec<WorkKind>,
}

impl Default for MoreLikeThisConfig {
    fn default() -> Self {
        Self {
            played_lookback: 20,
            per_genre_pool: 25,
            kinds: vec![WorkKind::Movie, WorkKind::Series],
        }
    }
}

/// Genre-overlap-based "more like this".
///
/// 1. Read the user's N most recently played works.
/// 2. Collect the union of their genre tags.
/// 3. For each genre, pull other works with that tag via
///    `Storage::search_works_by_tags`.
/// 4. Score each candidate by the count of input genres
///    they share.
/// 5. Exclude the input works themselves and return the
///    top `limit`.
///
/// When the user has no plays, the result is empty. When a
/// played work has no genres, it contributes nothing.
pub async fn more_like_this(
    storage: &dyn Storage,
    user: UserId,
    limit: u32,
    cfg: &MoreLikeThisConfig,
) -> Result<Vec<Recommendation>, noflex_storage::error::StorageError> {
    let played = storage
        .recently_played_for_user(user, cfg.played_lookback)
        .await?;
    if played.is_empty() {
        return Ok(Vec::new());
    }
    let mut played_ids: Vec<Uuid> = played.iter().map(|w| w.id.into_uuid()).collect();
    played_ids.sort();
    played_ids.dedup();
    let mut seen_genres: HashMap<Uuid, Tag> = HashMap::new();
    for w in &played {
        let tags = storage
            .tags_for_target(TagTarget::Work(w.id))
            .await
            .unwrap_or_default();
        for t in tags {
            if t.namespace == "genre" {
                seen_genres.insert(t.id.into_uuid(), t);
            }
        }
    }
    if seen_genres.is_empty() {
        return Ok(Vec::new());
    }
    let mut scores: HashMap<Uuid, (u32, WorkId, Vec<String>)> = HashMap::new();
    for (gid, tag) in &seen_genres {
        let q = TagQuery {
            include_tags: vec![TagId::from_uuid(*gid)],
            exclude_tags: Vec::new(),
            kinds: cfg.kinds.clone(),
            limit: cfg.per_genre_pool,
            cursor: None,
        };
        let page = storage.search_works_by_tags(&q).await?;
        for wid in page.items {
            if played_ids.binary_search(&wid.into_uuid()).is_ok() {
                continue;
            }
            let entry = scores
                .entry(wid.into_uuid())
                .or_insert((0, wid, Vec::new()));
            entry.0 += 1;
            entry.2.push(tag.slug.clone());
        }
    }
    let candidate_ids = scores.values().map(|(_, id, _)| *id).collect::<Vec<_>>();
    let works = storage
        .get_works(&candidate_ids)
        .await?
        .into_iter()
        .map(|work| (work.id.into_uuid(), work))
        .collect::<HashMap<_, _>>();
    let mut out: Vec<Recommendation> = scores
        .into_iter()
        .filter_map(|(id, (score, _, genres))| {
            works.get(&id).cloned().map(|work| Recommendation {
                rationale: format!("shared genres: {}", genres.join(", ")),
                work,
                score,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.work.title.cmp(&b.work.title))
    });
    out.truncate(limit as usize);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use noflex_core::model::WorkKind;

    use super::*;

    #[test]
    fn default_config_targets_movies_and_series() {
        let cfg = MoreLikeThisConfig::default();
        assert_eq!(cfg.played_lookback, 20);
        assert_eq!(cfg.per_genre_pool, 25);
        assert!(cfg.kinds.contains(&WorkKind::Movie));
        assert!(cfg.kinds.contains(&WorkKind::Series));
    }

    #[test]
    fn recommendation_struct_carries_score_and_rationale() {
        let r = Recommendation {
            work: noflex_storage::domain::Work {
                id: noflex_core::WorkId::new(),
                kind: WorkKind::Movie,
                parent_id: None,
                library_id: noflex_core::LibraryId::new(),
                title: "Test".to_string(),
                sort_title: "test".to_string(),
                year: Some(2024),
                release_date: None,
                overview: None,
                external_ids: vec![],
                index_number: None,
                index_end: None,
                parent_index: None,
                metadata_status: noflex_core::MetadataStatus::Unconfirmed,
                tagline: None,
                original_title: None,
                community_rating: None,
                locked_fields: vec![],
                added_at: time::OffsetDateTime::now_utc(),
                updated_at: time::OffsetDateTime::now_utc(),
            },
            score: 3,
            rationale: "shared genres: sci-fi, action".to_string(),
        };
        assert_eq!(r.score, 3);
        assert!(r.rationale.starts_with("shared genres:"));
    }
}
