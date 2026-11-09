// SPDX-License-Identifier: AGPL-3.0-only
//
// Personal recommendations, computed on demand from watch history. Rank
// positions are the only output: scores mean nothing outside this module, and
// multi-server clients merge lists by rank.
//
// Taste is compared title against title rather than against one user profile.
// Summing tags into a profile would treat "action" and "big budget" as
// independent, so someone who loves big action films and dislikes big-budget
// dramas would look the same as someone who doesn't, and mixed tastes would
// average into nothing.

use std::collections::{HashMap, HashSet};

use noflex_core::{TagId, WorkId, WorkKind};
use noflex_storage::{
    RequestUser, Storage,
    domain::{ContentCategory, WatchedWork, Work, WorkFeatures},
    error::StorageResult,
    search::RRF_K,
};
use time::{Duration, OffsetDateTime};

const RECOMMENDED_KINDS: [WorkKind; 2] = [WorkKind::Movie, WorkKind::Series];
/// What a title is, as far as taste goes. `parental` is the age-rating
/// namespace; `person` holds the directors and lead cast providers report.
const FEATURE_NAMESPACES: [&str; 6] = ["genre", "studio", "person", "parental", "keyword", "scale"];

/// How many recently watched works describe a user's current taste.
const TASTE_SEEDS: usize = 30;
/// A seed's influence halves every this many days.
const TASTE_HALF_LIFE_DAYS: f32 = 60.0;

// Starting values, to be tuned on a real library.
const FEEDBACK_LOVED: f32 = 3.0;
const FEEDBACK_FINISHED: f32 = 1.0;
const FEEDBACK_IN_PROGRESS: f32 = 0.3;
const FEEDBACK_ABANDONED: f32 = -0.5;
const FEEDBACK_DISLIKED: f32 = -2.0;
/// User ratings are on the same 0-10 scale as community ratings.
const LOVED_RATING: f32 = 8.0;
const DISLIKED_RATING: f32 = 4.0;
/// An unfinished work idle this long has been dropped, not paused.
const ABANDONED_AFTER: Duration = Duration::days(30);
/// A movie stopped past this share of its runtime was mostly watched.
const ABANDONED_BEFORE_FRACTION: f32 = 0.5;

/// Titles at least this similar count against the variety cap.
const SIMILAR_TITLE: f32 = 0.5;
/// How many closely similar titles one list holds before others get a turn.
const MAX_SIMILAR_TITLES: usize = 3;

/// Up to `limit` works for `user`, best first, excluding anything they have
/// started. Merges per-title tag similarity with, when `embedder_namespace` is
/// set and a liked seed has a vector, embedding similarity. Without positive
/// history, falls back to the highest-rated titles.
///
/// `category` narrows what's recommended, not what describes the user's taste:
/// the shows they watch still say which movies they'd like.
///
/// Respects library grants and parental restrictions through `user`.
pub(crate) async fn recommendations(
    storage: &dyn Storage,
    embedder_namespace: Option<&str>,
    user: RequestUser,
    category: Option<ContentCategory>,
    limit: u32,
    now: OffsetDateTime,
) -> StorageResult<Vec<Work>> {
    let catalog = storage
        .work_features(
            &RECOMMENDED_KINDS,
            &FEATURE_NAMESPACES,
            user.access_user_id(),
        )
        .await?;
    let history = storage.watched_top_level_works(user.user_id).await?;
    let started = history.iter().map(|w| w.work).collect::<HashSet<_>>();
    let seeds = seeds(&history, &catalog, now);
    let similarity = Similarity::new(&catalog);

    let mut rankings = Vec::new();
    if seeds.iter().any(|seed| seed.weight > 0.0) {
        rankings.push(similarity.rank(&seeds, &started));
        if let Some(namespace) = embedder_namespace {
            rankings
                .push(by_taste_vector(storage, namespace, user, &seeds, &started, limit).await?);
        }
    }
    let wanted = match category {
        Some(category) => Some(
            storage
                .category_work_ids(category, user.access_user_id())
                .await?
                .into_iter()
                .collect::<HashSet<_>>(),
        ),
        None => None,
    };
    let in_category = |work: &WorkId| wanted.as_ref().is_none_or(|set| set.contains(work));
    let mut fused = fuse(&rankings);
    fused.retain(in_category);
    let ranked = if fused.is_empty() {
        let mut highest_rated = by_rating(&catalog, &started);
        highest_rated.retain(in_category);
        highest_rated
    } else {
        similarity.with_variety(fused, limit as usize)
    };
    let top = ranked.into_iter().take(limit as usize).collect::<Vec<_>>();
    works_in_order(storage, &top).await
}

