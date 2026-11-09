// SPDX-License-Identifier: AGPL-3.0-only
//
// Long-lived credentials and device linking. API keys are shown to
// the user exactly once — storage only ever sees the SHA-256 hex, the
// same treatment refresh tokens get in `auth.rs`. Device codes are the
// Plex-style link flow: an unauthenticated device displays a short
// code, a signed-in user authorizes it, and the device polls until
// the code trades for an ordinary session token pair.

use std::sync::Arc;

use async_graphql::{Context, Object, SimpleObject};
use noflex_core::{ApiKeyId, DeviceCodeId, UserId};
use noflex_storage::{
    Storage, StorageError,
    domain::{ActivityKind, ApiKey, DeviceCode},
};
use time::OffsetDateTime;

use crate::{
    auth::{LoginResult, mint_session},
    users::authed_principal,
};

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

/// An API key at the only moment its plaintext exists: the create
/// response. Listings show the `api_key` row, never `key`.
#[derive(SimpleObject)]
pub struct CreatedApiKey {
    pub key: String,
    pub api_key: ApiKey,
}

/// The device's half of the link flow: show `code` on screen, then
/// poll `pollDeviceCode` until it yields a session or `expires_at`
/// passes.
#[derive(SimpleObject)]
pub struct DeviceCodeInfo {
    pub code: String,
    pub expires_at: OffsetDateTime,
}

const CODE_LEN: usize = 6;
const CODE_TTL: time::Duration = time::Duration::minutes(10);
/// Glyphs that survive being read off a TV and retyped by hand: no
/// 0/O, no 1/I/L.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// Collision rerolls before giving up on a fresh code.
const CODE_CREATE_ATTEMPTS: u32 = 5;

fn generate_code() -> String {
    // OS randomness arrives hex-encoded (the refresh-token generator);
    // decode pairs back to bytes and reject the 248..=255 tail so the
    // alphabet index stays unbiased (248 = 8 * CODE_ALPHABET.len()).
    let mut code = String::with_capacity(CODE_LEN);
    while code.len() < CODE_LEN {
        for pair in noflex_auth::generate_refresh_token()
            .into_bytes()
            .as_chunks::<2>()
            .0
        {
            let (Some(hi), Some(lo)) = (
                (pair[0] as char).to_digit(16),
                (pair[1] as char).to_digit(16),
            ) else {
                continue;
            };
            let byte = ((hi << 4) | lo) as u8;
            if byte >= 248 {
                continue;
            }
            code.push(CODE_ALPHABET[(byte % CODE_ALPHABET.len() as u8) as usize] as char);
            if code.len() == CODE_LEN {
                break;
            }
        }
    }
    code
}

#[derive(Default)]
pub struct CredentialsQuery;

#[Object]
impl CredentialsQuery {
    /// Every API key the caller owns, active and revoked alike,
    /// newest first. Filter on `revokedAt`.
    async fn api_keys(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<ApiKey>> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .list_api_keys(principal.user_id)
            .await
            .map_err(gql_err)
    }
}

#[derive(Default)]
pub struct CredentialsMutation;

#[Object]
impl CredentialsMutation {
    /// Mint an API key for the calling user. The plaintext `key`
    /// appears in this response and nowhere else, ever.
    async fn create_api_key(
        &self,
        ctx: &Context<'_>,
        name: String,
    ) -> async_graphql::Result<CreatedApiKey> {
        let principal = authed_principal(ctx)?;
        if name.trim().is_empty() {
            return Err(async_graphql::Error::new("name is required"));
        }
        let storage = storage(ctx)?;
        let key = noflex_auth::generate_api_key();
        let key_hash = noflex_auth::hash_api_key(&key);
        let api_key = storage
            .create_api_key(principal.user_id, &name, &key_hash)
            .await
            .map_err(gql_err)?;
        Ok(CreatedApiKey { key, api_key })
    }

