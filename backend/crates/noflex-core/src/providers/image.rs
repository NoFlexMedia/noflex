// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    model::{ExternalId, ImageKind},
};

/// One image a provider offers for a record, for an administrator choosing
/// artwork by hand. Mirrors the WIT `image-candidate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageCandidate {
    /// Provider-owned and opaque; handed back to
    /// [`ImageProvider::fetch_candidate`] unchanged.
    pub handle: String,
    pub kind: ImageKind,
    /// ISO 639-1 code of the text on the image; `None` when text-free.
    pub language: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The provider's own rating, higher is better; comparable only within
    /// one provider.
    pub score: Option<f32>,
}

#[async_trait]
pub trait ImageProvider: Send + Sync {
    fn id(&self) -> &str;

    /// Fetch raw image bytes for a given external ID + kind. The provider is
    /// responsible for any HTTP / authentication / rate-limiting. A person's
    /// id with [`ImageKind::Primary`] answers with their photo.
    async fn fetch(&self, external_id: &ExternalId, kind: ImageKind) -> PluginResult<Bytes>;

    /// Every `kind` image the provider has for `external_id`, best first.
    async fn list(
        &self,
        external_id: &ExternalId,
        kind: ImageKind,
    ) -> PluginResult<Vec<ImageCandidate>>;

    /// The image a [`Self::list`] candidate's `handle` names: a small
    /// rendition for a picker when `preview`, else the original.
    async fn fetch_candidate(&self, handle: &str, preview: bool) -> PluginResult<Bytes>;
}
