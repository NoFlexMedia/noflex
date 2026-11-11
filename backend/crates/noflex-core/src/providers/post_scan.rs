// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;

use crate::{
    error::PluginResult,
    ids::{LibraryId, WorkId},
};

/// Runs after each library scan completes (or after items are added/removed).
/// Use for: rebuilding caches, computing embeddings, sending notifications.
#[async_trait]
pub trait PostScanTask: Send + Sync {
    fn id(&self) -> &str;

    async fn on_scan_complete(
        &self,
        library: LibraryId,
        added: &[WorkId],
        updated: &[WorkId],
        removed: &[WorkId],
    ) -> PluginResult<()>;
}
