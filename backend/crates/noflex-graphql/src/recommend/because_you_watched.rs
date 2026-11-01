// SPDX-License-Identifier: AGPL-3.0-only

use noflex_core::{
    ids::UserId,
    model::{TagTarget, WorkKind},
};
use noflex_storage::{domain::Work, query::TagQuery, store::Storage};
use uuid::Uuid;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BecauseYouWatchedItem {
    pub seed: Work,
    pub similar: Vec<Work>,
}

/// Built-in "because you watched X" fallback.
///
/// For each of the user's N most recently played works
/// (the "seed"), collect the work's genre + studio tags,
/// and pull K other works sharing at least one of those
/// tags. The plugin recommendation hook provides the real ranking;
/// this built-in fallback is what the
/// `/Users/{id}/Items/Suggestions` endpoint returns when
/// no plugin is registered.
#[allow(dead_code)]
pub async fn because_you_watched(
    storage: &dyn Storage,
    user: UserId,
    per_seed_pool: u32,
    seed_count: u32,
) -> Result<Vec<BecauseYouWatchedItem>, noflex_storage::error::StorageError> {
    let played = storage.recently_played_for_user(user, seed_count).await?;
    let mut seen_seeds: Vec<Uuid> = Vec::with_capacity(played.len());
    let mut out = Vec::with_capacity(played.len());
    for seed in played {
        let seed_uuid = seed.id.into_uuid();
        if seen_seeds.contains(&seed_uuid) {
            continue;
        }
        let tags = storage
            .tags_for_target(TagTarget::Work(seed.id))
            .await
            .unwrap_or_default();
        let mut seen_similar: Vec<Uuid> = Vec::new();
        let mut similar_ids = Vec::new();
        for t in tags
            .iter()
            .filter(|t| matches!(t.namespace.as_str(), "genre" | "studio"))
        {
            if similar_ids.len() as u32 >= per_seed_pool {
                break;
            }
            let q = TagQuery {
                include_tags: vec![t.id],
                exclude_tags: Vec::new(),
                kinds: vec![seed.kind, WorkKind::Movie, WorkKind::Series],
                limit: per_seed_pool,
                cursor: None,
            };
            let page = storage.search_works_by_tags(&q).await?;
            for wid in page.items {
                let wid_uuid = wid.into_uuid();
                if wid_uuid == seed_uuid || seen_seeds.contains(&wid_uuid) {
                    continue;
                }
                if seen_similar.contains(&wid_uuid) {
                    continue;
                }
                seen_similar.push(wid_uuid);
                similar_ids.push(wid);
                if similar_ids.len() as u32 >= per_seed_pool {
                    break;
                }
            }
        }
        let similar = storage.get_works(&similar_ids).await?;
        seen_seeds.push(seed_uuid);
        out.push(BecauseYouWatchedItem { seed, similar });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn because_you_watched_item_carries_seed_and_similar() {
        let item = BecauseYouWatchedItem {
            seed: make_work("The Matrix", WorkKind::Movie),
            similar: vec![make_work("Blade Runner 2049", WorkKind::Movie)],
        };
        assert_eq!(item.seed.title, "The Matrix");
        assert_eq!(item.similar.len(), 1);
    }

    fn make_work(title: &str, kind: WorkKind) -> Work {
        Work {
            id: noflex_core::WorkId::new(),
            kind,
            parent_id: None,
            library_id: noflex_core::LibraryId::new(),
            title: title.to_string(),
            sort_title: title.to_lowercase(),
            year: None,
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
        }
    }
}
