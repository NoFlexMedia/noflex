// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{error::PluginResult, model::WorkKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedItem {
    pub kind: WorkKind,
    pub title: String,
    pub year: Option<i32>,
    pub series_title: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_end: Option<u32>,
    pub edition_tag: Option<String>,
    pub part_index: Option<u32>,
    pub is_sample_or_trailer: bool,
    /// Quality tags parsed from filename, e.g. ["1080p", "HDR", "Atmos"].
    pub quality_tags: Vec<String>,
}

#[async_trait]
pub trait ItemResolver: Send + Sync {
    fn id(&self) -> &str;

    /// Try to parse a single filesystem path into a `ResolvedItem`. Returning
    /// `None` means "this resolver doesn't recognize this path" — the next
    /// resolver in the chain will be tried.
    async fn resolve(&self, path: &str) -> PluginResult<Option<ResolvedItem>>;
}
