// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use bytes::Bytes;

use crate::{
    error::PluginResult,
    model::{ExternalId, ImageKind},
};

#[async_trait]
pub trait ImageProvider: Send + Sync {
    fn id(&self) -> &str;

    /// Fetch raw image bytes for a given external ID + kind. The provider is
    /// responsible for any HTTP / authentication / rate-limiting.
    async fn fetch(&self, external_id: &ExternalId, kind: ImageKind) -> PluginResult<Bytes>;
}
