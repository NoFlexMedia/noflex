// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::error::PluginResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleMatch {
    pub provider: String,
    pub language: String,
    /// Best-effort BCP-47 (e.g. "en-US"); falls back to ISO-639.
    pub locale_hint: Option<String>,
    pub format: String, // "srt" | "vtt" | "ass" | ...
    pub score: f32,
    /// Provider-internal handle used to retrieve the actual subtitle bytes.
    pub handle: String,
}

#[async_trait]
pub trait SubtitleProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn search(
        &self,
        media_hash: &str,
        languages: &[String],
    ) -> PluginResult<Vec<SubtitleMatch>>;

    async fn download(&self, handle: &str) -> PluginResult<Bytes>;
}
