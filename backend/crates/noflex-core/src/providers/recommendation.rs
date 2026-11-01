// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    model::{ExternalId, WorkKind},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationRequest {
    pub user: Option<crate::ids::UserId>,
    pub kind: WorkKind,
    pub seed: Vec<ExternalId>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub external_id: ExternalId,
    pub title: String,
    pub year: Option<i32>,
    pub score: f32,
    /// "more-like-this" (vector similarity), "continue-series",
    /// "because-you-watched-X" (upstream "X-watched → Y" rules), or
    /// plugin-defined. Used by the UI to label the row.
    pub rationale: String,
}

#[async_trait]
pub trait RecommendationProvider: Send + Sync {
    fn id(&self) -> &str;
    fn supports(&self, kind: WorkKind) -> bool;
    async fn recommend(&self, req: &RecommendationRequest) -> PluginResult<Vec<Recommendation>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WorkKind;

    struct Empty;

    #[async_trait]
    impl RecommendationProvider for Empty {
        fn id(&self) -> &str {
            "empty"
        }
        fn supports(&self, _kind: WorkKind) -> bool {
            true
        }
        async fn recommend(
            &self,
            _req: &RecommendationRequest,
        ) -> PluginResult<Vec<Recommendation>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn empty_provider_returns_no_rows() {
        let p = Empty;
        let req = RecommendationRequest {
            user: None,
            kind: WorkKind::Movie,
            seed: Vec::new(),
            limit: 10,
        };
        let out = p.recommend(&req).await.expect("recommend");
        assert!(out.is_empty());
    }
}
