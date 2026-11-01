// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::error::PluginResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmbedRequest {
    Text(String),
    Image(Bytes),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResult {
    pub model_id: String,
    pub dimensions: u32,
    pub vector: Vec<f32>,
}

/// Embedding provider for text / image inputs. Production embedders ship
/// as WASM plugins implementing the `embedder` WIT hook (local inference
/// only — the self-hosted invariant rules out external inference APIs);
/// this trait mirrors that surface for native callers.
#[async_trait]
pub trait Embedder: Send + Sync {
    fn id(&self) -> &str;

    fn namespace(&self) -> &str;

    fn dimensions(&self) -> u32;

    async fn embed(&self, request: EmbedRequest) -> PluginResult<EmbedResult>;
}
