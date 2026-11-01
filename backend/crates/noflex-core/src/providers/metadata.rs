// SPDX-License-Identifier: AGPL-3.0-only

use async_graphql::SimpleObject;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    ids::WorkId,
    model::{ExternalId, ImageKind, WorkKind},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub title: String,
    pub year: Option<i32>,
    pub kind: WorkKind,
    /// Hints from the scanner (e.g., parsed folder path) to disambiguate matches.
    pub hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SimpleObject)]
pub struct MetadataMatch {
    pub external_id: ExternalId,
    pub title: String,
    pub year: Option<i32>,
    /// Confidence in [0.0, 1.0]. Below ~0.6 the scanner flags for manual review.
    pub confidence: f32,
    /// Provider id that produced this match (e.g. "tmdb", "tvdb", or a
    /// plugin id for WASM providers).
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataImage {
    pub kind: ImageKind,
    pub url: String,
    pub language: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataPerson {
    pub name: String,
    pub role: Option<String>,
    pub character: Option<String>,
    pub photo_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataPayload {
    pub external_ids: Vec<ExternalId>,
    pub title: String,
    pub sort_title: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub runtime_ms: Option<i64>,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    pub people: Vec<MetadataPerson>,
    pub images: Vec<MetadataImage>,
    pub rating: Option<String>,
    pub community_rating: Option<f32>,
}

#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Stable identifier for this provider ("tmdb", "tvdb", "musicbrainz", ...).
    fn id(&self) -> &str;

    /// Which work kinds this provider can produce metadata for.
    fn supports(&self, kind: WorkKind) -> bool;

    async fn search(&self, query: &SearchQuery) -> PluginResult<Vec<MetadataMatch>>;

    async fn fetch(&self, external_id: &ExternalId) -> PluginResult<MetadataPayload>;

    /// Optional: fetch metadata for the children of a work (seasons for a series,
    /// episodes for a season). Default returns empty so providers can opt out.
    async fn fetch_children(
        &self,
        _parent: &ExternalId,
        _parent_work: WorkId,
    ) -> PluginResult<Vec<MetadataPayload>> {
        Ok(Vec::new())
    }
}
