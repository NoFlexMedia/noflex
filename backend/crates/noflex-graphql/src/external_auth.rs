// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions as _, Object, SimpleObject};
use noflex_auth::Principal;
use noflex_core::UserId;
use noflex_plugin_host::{HookKind, PluginRegistry};
use noflex_storage::{Storage, StorageError, domain::ExternalIdentity};

use crate::auth::{LoginResult, mint_session};

/// `extensions.code` of an unlink that would leave an account with no way to
/// sign in.
const LAST_SIGN_IN_METHOD: &str = "LAST_SIGN_IN_METHOD";

#[derive(SimpleObject)]
pub struct ExternalAuthProvider {
    id: String,
    display_name: String,
}

/// Where to send the browser to begin an external login or link.
#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "ExternalAuthStart")]
pub struct ExternalAuthChallenge {
    pub authorization_url: String,
}

/// External login/link failures. `Display` strings are stable and
/// client-facing; detail for operators goes to the log, not the error.
#[derive(Debug, thiserror::Error)]
pub enum ExternalAuthError {
    /// Server-side configuration (e.g. `public_url`) is missing or invalid.
    #[error("external authentication is unavailable: {0}")]
    Unavailable(String),
    #[error("setup must be completed before external login")]
    SetupRequired,
    #[error("external auth provider is not available")]
    ProviderUnavailable,
    #[error("external auth provider rejected the request")]
    ProviderRejected,
    #[error("clientNonce must be 32 to 512 characters")]
    InvalidNonce,
    #[error("too many external logins in progress; try again later")]
    Busy,
    /// Unknown, expired, replayed, or not bound to this browser/user.
    #[error("external login code is invalid or expired")]
    InvalidCode,
    #[error("no NoFlex account is linked to this identity; ask an admin")]
    NotLinked,
    #[error("this external identity is already linked to another account")]
    AlreadyLinked,
    #[error("internal error")]
    Storage(#[from] StorageError),
    #[error("internal error")]
    Internal(String),
}

/// The external login/link state machine. Every flow is bound to the
/// browser that started it: `start_*` takes a client-generated secret
/// `client_nonce`, and the matching `complete_*` must present it again
/// with the single-use exchange code the provider callback hands that
/// browser.
#[async_trait::async_trait]
pub trait ExternalAuthService: Send + Sync {
    /// Whether any flow can start: the server has a valid public URL for
    /// providers to call back to. Until it does, every `start_*` fails with
    /// [`ExternalAuthError::Unavailable`].
    fn available(&self) -> bool;

