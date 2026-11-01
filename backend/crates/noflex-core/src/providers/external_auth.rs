// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::PluginResult;

/// An identity the provider verified with the upstream IdP. `(issuer,
/// subject)` is the stable key; the other claims are display hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedIdentity {
    pub issuer: String,
    pub subject: String,
    pub preferred_username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

/// Interactive login through an external identity provider (OIDC, ...).
/// Mirrors the WIT `external-auth` interface. The host owns `state` and
/// the PKCE material and validates both itself; the provider owns
/// discovery, the code exchange, and the identity lookup. Distinct from
/// [`super::AuthProvider`], which checks username/password credentials.
#[async_trait]
pub trait ExternalAuthProvider: Send + Sync {
    fn id(&self) -> &str;

    /// Returns the authorization URL the user is redirected to. It must
    /// carry `state`, `callback_uri`, and `pkce_challenge` (S256) through.
    async fn start(
        &self,
        callback_uri: String,
        state: String,
        pkce_challenge: String,
    ) -> PluginResult<String>;

    /// Exchanges the callback's `code` (with the matching `pkce_verifier`)
    /// and returns the verified identity.
    async fn complete(
        &self,
        callback_uri: String,
        code: String,
        pkce_verifier: String,
    ) -> PluginResult<VerifiedIdentity>;
}
