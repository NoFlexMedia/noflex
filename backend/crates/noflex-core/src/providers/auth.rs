// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{error::PluginResult, ids::UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCredentials {
    pub username: String,
    /// Plaintext password in the canonical form. Provider impls hash and
    /// compare against their own stored format.
    pub password: String,
    pub device: Option<String>,
    pub client: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthVerdict {
    Authenticated {
        user_id: UserId,
        display_name: String,
        is_admin: bool,
    },
    /// Provider knows the user but rejects the credentials.
    Rejected,
    /// Provider does not know about this user; the next provider should try.
    NotMine,
}

#[async_trait]
pub trait AuthProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn authenticate(&self, creds: &AuthCredentials) -> PluginResult<AuthVerdict>;
}
