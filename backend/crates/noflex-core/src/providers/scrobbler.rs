// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    ids::{UserId, WorkId},
    model::{ExternalId, WorkKind},
};

/// Scrobble state machine (the Trakt `/scrobble/start|pause|stop` shape).
/// `Paused` comes once when a session turns from playing to paused, and
/// `Started` again when it turns back, never per heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrobbleAction {
    Started,
    Paused,
    Stopped,
}

/// One scrobble occurrence, enriched by the host from storage so scrobbler
/// plugins need no library lookups of their own. `position_ms` is present
/// for a pause, a resume and progress-at-stop, and a `Stopped` without it is
/// a title marked played; `runtime_ms` is the edition runtime when the host
/// could resolve one — the plugin derives its own progress percent and
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
    /// For an episode, its series' provider ids; empty otherwise.
    pub series_external_ids: Vec<ExternalId>,
    /// For an episode, its season's number and its own within it.
    pub season_number: Option<u32>,
    pub episode_number: Option<u32>,
    pub action: ScrobbleAction,
    pub position_ms: Option<u64>,
    pub runtime_ms: Option<u64>,
}

/// Per-user scrobbling (Trakt, ...). The host derives events from
/// the domain bus and skips users who never linked an account.
#[async_trait]
pub trait Scrobbler: Send + Sync {
    fn id(&self) -> &str;

    async fn scrobble(&self, event: ScrobbleEvent) -> PluginResult<()>;
}
