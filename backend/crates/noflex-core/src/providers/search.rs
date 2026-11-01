// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{error::PluginResult, ids::WorkId, model::WorkKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub work: WorkId,
    pub score: f32,
    /// Optional rationale for debugging / explainability.
    pub matched_on: Vec<String>,
}

/// Override or augment the storage-native search. A backend-specific search
/// like Meilisearch or Elasticsearch would plug in here. The host runs all
/// `SearchProvider`s in parallel and fuses results (RRF by default).
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn search(
        &self,
        query: &str,
        kinds: &[WorkKind],
        limit: u32,
    ) -> PluginResult<Vec<SearchHit>>;
}