#[derive(Debug, Clone, Copy)]
struct Seed {
    work: WorkId,
    /// Recency times feedback; negative for titles the user turned away from.
    weight: f32,
}

/// The latest movies and series in `history` with their weights. Titles that
/// aren't in `catalog` have left the user's reach and say nothing current.
fn seeds(history: &[WatchedWork], catalog: &[WorkFeatures], now: OffsetDateTime) -> Vec<Seed> {
    let by_id = catalog
        .iter()
        .map(|features| (features.work, features))
        .collect::<HashMap<_, _>>();
    history
        .iter()
        .filter_map(|watched| Some((watched, *by_id.get(&watched.work)?)))
        .take(TASTE_SEEDS)
        .map(|(watched, features)| Seed {
            work: watched.work,
            weight: recency_weight(now - watched.last_played) * feedback(watched, features, now),
        })
        .collect()
}

fn recency_weight(age: Duration) -> f32 {
    let days = age.whole_hours().max(0) as f32 / 24.0;
    0.5_f32.powf(days / TASTE_HALF_LIFE_DAYS)
}

/// How much a title says the user likes things like it. An explicit opinion
/// outranks what watching implies.
fn feedback(watched: &WatchedWork, features: &WorkFeatures, now: OffsetDateTime) -> f32 {
    if watched
        .rating
        .is_some_and(|rating| rating <= DISLIKED_RATING)
    {
        return FEEDBACK_DISLIKED;
    }
    if watched.favorite || watched.rating.is_some_and(|rating| rating >= LOVED_RATING) {
        return FEEDBACK_LOVED;
    }
    if watched.finished {
        return FEEDBACK_FINISHED;
    }
    if now - watched.last_played >= ABANDONED_AFTER && stopped_early(watched, features) {
        return FEEDBACK_ABANDONED;
    }
    FEEDBACK_IN_PROGRESS
}

/// Whether an unfinished title was left early. A series with no finished
/// episode was; for a movie, only a known runtime can say.
fn stopped_early(watched: &WatchedWork, features: &WorkFeatures) -> bool {
    if features.kind == WorkKind::Series {
        return true;
    }
    match (watched.position_ms, features.runtime_ms) {
        (Some(position), Some(runtime)) if runtime > 0 => {
            (position as f32 / runtime as f32) < ABANDONED_BEFORE_FRACTION
        }
        _ => false,
    }
}

/// One thing a title can have in common with another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Feature {
    Tag(TagId),
    Decade(i32),
    RuntimeBucket(u8),
    RatingBucket(u8),
    Kind(WorkKind),
}

fn features_of(work: &WorkFeatures) -> Vec<Feature> {
    let mut features = work
        .tags
        .iter()
        .copied()
        .map(Feature::Tag)
        .collect::<Vec<_>>();
    features.push(Feature::Kind(work.kind));
    if let Some(year) = work.year {
        features.push(Feature::Decade(year / 10));
    }
    if let Some(runtime) = work.runtime_ms {
        let minutes = runtime / 60_000;
        features.push(Feature::RuntimeBucket(match minutes {
            ..30 => 0,
            30..60 => 1,
            60..100 => 2,
            100..140 => 3,
            _ => 4,
        }));
    }
    if let Some(rating) = work.community_rating {
        features.push(Feature::RatingBucket(rating.clamp(0.0, 10.0) as u8));
    }
    features
}