    /// Begin a login with `provider`.
    async fn start_login(
        &self,
        provider: &str,
        client_nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError>;

    /// Begin linking a `provider` identity to `user`.
    async fn start_link(
        &self,
        provider: &str,
        user: UserId,
        client_nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError>;

    /// Spend a login exchange code, returning the account to sign in.
    /// The code is consumed even when the call fails.
    ///
    /// # Errors
    /// [`ExternalAuthError::InvalidCode`] for unknown, expired, link, or
    /// nonce-mismatched codes; [`ExternalAuthError::NotLinked`] when the
    /// identity has no account and provisioning is disabled.
    async fn complete_login(
        &self,
        code: &str,
        client_nonce: &str,
    ) -> Result<UserId, ExternalAuthError>;

    /// Spend a link exchange code and attach its identity to `user`.
    /// The code is consumed even when the call fails.
    ///
    /// # Errors
    /// [`ExternalAuthError::InvalidCode`] unless the code was started by
    /// `user` with this nonce; [`ExternalAuthError::AlreadyLinked`] when
    /// another account owns the identity.
    async fn complete_link(
        &self,
        code: &str,
        client_nonce: &str,
        user: UserId,
    ) -> Result<ExternalIdentity, ExternalAuthError>;
}

fn gql_error(error: ExternalAuthError) -> async_graphql::Error {
    if let ExternalAuthError::Storage(source) = &error {
        tracing::error!(error = %source, "external auth storage failure");
    } else if let ExternalAuthError::Internal(detail) = &error {
        tracing::error!(%detail, "external auth internal failure");
    }
    async_graphql::Error::new(error.to_string())
}

#[derive(Default)]
pub struct ExternalAuthQuery;

#[Object]
impl ExternalAuthQuery {
    /// The providers a sign-in screen can offer. Empty while the server can't
    /// start a flow with any of them, so no client shows a button that fails.
    async fn external_auth_providers(&self, ctx: &Context<'_>) -> Vec<ExternalAuthProvider> {
        let available = ctx
            .data::<Arc<dyn ExternalAuthService>>()
            .is_ok_and(|service| service.available());
        let (true, Ok(registry)) = (available, ctx.data::<Arc<PluginRegistry>>()) else {
            return Vec::new();
        };
        registry
            .snapshot()
            .into_iter()
            .filter(|plugin| plugin.hooks.contains(&HookKind::Auth))
            .map(|plugin| ExternalAuthProvider {
                id: plugin.id,
                display_name: plugin.manifest.display_name,
            })
            .collect()
    }

    async fn my_external_identities(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<ExternalIdentity>> {
        let user_id = authenticated(ctx)?;
        let storage = ctx.data::<Arc<dyn Storage>>()?;
        storage
            .list_external_identities(user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}

#[derive(Default)]
pub struct ExternalAuthMutation;

#[Object]
impl ExternalAuthMutation {
    /// Begin an external login. `clientNonce` is a fresh random secret
    /// (at least 32 characters) the browser keeps until
    /// `completeExternalLogin`. The provider callback redirects to
    /// `/#external-code=<code>`.
    async fn start_external_login(
        &self,
        ctx: &Context<'_>,
        provider: String,
        client_nonce: String,
    ) -> async_graphql::Result<ExternalAuthChallenge> {
        let service = ctx.data::<Arc<dyn ExternalAuthService>>()?;
        service
            .start_login(&provider, &client_nonce)
            .await
            .map_err(gql_error)
    }

    async fn complete_external_login(
        &self,
        ctx: &Context<'_>,
        code: String,
        client_nonce: String,
        client: Option<String>,
        device: Option<String>,
    ) -> async_graphql::Result<LoginResult> {
        let service = ctx.data::<Arc<dyn ExternalAuthService>>()?;
        let user_id = service
            .complete_login(&code, &client_nonce)
            .await
            .map_err(gql_error)?;
        let storage = ctx.data::<Arc<dyn Storage>>()?;
        let user = storage
            .get_user(user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .ok_or_else(|| gql_error(ExternalAuthError::InvalidCode))?;
        let (token, refresh_token) = mint_session(ctx, &user, client, device).await?;
        Ok(LoginResult {
            token,
            refresh_token,
            user,
        })
    }

    /// Begin linking an external identity to the caller. The provider
    /// callback redirects to `/#external-link-code=<code>`; nothing is
    /// linked until `completeExternalIdentityLink`.
    async fn start_external_identity_link(
        &self,
        ctx: &Context<'_>,
        provider: String,
        client_nonce: String,
    ) -> async_graphql::Result<ExternalAuthChallenge> {
        let user_id = authenticated(ctx)?;
        let service = ctx.data::<Arc<dyn ExternalAuthService>>()?;
        service
            .start_link(&provider, user_id, &client_nonce)
            .await
            .map_err(gql_error)
    }

    async fn complete_external_identity_link(
        &self,
        ctx: &Context<'_>,
        code: String,
        client_nonce: String,
    ) -> async_graphql::Result<ExternalIdentity> {
        let user_id = authenticated(ctx)?;
        let service = ctx.data::<Arc<dyn ExternalAuthService>>()?;
        service
            .complete_link(&code, &client_nonce, user_id)
            .await
            .map_err(gql_error)
    }

    /// Unlink the caller's identity with a provider. Returns it, or null when
    /// they had none.
    ///
    /// Refused with code `LAST_SIGN_IN_METHOD` when it's their last identity
    /// and the account has no password, as nothing could sign in to it again.
    // Two concurrent unlinks of an account's last two identities can both pass
    // the check; acceptable until it moves into storage.
    async fn revoke_external_identity(
        &self,
        ctx: &Context<'_>,
        provider: String,
    ) -> async_graphql::Result<Option<ExternalIdentity>> {
        let user_id = authenticated(ctx)?;
        let storage = ctx.data::<Arc<dyn Storage>>()?;
        let mut identities = storage
            .list_external_identities(user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let Some(index) = identities
            .iter()
            .position(|identity| identity.plugin_id == provider)
        else {
            return Ok(None);
        };
        let identity = identities.swap_remove(index);
        if identities.is_empty() {
            let user = storage
                .get_user(user_id)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?
                .ok_or_else(|| async_graphql::Error::new("user not found"))?;
            if user.password_hash.is_empty() {
                return Err(last_sign_in_method());
            }
        }
        let removed = storage
            .unlink_external_identity(user_id, &provider)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(removed.then_some(identity))
    }
}

fn last_sign_in_method() -> async_graphql::Error {
    async_graphql::Error::new("this is the only way you can sign in, so it can't be disconnected")
        .extend_with(|_, ext| ext.set("code", LAST_SIGN_IN_METHOD))
}

fn authenticated(ctx: &Context<'_>) -> async_graphql::Result<UserId> {
    let principal = ctx
        .data::<Principal>()
        .map_err(|_| async_graphql::Error::new("authentication required"))?;
    if !principal.is_authenticated() {
        return Err(async_graphql::Error::new("authentication required"));
    }
    Ok(principal.user_id)
}
