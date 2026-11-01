// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{error::PluginResult, ids::UserId};

/// Where the user goes to authorize a link, plus the opaque state the host
/// binds to the linking user and echoes back to `link_complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkChallenge {
    /// OAuth authorize or device-code page, opened out-of-band.
    pub auth_url: String,
    pub state: String,
}

/// One per-user credential the host persists for the linking user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSecret {
    pub key: String,
    pub value: String,
}

/// Per-user account linking (Trakt-style OAuth). Mirrors the WIT
/// `user-link` interface. The provider owns the upstream dance; the host
/// owns token storage, so providers hand tokens back instead of persisting
/// them, and read them later through the host's per-user secret import.
/// Both calls run bound to `user`: per-user secrets of any other user are
/// refused.
#[async_trait]
pub trait UserLinkProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn link_start(&self, user: UserId) -> PluginResult<LinkChallenge>;

    /// `callback_query` is the raw query string the upstream redirect
    /// delivered. An empty result means "linked, nothing to store" (device
    /// flows that deliver tokens later from a background task).
    async fn link_complete(
        &self,
        user: UserId,
        state: String,
        callback_query: String,
    ) -> PluginResult<Vec<UserSecret>>;
}