/// Cosine similarity between titles over their features, with each feature
/// weighted by how rare it is in the catalog: sharing an obscure keyword says
/// far more than both being movies.
struct Similarity {
    /// Each title's features with their weights.
    titles: HashMap<WorkId, Vec<(Feature, f32)>>,
    norms: HashMap<WorkId, f32>,
    postings: HashMap<Feature, Vec<WorkId>>,
}

impl Similarity {
    fn new(catalog: &[WorkFeatures]) -> Self {
        let raw = catalog
            .iter()
            .map(|work| (work.work, features_of(work)))
            .collect::<Vec<_>>();
        let mut postings: HashMap<Feature, Vec<WorkId>> = HashMap::new();
        for (work, features) in &raw {
            for feature in features {
                postings.entry(*feature).or_default().push(*work);
            }
        }
        let total = catalog.len() as f32;
        let mut titles = HashMap::with_capacity(raw.len());
        let mut norms = HashMap::with_capacity(raw.len());
        for (work, features) in raw {
            let weighted = features
                .into_iter()
                .map(|feature| {
                    let count = postings.get(&feature).map_or(1, Vec::len) as f32;
                    (feature, (1.0 + total / count).ln())
                })
                .collect::<Vec<_>>();
            let norm = weighted
                .iter()
                .map(|(_, weight)| weight * weight)
                .sum::<f32>()
                .sqrt();
            norms.insert(work, norm);
            titles.insert(work, weighted);
        }
        Self {
            titles,
            norms,
            postings,
        }
    }

    fn between(&self, a: WorkId, b: WorkId) -> f32 {
        let (Some(features), Some(other)) = (self.titles.get(&a), self.titles.get(&b)) else {
            return 0.0;
        };
        let dot = features
            .iter()
            .filter(|(feature, _)| other.iter().any(|(theirs, _)| theirs == feature))
            .map(|(_, weight)| weight * weight)
            .sum::<f32>();
        let norm =
            self.norms.get(&a).copied().unwrap_or(0.0) * self.norms.get(&b).copied().unwrap_or(0.0);
        if norm == 0.0 { 0.0 } else { dot / norm }
    }

    /// Titles outside `excluded` that resemble what the seeds pull towards,
    /// best first. Titles nothing recommends are left out.
    fn rank(&self, seeds: &[Seed], excluded: &HashSet<WorkId>) -> Vec<WorkId> {
        let mut scores: HashMap<WorkId, f32> = HashMap::new();
        for seed in seeds {
            let (Some(features), Some(seed_norm)) =
                (self.titles.get(&seed.work), self.norms.get(&seed.work))
            else {
                continue;
            };
            let mut dots: HashMap<WorkId, f32> = HashMap::new();
            for (feature, weight) in features {
                for candidate in self.postings.get(feature).into_iter().flatten() {
                    if !excluded.contains(candidate) {
                        *dots.entry(*candidate).or_default() += weight * weight;
                    }
                }
            }
            for (candidate, dot) in dots {
                let norm = seed_norm * self.norms.get(&candidate).copied().unwrap_or(0.0);
                if norm > 0.0 {
                    *scores.entry(candidate).or_default() += dot / norm * seed.weight;
                }
            }
        }
        let mut ranked = scores
            .into_iter()
            .filter(|(_, score)| *score > 0.0)
            .collect::<Vec<_>>();
        // Ids break ties so a page boundary stays put between requests.
        ranked.sort_by(|(a_id, a), (b_id, b)| {
            b.total_cmp(a)
                .then_with(|| a_id.into_uuid().cmp(&b_id.into_uuid()))
        });
        ranked.into_iter().map(|(id, _)| id).collect()
    }

    /// `ranked` reordered so the first `limit` titles don't crowd around one
    /// kind of thing: a title closely similar to too many already chosen waits
    /// until the end instead of being dropped.
    fn with_variety(&self, ranked: Vec<WorkId>, limit: usize) -> Vec<WorkId> {
        let mut chosen: Vec<WorkId> = Vec::with_capacity(ranked.len());
        let mut waiting = Vec::new();
        for work in ranked {
            let similar = if chosen.len() < limit {
                chosen
                    .iter()
                    .filter(|other| self.between(work, **other) >= SIMILAR_TITLE)
                    .count()
            } else {
                0
            };
            if similar >= MAX_SIMILAR_TITLES {
                waiting.push(work);
            } else {
                chosen.push(work);
            }
        }
        chosen.extend(waiting);
        chosen
    }
}