    /// Revoke an API key. Returns it, stamped with `revokedAt`, or null
    /// when the id was unknown or already revoked. Non-admins can only
    /// reach their own keys and get a uniform "not found" otherwise, so
    /// keys aren't enumerable; admins revoke by id directly.
    async fn revoke_api_key(
        &self,
        ctx: &Context<'_>,
        id: ApiKeyId,
    ) -> async_graphql::Result<Option<ApiKey>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let key = storage.get_api_key(id).await.map_err(gql_err)?;
        let owns = key
            .as_ref()
            .is_some_and(|key| key.user_id == principal.user_id);
        if !owns && !principal.is_admin {
            return Err(async_graphql::Error::new("api key not found"));
        }
        if !storage.revoke_api_key(id).await.map_err(gql_err)? {
            return Ok(None);
        }
        storage.get_api_key(id).await.map_err(gql_err)
    }

    /// Begin the device-link flow: an unauthenticated device (a TV
    /// with no credentials yet) asks for a code to show on screen.
    async fn create_device_code(
        &self,
        ctx: &Context<'_>,
        client: Option<String>,
        device: Option<String>,
    ) -> async_graphql::Result<DeviceCodeInfo> {
        let storage = storage(ctx)?;
        let now = OffsetDateTime::now_utc();
        for attempt in 0..CODE_CREATE_ATTEMPTS {
            let code = DeviceCode {
                id: DeviceCodeId::new(),
                code: generate_code(),
                user_id: None,
                client: client.clone(),
                device: device.clone(),
                created_at: now,
                expires_at: now + CODE_TTL,
            };
            match storage.create_device_code(&code).await {
                Ok(()) => {
                    return Ok(DeviceCodeInfo {
                        code: code.code,
                        expires_at: code.expires_at,
                    });
                }
                // A colliding code just rerolls.
                Err(StorageError::Constraint(_)) if attempt + 1 < CODE_CREATE_ATTEMPTS => {}
                Err(e) => return Err(gql_err(e)),
            }
        }
        Err(async_graphql::Error::new(
            "could not allocate a device code",
        ))
    }

    /// Approve a pending code, signing the device showing it in as the
    /// caller. An administrator can pass `userId` to sign it in as someone
    /// else instead, for a child with no device of their own to approve
    /// from; that goes in the activity log. `false` for unknown, expired, or
    /// already-authorized codes — deliberately indistinguishable.
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn authorize_device_code(
        &self,
        ctx: &Context<'_>,
        code: String,
        user_id: Option<UserId>,
    ) -> async_graphql::Result<bool> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let Some(other) = user_id.filter(|id| *id != principal.user_id) else {
            return storage
                .authorize_device_code(&code, principal.user_id)
                .await
                .map_err(gql_err);
        };
        crate::access::require_admin(ctx)?;
        if storage.get_user(other).await.map_err(gql_err)?.is_none() {
            return Err(async_graphql::Error::new("user not found"));
        }
        let pending = storage.device_code_by_code(&code).await.map_err(gql_err)?;
        if !storage
            .authorize_device_code(&code, other)
            .await
            .map_err(gql_err)?
        {
            return Ok(false);
        }
        let detail = serde_json::json!({
            "client": pending.as_ref().and_then(|row| row.client.as_deref()),
            "device": pending.as_ref().and_then(|row| row.device.as_deref()),
        })
        .to_string();
        // The approval stands either way; an unrecorded one is logged.
        if let Err(error) = storage
            .append_activity(
                ActivityKind::DeviceApprovedFor,
                Some(&principal.user_id.to_string()),
                Some(&other.to_string()),
                Some(&detail),
                OffsetDateTime::now_utc(),
            )
            .await
        {
            tracing::error!(
                admin = %principal.user_id,
                user = %other,
                %error,
                "recording a device approved for someone else failed"
            );
        }
        Ok(true)
    }

    /// The device side of the link flow, unauthenticated. Null while
    /// the code is pending, unknown, or expired. Once authorized, the
    /// first poll mints a session for the bound user (carrying the
    /// code's client/device), consumes the code, and returns the same
    /// token pair `login` issues — later polls see null again.
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn poll_device_code(
        &self,
        ctx: &Context<'_>,
        code: String,
    ) -> async_graphql::Result<Option<LoginResult>> {
        let storage = storage(ctx)?;
        let Some(row) = storage.device_code_by_code(&code).await.map_err(gql_err)? else {
            return Ok(None);
        };
        let Some(user_id) = row.user_id else {
            return Ok(None);
        };
        let Some(user) = storage.get_user(user_id).await.map_err(gql_err)? else {
            return Ok(None);
        };
        let (token, refresh_token) = mint_session(ctx, &user, row.client, row.device).await?;
        // One-shot: the code dies the moment it mints a session.
        storage.delete_device_code(row.id).await.map_err(gql_err)?;
        Ok(Some(LoginResult {
            token,
            refresh_token,
            user,
        }))
    }
}
