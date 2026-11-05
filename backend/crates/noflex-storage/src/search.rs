// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_graphql::{
    ComplexObject, SimpleObject,
    dataloader::{DataLoader, Loader},
};
use noflex_core::WorkId;
use serde::{Deserialize, Serialize};

use crate::{RequestUser, Storage, domain::Work};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkLoadKey {
    pub work_id: WorkId,
    pub user: RequestUser,
    pub access_checked: bool,
}

pub struct WorkLoader {
    storage: Arc<dyn Storage>,
}

impl WorkLoader {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }
}

impl Loader<WorkLoadKey> for WorkLoader {
    type Value = Option<Work>;
    type Error = String;

    async fn load(
        &self,
        keys: &[WorkLoadKey],
    ) -> Result<HashMap<WorkLoadKey, Self::Value>, Self::Error> {
        let ids = keys.iter().map(|key| key.work_id).collect::<Vec<_>>();
        let works = self
            .storage
            .get_works(&ids)
            .await
            .map_err(|e| e.to_string())?;
        let works = works
            .into_iter()
            .map(|work| (work.id, work))
            .collect::<HashMap<_, _>>();
        let mut allowed_by_user: HashMap<_, HashSet<_>> = HashMap::new();
        for user in keys
            .iter()
            .filter(|key| !key.access_checked && !key.user.is_admin)
            .map(|key| key.user.user_id)
            .collect::<HashSet<_>>()
        {
            let accessible = self
                .storage
                .accessible_work_ids(user, &ids)
                .await
                .map_err(|e| e.to_string())?;
            allowed_by_user.insert(user, accessible.into_iter().collect());
        }
        Ok(keys
            .iter()
            .copied()
            .map(|key| {
                let visible = key.access_checked
                    || key.user.is_admin
                    || allowed_by_user
                        .get(&key.user.user_id)
                        .is_some_and(|ids| ids.contains(&key.work_id));
                (
                    key,
                    visible.then(|| works.get(&key.work_id).cloned()).flatten(),
                )
            })
            .collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
#[graphql(complex)]
pub struct SearchHit {
    /// Raw id used internally to index into the hydrated `work`
    /// field below — the wire surface exposes it as `id` (ID)
    /// via the ComplexObject impl so the id stays usable without
    /// forcing every client to follow it up with a second query.
    #[graphql(skip)]
    pub work: WorkId,
    /// Set only by the backend that applied the caller's access filter;
    /// never serialized, so external input cannot claim a pre-checked hit.
    #[graphql(skip)]
    #[serde(skip)]
    pub access_checked: bool,
    pub score: f32,
    /// How close the work is to the query by meaning, from 0 to 1; null when it
    /// wasn't compared. Unlike `score`, this means the same thing on any server
    /// running the same embedding model.
    pub similarity: Option<f32>,
    /// Why the work matched, strongest reason first: `title`, `credits` or
    /// `overview` for the query's own words, `meaning` for an embedding match.
    pub matched_fields: Vec<String>,
}

#[ComplexObject]
impl SearchHit {
    /// Id of the matching work, which a search lists only once, so it identifies
    /// the hit too. Normalizing caches need it to append pages rather than treat
    /// every hit as the same record.
    async fn id(&self) -> WorkId {
        self.work
    }

    /// Hydrated work for this hit. `None` when the work was
    /// deleted between the search and the hydration read — a real
    /// race in the delete-during-scan window — so the field is
    /// null rather than erroring the whole page.
    async fn work(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Option<Work>> {
        let loader = ctx
            .data::<DataLoader<WorkLoader>>()
            .map_err(|_| async_graphql::Error::new("work loader not configured"))?;
        loader
            .load_one(WorkLoadKey {
                work_id: self.work,
                user: crate::access::request_user(ctx)?,
                access_checked: self.access_checked,
            })
            .await
            .map_err(async_graphql::Error::new)
            .map(Option::flatten)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorHit {
    pub work: WorkId,
    pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub backend: String,
    pub version: String,
    pub schema_version: u32,
    pub healthy: bool,
    pub notes: Vec<String>,
}

/// The rank-smoothing constant reciprocal rank fusion uses everywhere: a
/// result at zero-based `rank` contributes `1 / (RRF_K + rank + 1)`.
pub const RRF_K: f32 = 60.0;

/// [`SearchHit::matched_fields`] entry for a match on a work's own title.
pub const MATCHED_TITLE: &str = "title";

/// [`SearchHit::matched_fields`] entry for a match on the name of someone credited
/// on the work.
pub const MATCHED_CREDITS: &str = "credits";

/// [`SearchHit::matched_fields`] entry for a match on embedding similarity.
pub const MATCHED_MEANING: &str = "meaning";

/// Order for fused hits: anything matching by name first, so someone typing a
/// title or a person gets it, where fused scores alone would put an unrelated
/// nearest neighbour above the fifth film of an actor's filmography.
pub fn hybrid_order(a: &SearchHit, b: &SearchHit) -> std::cmp::Ordering {
    let by_name = |hit: &SearchHit| {
        hit.matched_fields
            .iter()
            .any(|field| field == MATCHED_TITLE || field == MATCHED_CREDITS)
    };
    by_name(b).cmp(&by_name(a)).then_with(|| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Fuses lexical and vector results with reciprocal rank fusion (RRF).
///
/// The constant `k = 60` is the standard from the original RRF paper
/// (Cormack et al. 2009). It smooths the contribution of lower-ranked
/// hits without parameter tuning per-corpus.
pub fn fuse_hybrid(
    lexical: Vec<SearchHit>,
    vector: Vec<VectorHit>,
    access_checked: bool,
    limit: u32,
) -> Vec<SearchHit> {
    let rrf = |rank: usize| 1.0 / (RRF_K + rank as f32 + 1.0);
    let mut fused = lexical
        .into_iter()
        .enumerate()
        .map(|(rank, hit)| {
            let hit = SearchHit {
                access_checked,
                score: rrf(rank),
                ..hit
            };
            (hit.work, hit)
        })
        .collect::<HashMap<_, _>>();
    for (rank, hit) in vector.into_iter().enumerate() {
        let entry = fused.entry(hit.work).or_insert_with(|| SearchHit {
            work: hit.work,
            access_checked,
            score: 0.0,
            similarity: None,
            matched_fields: Vec::new(),
        });
        entry.score += rrf(rank);
        entry.similarity = Some(hit.similarity);
        entry.matched_fields.push(MATCHED_MEANING.to_string());
    }
    let mut out = fused.into_values().collect::<Vec<_>>();
    out.sort_by(hybrid_order);
    out.truncate(limit as usize);
    out
}

/// Default hybrid-search impl. Backends with a native hybrid query plan
/// override `Storage::search_hybrid` and skip this.
pub async fn default_hybrid<S>(
    store: &S,
    query: &crate::query::HybridQuery,
) -> crate::error::StorageResult<Vec<SearchHit>>
where
    S: crate::store::Storage + ?Sized,
{
    use crate::query::SearchMode;

    let want_lex = matches!(query.mode, SearchMode::Lexical | SearchMode::Hybrid);
    let want_vec = matches!(query.mode, SearchMode::Vector | SearchMode::Hybrid)
        && query.query_vector.is_some();

    let lex_hits: Vec<SearchHit> = if want_lex {
        store
            .search_lexical(
                &query.text,
                &query.kinds,
                query.access_user_id,
                query.limit * 2,
            )
            .await?
    } else {
        Vec::new()
    };

    let vec_hits: Vec<VectorHit> = if want_vec {
        let v = query.query_vector.as_deref().unwrap_or(&[]);
        store
            .search_vector(
                &query.vector_namespace,
                v,
                &query.kinds,
                query.access_user_id,
                query.limit * 2,
            )
            .await?
    } else {
        Vec::new()
    };

    Ok(fuse_hybrid(
        lex_hits,
        vec_hits,
        query.access_user_id.is_some(),
        query.limit,
    ))
}