async fn by_taste_vector(
    storage: &dyn Storage,
    namespace: &str,
    user: RequestUser,
    seeds: &[Seed],
    started: &HashSet<WorkId>,
    limit: u32,
) -> StorageResult<Vec<WorkId>> {
    // A vector can't subtract a dislike usefully: the opposite of a drama's
    // plot summary isn't anything in particular. The tag ranking handles those.
    let weights = seeds
        .iter()
        .filter(|seed| seed.weight > 0.0)
        .map(|seed| (seed.work, seed.weight))
        .collect::<HashMap<_, _>>();
    let ids = weights.keys().copied().collect::<Vec<_>>();
    let weighted = storage
        .work_embeddings(namespace, &ids)
        .await?
        .into_iter()
        .filter_map(|(id, vector)| weights.get(&id).map(|weight| (vector, *weight)))
        .collect::<Vec<_>>();
    let Some(taste) = taste_vector(&weighted) else {
        return Ok(Vec::new());
    };
    let fetch = limit.saturating_add(started.len() as u32);
    let hits = storage
        .search_vector(
            namespace,
            &taste,
            &RECOMMENDED_KINDS,
            user.access_user_id(),
            fetch,
        )
        .await?;
    Ok(hits
        .into_iter()
        .map(|hit| hit.work)
        .filter(|id| !started.contains(id))
        .collect())
}

/// The unit-length weighted mean of `seeds`. Vectors whose length differs from
/// the first are skipped: they come from another model and can't be averaged.
/// `None` when there is nothing to average.
fn taste_vector(seeds: &[(Vec<f32>, f32)]) -> Option<Vec<f32>> {
    let dimensions = seeds.first()?.0.len();
    let mut sum = vec![0.0_f32; dimensions];
    for (vector, weight) in seeds.iter().filter(|(v, _)| v.len() == dimensions) {
        for (total, value) in sum.iter_mut().zip(vector) {
            *total += value * weight;
        }
    }
    let length = sum.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length == 0.0 || !length.is_finite() {
        return None;
    }
    sum.iter_mut().for_each(|value| *value /= length);
    Some(sum)
}

