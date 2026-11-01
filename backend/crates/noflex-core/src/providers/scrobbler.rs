// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    ids::{UserId, WorkId},
    model::{ExternalId, WorkKind},
};

/// Scrobble state machine (the Trakt `/scrobble/start|pause|stop` shape).
/// `Paused` exists on the wire but is not emitted yet — Playing/Paused
/// heartbeats are too hot to broadcast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrobbleAction {
    Started,
    Paused,
    Stopped,
}

/// One scrobble occurrence, enriched by the host from storage so scrobbler
/// plugins need no library lookups of their own. `position_ms` is present
/// only for progress-at-stop; `runtime_ms` is the edition runtime when the
/// host could resolve one — the plugin derives its own progress percent and
/// watched threshold from the pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrobbleEvent {
    pub user: UserId,
    pub work: WorkId,
    pub kind: WorkKind,
    pub title: String,
    pub year: Option<i32>,
    /// Provider ids the work carries (tmdb, tvdb, imdb, ...) — scrobblers
    /// match on these, never on titles.
    pub external_ids: Vec<ExternalId>,
    pub action: ScrobbleAction,
    pub position_ms: Option<u64>,
    pub runtime_ms: Option<u64>,
}

/// Per-user watch-history sync (Trakt, ...). The host derives events from
/// the domain bus and skips users who never linked an account.
#[async_trait]
pub trait Scrobbler: Send + Sync {
    fn id(&self) -> &str;

    async fn scrobble(&self, event: ScrobbleEvent) -> PluginResult<()>;
}