/// Reciprocal rank fusion of `rankings`, best first.
fn fuse(rankings: &[Vec<WorkId>]) -> Vec<WorkId> {
    let mut scores: HashMap<WorkId, f32> = HashMap::new();
    for ranking in rankings {
        for (rank, work) in ranking.iter().enumerate() {
            *scores.entry(*work).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|(a_id, a), (b_id, b)| {
        b.total_cmp(a)
            .then_with(|| a_id.into_uuid().cmp(&b_id.into_uuid()))
    });
    ranked.into_iter().map(|(id, _)| id).collect()
}

/// For users with nothing to go on: unstarted titles by community rating, then
/// newest first, with unrated titles last.
fn by_rating(catalog: &[WorkFeatures], started: &HashSet<WorkId>) -> Vec<WorkId> {
    let mut titles = catalog
        .iter()
        .filter(|work| !started.contains(&work.work))
        .collect::<Vec<_>>();
    titles.sort_by(|a, b| {
        let rating = |work: &WorkFeatures| work.community_rating.unwrap_or(-1.0);
        rating(b)
            .total_cmp(&rating(a))
            .then_with(|| b.added_at.cmp(&a.added_at))
            .then_with(|| a.work.into_uuid().cmp(&b.work.into_uuid()))
    });
    titles.into_iter().map(|work| work.work).collect()
}

async fn works_in_order(storage: &dyn Storage, ids: &[WorkId]) -> StorageResult<Vec<Work>> {
    let mut works = storage
        .get_works(ids)
        .await?
        .into_iter()
        .map(|work| (work.id, work))
        .collect::<HashMap<_, _>>();
    Ok(ids.iter().filter_map(|id| works.remove(id)).collect())
}

#[cfg(test)]
mod tests {
    use noflex_core::{EmbeddingTarget, LibraryId, MetadataStatus, PlayTarget, UserId};
    use noflex_storage::domain::{Library, User};
    use noflex_storage_turso::TursoStore;

    use super::*;

    fn title(tags: &[TagId]) -> WorkFeatures {
        WorkFeatures {
            work: WorkId::new(),
            kind: WorkKind::Movie,
            year: None,
            community_rating: None,
            runtime_ms: None,
            added_at: OffsetDateTime::UNIX_EPOCH,
            tags: tags.to_vec(),
        }
    }

    fn seed(work: &WorkFeatures, weight: f32) -> Seed {
        Seed {
            work: work.work,
            weight,
        }
    }

    fn watched(work: WorkId, age_days: i64, now: OffsetDateTime) -> WatchedWork {
        WatchedWork {
            work,
            last_played: now - Duration::days(age_days),
            finished: false,
            position_ms: None,
            favorite: false,
            rating: None,
        }
    }

    #[test]
    fn per_title_scoring_keeps_feature_combinations_apart() {
        let (action, drama, big) = (TagId::new(), TagId::new(), TagId::new());
        let loved = title(&[action, big]);
        let disliked = title(&[drama, big]);
        let big_action = title(&[action, big]);
        let big_drama = title(&[drama, big]);
        let catalog = [
            loved.clone(),
            disliked.clone(),
            big_action.clone(),
            big_drama.clone(),
        ];
        let similarity = Similarity::new(&catalog);
        let started = HashSet::from([loved.work, disliked.work]);

        let ranked = similarity.rank(&[seed(&loved, 1.0), seed(&disliked, -1.0)], &started);

        assert_eq!(
            ranked,
            [big_action.work],
            "big-budget drama nets out negative though \"big\" is liked with action"
        );
    }

    #[test]
    fn rare_features_outweigh_common_ones() {
        let (common, rare) = (TagId::new(), TagId::new());
        let seen = title(&[common, rare]);
        let shares_rare = title(&[rare]);
        let shares_common = title(&[common]);
        let mut catalog = vec![seen.clone(), shares_rare.clone(), shares_common.clone()];
        catalog.extend((0..20).map(|_| title(&[common])));
        let similarity = Similarity::new(&catalog);

        let ranked = similarity.rank(&[seed(&seen, 1.0)], &HashSet::from([seen.work]));

        assert_eq!(ranked[0], shares_rare.work);
    }

    #[test]
    fn started_titles_are_never_ranked() {
        let tag = TagId::new();
        let (seen, also_started, fresh) = (title(&[tag]), title(&[tag]), title(&[tag]));
        let catalog = [seen.clone(), also_started.clone(), fresh.clone()];
        let ranked = Similarity::new(&catalog).rank(
            &[seed(&seen, 1.0)],
            &HashSet::from([seen.work, also_started.work]),
        );
        assert_eq!(ranked, [fresh.work]);
    }

    #[test]
    fn feedback_orders_opinions_over_implied_interest() {
        let now = OffsetDateTime::now_utc();
        let movie = |runtime_ms| WorkFeatures {
            runtime_ms,
            ..title(&[])
        };
        let hour = Some(3_600_000);

        let mut favorite = watched(WorkId::new(), 1, now);
        favorite.favorite = true;
        assert_eq!(feedback(&favorite, &movie(hour), now), FEEDBACK_LOVED);

        let mut panned = watched(WorkId::new(), 1, now);
        panned.finished = true;
        panned.rating = Some(2.0);
        assert_eq!(
            feedback(&panned, &movie(hour), now),
            FEEDBACK_DISLIKED,
            "a low rating outranks having finished it"
        );

        let mut finished = watched(WorkId::new(), 1, now);
        finished.finished = true;
        assert_eq!(feedback(&finished, &movie(hour), now), FEEDBACK_FINISHED);

        let mut early = watched(WorkId::new(), 60, now);
        early.position_ms = Some(600_000);
        assert_eq!(feedback(&early, &movie(hour), now), FEEDBACK_ABANDONED);

        let mut recent = early;
        recent.last_played = now - Duration::days(1);
        assert_eq!(
            feedback(&recent, &movie(hour), now),
            FEEDBACK_IN_PROGRESS,
            "not idle long enough to call it abandoned"
        );

        let mut nearly_done = early;
        nearly_done.position_ms = Some(3_000_000);
        assert_eq!(
            feedback(&nearly_done, &movie(hour), now),
            FEEDBACK_IN_PROGRESS
        );
        assert_eq!(
            feedback(&early, &movie(None), now),
            FEEDBACK_IN_PROGRESS,
            "without a runtime there's no telling how early it was"
        );
    }

    #[test]
    fn variety_cap_defers_crowding_titles_without_dropping_them() {
        let (crowded, other) = (TagId::new(), TagId::new());
        let similar = (0..5).map(|_| title(&[crowded])).collect::<Vec<_>>();
        let different = title(&[other]);
        let mut catalog = similar.clone();
        catalog.push(different.clone());
        let ranked = similar
            .iter()
            .map(|work| work.work)
            .chain([different.work])
            .collect::<Vec<_>>();

        let out = Similarity::new(&catalog).with_variety(ranked.clone(), 4);

        assert_eq!(out[..3], ranked[..3], "the cap allows the first three");
        assert_eq!(out[3], different.work, "the next slot goes elsewhere");
        assert_eq!(out.len(), ranked.len());
    }

    #[test]
    fn recent_seeds_dominate_the_taste_vector() {
        let recent = recency_weight(Duration::days(1));
        let old = recency_weight(Duration::days(TASTE_HALF_LIFE_DAYS as i64 * 4));
        let taste =
            taste_vector(&[(vec![1.0, 0.0], recent), (vec![0.0, 1.0], old)]).expect("taste");
        assert!(taste[0] > taste[1] * 10.0, "{taste:?}");
    }

    #[test]
    fn vectors_from_another_model_are_ignored() {
        let taste =
            taste_vector(&[(vec![1.0, 0.0], 1.0), (vec![0.0, 1.0, 5.0], 1.0)]).expect("taste");
        assert_eq!(taste, [1.0, 0.0]);
    }

    #[test]
    fn fusion_rewards_agreement_between_rankings() {
        let (a, b, c) = (WorkId::new(), WorkId::new(), WorkId::new());
        let fused = fuse(&[vec![a, b], vec![c, b]]);
        assert_eq!(fused[0], b, "second in both beats first in one");
    }

    #[test]
    fn users_without_history_get_the_highest_rated_titles() {
        let rated = |rating| WorkFeatures {
            community_rating: rating,
            ..title(&[])
        };
        let (best, good, unrated) = (rated(Some(9.0)), rated(Some(7.0)), rated(None));
        let catalog = [unrated.clone(), good.clone(), best.clone()];
        assert_eq!(
            by_rating(&catalog, &HashSet::new()),
            [best.work, good.work, unrated.work]
        );
    }

    async fn store_with_user() -> (TursoStore, RequestUser, LibraryId) {
        let store = TursoStore::open_temp().await.expect("store");
        store.migrate().await.expect("migrate");
        let user = User {
            id: UserId::new(),
            username: "viewer".to_string(),
            display_name: None,
            password_hash: "argon2id$placeholder".to_string(),
            is_admin: true,
            role: noflex_core::UserRole::Admin,
            created_at: OffsetDateTime::now_utc(),
            avatar_updated_at: None,
            has_pin: false,
        };
        store.upsert_user(&user).await.expect("user");
        let library = Library {
            id: LibraryId::new(),
            name: "Films".to_string(),
            ..Default::default()
        };
        store.upsert_library(&library).await.expect("library");
        let request = RequestUser {
            user_id: user.id,
            is_admin: true,
        };
        (store, request, library.id)
    }

    async fn work(store: &TursoStore, library: LibraryId, title: &str) -> WorkId {
        let now = OffsetDateTime::now_utc();
        let work = Work {
            id: WorkId::new(),
            kind: WorkKind::Movie,
            parent_id: None,
            library_id: library,
            title: title.to_string(),
            sort_title: title.to_lowercase(),
            year: None,
            release_date: None,
            overview: None,
            external_ids: vec![],
            index_number: None,
            index_end: None,
            parent_index: None,
            metadata_status: MetadataStatus::Unconfirmed,
            tagline: None,
            original_title: None,
            community_rating: None,
            locked_fields: vec![],
            metadata_provider: None,
            added_at: now,
            updated_at: now,
        };
        store.upsert_work(&work).await.expect("work");
        work.id
    }

    fn titles(works: &[Work]) -> Vec<&str> {
        works.iter().map(|work| work.title.as_str()).collect()
    }

    #[tokio::test]
    async fn taste_vector_ranks_by_liked_titles_embeddings() {
        let (store, user, library) = store_with_user().await;
        let seen = work(&store, library, "Seen").await;
        let far = work(&store, library, "Far").await;
        let near = work(&store, library, "Near").await;
        for (id, vector) in [(seen, [1.0, 0.0]), (near, [0.9, 0.1]), (far, [0.0, 1.0])] {
            store
                .upsert_embedding(EmbeddingTarget::Work(id), "taste", "model", &vector)
                .await
                .expect("embedding");
        }
        store
            .mark_played(user.user_id, PlayTarget::Work(seen), true)
            .await
            .expect("played");

        let liked = [Seed {
            work: seen,
            weight: 1.0,
        }];
        let started = HashSet::from([seen]);
        let ranked = by_taste_vector(&store, "taste", user, &liked, &started, 10)
            .await
            .expect("ranking");
        assert_eq!(ranked, [near, far]);

        let disliked = [Seed {
            work: seen,
            weight: -1.0,
        }];
        let ranked = by_taste_vector(&store, "taste", user, &disliked, &started, 10)
            .await
            .expect("ranking");
        assert!(ranked.is_empty(), "dislikes don't steer the vector");

        let out = recommendations(
            &store,
            Some("taste"),
            user,
            None,
            10,
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("recommendations");
        let mut listed = titles(&out);
        listed.sort_unstable();
        assert_eq!(listed, ["Far", "Near"], "started titles stay out");
    }

    #[tokio::test]
    async fn a_category_narrows_recommendations_but_not_taste() {
        let (store, user, library) = store_with_user().await;
        let watched_show = work(&store, library, "Watched show").await;
        let mut series = store
            .get_work(watched_show)
            .await
            .expect("get")
            .expect("work");
        series.kind = WorkKind::Series;
        store.upsert_work(&series).await.expect("series");
        let other_show = work(&store, library, "Other show").await;
        let mut other = store
            .get_work(other_show)
            .await
            .expect("get")
            .expect("work");
        other.kind = WorkKind::Series;
        store.upsert_work(&other).await.expect("series");
        work(&store, library, "A movie").await;
        store
            .mark_played(user.user_id, PlayTarget::Work(watched_show), true)
            .await
            .expect("played");

        let now = OffsetDateTime::now_utc();
        let everything = recommendations(&store, None, user, None, 10, now)
            .await
            .expect("recommendations");
        assert_eq!(titles(&everything), ["Other show"]);
        let movies = recommendations(&store, None, user, Some(ContentCategory::Movies), 10, now)
            .await
            .expect("recommendations");
        assert_eq!(titles(&movies), ["A movie"]);
    }

    #[tokio::test]
    async fn new_users_still_get_recommendations_without_an_embedder() {
        let (store, user, library) = store_with_user().await;
        work(&store, library, "Only title").await;

        let out = recommendations(&store, None, user, None, 10, OffsetDateTime::now_utc())
            .await
            .expect("recommendations");

        assert_eq!(titles(&out), ["Only title"]);
    }
}
