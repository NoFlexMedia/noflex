// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use axum::response::{IntoResponse, Redirect, Response};
use noflex_core::{UserId, UserRole};
use noflex_graphql::{ExternalAuthChallenge, ExternalAuthError, ExternalAuthService};
use noflex_plugin_host::{
    HookKind, PluginId, PluginRegistry,
    external_auth::exports::noflex::plugin::external_auth::VerifiedIdentity,
};
use noflex_storage::{
    Storage,
    domain::{ExternalIdentity, User},
    error::StorageError,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use url::{Host, Url};

const STATE_TTL: time::Duration = time::Duration::minutes(10);
const EXCHANGE_TTL: time::Duration = time::Duration::minutes(2);
/// Bounds memory and plugin dispatches under an unauthenticated
/// `start` flood: once full, new flows are refused until entries expire.
const MAX_IN_FLIGHT: usize = 1024;
const NONCE_LEN: std::ops::RangeInclusive<usize> = 32..=512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Login,
    Link(UserId),
}

struct Pending {
    plugin_id: String,
    callback_uri: String,
    verifier: String,
    purpose: Purpose,
    nonce_hash: [u8; 32],
    expires_at: OffsetDateTime,
}

struct Verified {
    issuer: String,
    subject: String,
    username: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
}

struct Exchange {
    plugin_id: String,
    identity: Verified,
    purpose: Purpose,
    nonce_hash: [u8; 32],
    expires_at: OffsetDateTime,
}

/// In-flight external-auth state is deliberately memory-only. A restart
/// invalidates provider callbacks and unconsumed exchange codes.
pub struct ServerExternalAuth {
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    public_url: Result<String, String>,
    auto_provision: bool,
    pending: Mutex<HashMap<String, Pending>>,
    exchanges: Mutex<HashMap<String, Exchange>>,
}

impl ServerExternalAuth {
    /// `auto_provision` lets a verified identity with no linked account
    /// create one; otherwise only linked identities may log in.
    pub fn new(
        registry: Arc<PluginRegistry>,
        storage: Arc<dyn Storage>,
        public_url: Option<String>,
        auto_provision: bool,
    ) -> Self {
        Self {
            registry,
            storage,
            public_url: validate_public_url(public_url.as_deref()),
            auto_provision,
            pending: Mutex::new(HashMap::new()),
            exchanges: Mutex::new(HashMap::new()),
        }
    }

    /// Finish the provider round-trip for `state`, returning the SPA
    /// redirect carrying a single-use exchange code
    /// (`/#external-code=` or `/#external-link-code=`). Nothing is
    /// provisioned or linked here: that waits for the browser that
    /// started the flow to present the code with its nonce.
    pub async fn callback(
        &self,
        plugin_id: &str,
        state: &str,
        code: &str,
    ) -> Result<String, ExternalAuthError> {
        let pending = {
            let mut pending = lock(&self.pending)?;
            sweep(&mut pending, |value| value.expires_at);
            pending.remove(state)
        }
        .filter(|pending| pending.plugin_id == plugin_id)
        .ok_or(ExternalAuthError::InvalidCode)?;

        let plugin = self.auth_plugin(plugin_id)?;
        let verified = plugin
            .call_external_auth_complete(pending.callback_uri, code.to_owned(), pending.verifier)
            .await
            .map_err(|error| {
                tracing::warn!(plugin = plugin_id, %error, "external auth dispatch failed");
                ExternalAuthError::ProviderUnavailable
            })?
            .map_err(|error| {
                tracing::warn!(
                    plugin = plugin_id,
                    ?error,
                    "external auth rejected callback"
                );
                ExternalAuthError::ProviderRejected
            })?;
        let VerifiedIdentity {
            issuer,
            subject,
            preferred_username,
            display_name,
            email,
        } = verified;
        if issuer.is_empty() || subject.is_empty() {
            tracing::warn!(
                plugin = plugin_id,
                "provider returned an empty issuer or subject"
            );
            return Err(ExternalAuthError::ProviderRejected);
        }

        let exchange = noflex_auth::generate_refresh_token();
        {
            let mut exchanges = lock(&self.exchanges)?;
            sweep(&mut exchanges, |value| value.expires_at);
            if exchanges.len() >= MAX_IN_FLIGHT {
                return Err(ExternalAuthError::Busy);
            }
            exchanges.insert(
                exchange.clone(),
                Exchange {
                    plugin_id: plugin_id.to_owned(),
                    identity: Verified {
                        issuer,
                        subject,
                        username: preferred_username,
                        display_name,
                        email,
                    },
                    purpose: pending.purpose,
                    nonce_hash: pending.nonce_hash,
                    expires_at: OffsetDateTime::now_utc() + EXCHANGE_TTL,
                },
            );
        }
        let fragment = match pending.purpose {
            Purpose::Login => "external-code",
            Purpose::Link(_) => "external-link-code",
        };
        Ok(format!("/#{fragment}={}", percent_encode(&exchange)))
    }

    fn auth_plugin(
        &self,
        id: &str,
    ) -> Result<Arc<noflex_plugin_host::LoadedPlugin>, ExternalAuthError> {
        self.registry
            .get_plugin(&PluginId(id.to_owned()))
            .filter(|plugin| plugin.exports(HookKind::Auth))
            .ok_or(ExternalAuthError::ProviderUnavailable)
    }

    async fn start(
        &self,
        provider: &str,
        purpose: Purpose,
        client_nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError> {
        let nonce_hash = nonce_hash(client_nonce)?;
        let callback_uri = self.callback_uri(provider)?;
        if purpose == Purpose::Login && self.storage.list_users().await?.is_empty() {
            return Err(ExternalAuthError::SetupRequired);
        }
        let plugin = self.auth_plugin(provider)?;
        // Refuse before dispatching so a flood can't drive plugin calls.
        drop(self.pending_slot()?);
        let state = noflex_auth::generate_refresh_token();
        let verifier = noflex_auth::generate_refresh_token();
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        let authorization_url = plugin
            .call_external_auth_start(callback_uri.clone(), state.clone(), challenge)
            .await
            .map_err(|error| {
                tracing::warn!(plugin = provider, %error, "external auth dispatch failed");
                ExternalAuthError::ProviderUnavailable
            })?
            .map_err(|error| {
                tracing::warn!(
                    plugin = provider,
                    ?error,
                    "external auth provider rejected start"
                );
                ExternalAuthError::ProviderRejected
            })?;
        self.pending_slot()?.insert(
            state,
            Pending {
                plugin_id: provider.to_owned(),
                callback_uri,
                verifier,
                purpose,
                nonce_hash,
                expires_at: OffsetDateTime::now_utc() + STATE_TTL,
            },
        );
        Ok(ExternalAuthChallenge { authorization_url })
    }

    /// Sweep expired flows and lock the pending map iff it has room.
    fn pending_slot(&self) -> Result<MutexGuard<'_, HashMap<String, Pending>>, ExternalAuthError> {
        let mut pending = lock(&self.pending)?;
        sweep(&mut pending, |value| value.expires_at);
        if pending.len() >= MAX_IN_FLIGHT {
            return Err(ExternalAuthError::Busy);
        }
        Ok(pending)
    }

    /// Remove `code` (spending it whatever happens next) and return it
    /// only if it was minted for `purpose` and `client_nonce`.
    fn take_exchange(
        &self,
        code: &str,
        client_nonce: &str,
        purpose: Purpose,
    ) -> Result<Exchange, ExternalAuthError> {
        let presented = Sha256::digest(client_nonce.as_bytes());
        let exchange = {
            let mut exchanges = lock(&self.exchanges)?;
            sweep(&mut exchanges, |value| value.expires_at);
            exchanges.remove(code)
        }
        .ok_or(ExternalAuthError::InvalidCode)?;
        if exchange.purpose != purpose || !constant_time_eq(&exchange.nonce_hash, &presented) {
            return Err(ExternalAuthError::InvalidCode);
        }
        Ok(exchange)
    }

    async fn provision(
        &self,
        plugin_id: &str,
        verified: &Verified,
    ) -> Result<User, ExternalAuthError> {
        let base = username_base(
            verified.username.as_deref(),
            verified.display_name.as_deref(),
            verified.email.as_deref(),
        );
        let suffix = deterministic_suffix(plugin_id, &verified.issuer, &verified.subject);
        for attempt in 0..16u8 {
            let name = if attempt == 0 {
                base.clone()
            } else if attempt == 1 {
                format!("{base}-{suffix}")
            } else {
                format!("{base}-{suffix}-{attempt}")
            };
            let user = User {
                id: UserId::new(),
                name,
                password_hash: String::new(),
                is_admin: false,
                role: UserRole::User,
                created_at: OffsetDateTime::now_utc(),
            };
            match self.storage.upsert_user(&user).await {
                Ok(()) => match self
                    .storage
                    .link_external_identity(&identity(plugin_id, user.id, verified))
                    .await
                {
                    Ok(()) => return Ok(user),
                    Err(StorageError::Conflict(_)) => {
                        // Lost a race with a concurrent login for the
                        // same identity; adopt the winner's account.
                        self.storage.delete_user(user.id).await.ok();
                        if let Some(user) = self.linked_user(plugin_id, verified).await? {
                            return Ok(user);
                        }
                    }
                    Err(error) => {
                        self.storage.delete_user(user.id).await.ok();
                        return Err(error.into());
                    }
                },
                Err(StorageError::Constraint(_)) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(ExternalAuthError::Internal(
            "unable to allocate an external account username".into(),
        ))
    }

    async fn linked_user(
        &self,
        plugin_id: &str,
        verified: &Verified,
    ) -> Result<Option<User>, ExternalAuthError> {
        let Some(row) = self
            .storage
            .external_identity(plugin_id, &verified.issuer, &verified.subject)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(self.storage.get_user(row.user_id).await?.ok_or_else(
            || ExternalAuthError::Internal("external identity has no user".into()),
        )?))
    }

    fn callback_uri(&self, plugin_id: &str) -> Result<String, ExternalAuthError> {
        let base = self
            .public_url
            .as_ref()
            .map_err(|error| ExternalAuthError::Unavailable(error.clone()))?;
        Ok(format!(
            "{base}/auth/{}/callback",
            percent_encode(plugin_id)
        ))
    }
}

#[async_trait::async_trait]
impl ExternalAuthService for ServerExternalAuth {
    async fn start_login(
        &self,
        provider: &str,
        client_nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError> {
        self.start(provider, Purpose::Login, client_nonce).await
    }

    async fn start_link(
        &self,
        provider: &str,
        user: UserId,
        client_nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError> {
        self.start(provider, Purpose::Link(user), client_nonce)
            .await
    }

    async fn complete_login(
        &self,
        code: &str,
        client_nonce: &str,
    ) -> Result<UserId, ExternalAuthError> {
        let exchange = self.take_exchange(code, client_nonce, Purpose::Login)?;
        if self.storage.list_users().await?.is_empty() {
            return Err(ExternalAuthError::SetupRequired);
        }
        if let Some(user) = self
            .linked_user(&exchange.plugin_id, &exchange.identity)
            .await?
        {
            return Ok(user.id);
        }
        if !self.auto_provision {
            return Err(ExternalAuthError::NotLinked);
        }
        Ok(self
            .provision(&exchange.plugin_id, &exchange.identity)
            .await?
            .id)
    }

    async fn complete_link(
        &self,
        code: &str,
        client_nonce: &str,
        user: UserId,
    ) -> Result<ExternalIdentity, ExternalAuthError> {
        let exchange = self.take_exchange(code, client_nonce, Purpose::Link(user))?;
        let linked = identity(&exchange.plugin_id, user, &exchange.identity);
        match self.storage.link_external_identity(&linked).await {
            Ok(()) => Ok(linked),
            Err(StorageError::Conflict(_)) => Err(ExternalAuthError::AlreadyLinked),
            Err(error) => Err(error.into()),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, ExternalAuthError> {
    mutex
        .lock()
        .map_err(|_| ExternalAuthError::Internal("external auth state lock poisoned".into()))
}

fn nonce_hash(client_nonce: &str) -> Result<[u8; 32], ExternalAuthError> {
    if !NONCE_LEN.contains(&client_nonce.len()) {
        return Err(ExternalAuthError::InvalidNonce);
    }
    Ok(Sha256::digest(client_nonce.as_bytes()).into())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The SPA is served only at `/` and callbacks redirect to `/#…`, so a
/// sub-path `public_url` would hand the provider a callback the SPA
/// can't finish.
fn validate_public_url(configured: Option<&str>) -> Result<String, String> {
    let configured =
        configured.ok_or_else(|| "external authentication requires public_url".to_owned())?;
    let url = Url::parse(configured).map_err(|error| format!("invalid public_url: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err("invalid public_url: expected an absolute HTTP(S) URL".into());
    }
    if has_userinfo(configured) {
        return Err("invalid public_url: userinfo is not allowed".into());
    }
    if url.query().is_some() {
        return Err("invalid public_url: query is not allowed".into());
    }
    if url.fragment().is_some() {
        return Err("invalid public_url: fragment is not allowed".into());
    }
    if url.path() != "/" {
        return Err("invalid public_url: a path is not supported; serve NoFlex at the root".into());
    }
    if url.scheme() != "https" && !is_loopback(&url) {
        return Err("invalid public_url: HTTPS is required except for loopback hosts".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn has_userinfo(value: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority.contains('@'))
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(host)) => host.octets() == [127, 0, 0, 1],
        Some(Host::Ipv6(host)) => host.is_loopback(),
        None => false,
    }
}

pub async fn callback_response(
    auth: &ServerExternalAuth,
    plugin_id: &str,
    state: Option<&str>,
    code: Option<&str>,
) -> Response {
    let (Some(state), Some(code)) = (state, code) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "external login failed or expired",
        )
            .into_response();
    };
    match auth.callback(plugin_id, state, code).await {
        Ok(location) => Redirect::to(&location).into_response(),
        Err(error) => {
            tracing::warn!(plugin = plugin_id, %error, "external auth callback failed");
            (
                axum::http::StatusCode::BAD_REQUEST,
                "external login failed or expired",
            )
                .into_response()
        }
    }
}

fn identity(plugin_id: &str, user_id: UserId, verified: &Verified) -> ExternalIdentity {
    ExternalIdentity {
        plugin_id: plugin_id.to_owned(),
        issuer: verified.issuer.clone(),
        subject: verified.subject.clone(),
        user_id,
        username: verified.username.clone(),
        display_name: verified.display_name.clone(),
        email: verified.email.clone(),
        linked_at: OffsetDateTime::now_utc(),
    }
}

fn username_base(preferred: Option<&str>, display: Option<&str>, email: Option<&str>) -> String {
    let source = preferred
        .filter(|value| !value.trim().is_empty())
        .or_else(|| display.filter(|value| !value.trim().is_empty()))
        .or_else(|| email.and_then(|value| value.split('@').next()))
        .unwrap_or("user");
    let sanitized = source
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['-', '.', '_'])
        .to_owned();
    if sanitized.is_empty() {
        "user".into()
    } else {
        sanitized
    }
}

fn deterministic_suffix(provider: &str, issuer: &str, subject: &str) -> String {
    let digest = Sha256::digest(format!("{provider}|{issuer}|{subject}"));
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let value = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[((value >> 18) & 63) as usize] as char);
        out.push(TABLE[((value >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((value >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(value & 63) as usize] as char);
        }
    }
    out
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn sweep<T>(map: &mut HashMap<String, T>, expires: impl Fn(&T) -> OffsetDateTime) {
    let now = OffsetDateTime::now_utc();
    map.retain(|_, value| expires(value) > now);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use noflex_graphql::ExternalAuthError;
    use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
    use noflex_storage::Storage;
    use noflex_storage_turso::TursoStore;
    use time::OffsetDateTime;

    use super::{
        MAX_IN_FLIGHT, Pending, Purpose, ServerExternalAuth, nonce_hash, validate_public_url,
    };

    const NONCE: &str = "test-nonce-0123456789abcdefghijklmnopqrstuvwxyz";

    async fn auth() -> ServerExternalAuth {
        let store: Arc<dyn Storage> = Arc::new(TursoStore::open_temp().await.expect("temp store"));
        let engine = build_engine(&EngineConfig::default()).expect("engine");
        let registry = Arc::new(PluginRegistry::new(engine).expect("registry"));
        ServerExternalAuth::new(registry, store, Some("https://media.example".into()), false)
    }

    fn pending(expires_at: OffsetDateTime) -> Pending {
        Pending {
            plugin_id: "missing".into(),
            callback_uri: "https://media.example/auth/missing/callback".into(),
            verifier: "verifier".into(),
            purpose: Purpose::Login,
            nonce_hash: nonce_hash(NONCE).expect("nonce"),
            expires_at,
        }
    }

    #[test]
    fn public_url_is_normalized_once() {
        assert_eq!(
            validate_public_url(Some("https://media.example/")).as_deref(),
            Ok("https://media.example")
        );
        assert_eq!(
            validate_public_url(Some("http://localhost:8096/")).as_deref(),
            Ok("http://localhost:8096")
        );
        assert_eq!(
            validate_public_url(Some("http://127.0.0.1:8096")).as_deref(),
            Ok("http://127.0.0.1:8096")
        );
        assert_eq!(
            validate_public_url(Some("http://[::1]:8096/")).as_deref(),
            Ok("http://[::1]:8096")
        );
    }

    #[test]
    fn public_url_rejects_unsafe_or_non_absolute_values() {
        for value in [
            "media.example",
            "ftp://media.example",
            "http://media.example",
            "http://127.0.0.2:8096",
            "https://user@media.example",
            "https://@media.example",
            "https://media.example/?tenant=one",
            "https://media.example/#fragment",
            "https://media.example/noflex",
            "https://media.example/base///",
        ] {
            assert!(
                validate_public_url(Some(value)).is_err(),
                "accepted {value}"
            );
        }
        assert_eq!(
            validate_public_url(None).expect_err("missing URL"),
            "external authentication requires public_url"
        );
    }

    #[test]
    fn nonce_length_is_enforced() {
        assert!(matches!(
            nonce_hash("short"),
            Err(ExternalAuthError::InvalidNonce)
        ));
        assert!(nonce_hash(&"x".repeat(513)).is_err());
        assert!(nonce_hash(NONCE).is_ok());
    }

    #[tokio::test]
    async fn callback_rejects_expired_state_before_dispatch() {
        let auth = auth().await;
        auth.pending.lock().expect("pending lock").insert(
            "expired".into(),
            pending(OffsetDateTime::now_utc() - time::Duration::SECOND),
        );
        assert!(auth.callback("missing", "expired", "code").await.is_err());
        assert!(auth.pending.lock().expect("pending lock").is_empty());
    }

    #[tokio::test]
    async fn pending_is_swept_then_capped() {
        let auth = auth().await;
        let live = || pending(OffsetDateTime::now_utc() + time::Duration::MINUTE);
        {
            let mut map = auth.pending.lock().expect("pending lock");
            map.insert(
                "stale".into(),
                pending(OffsetDateTime::now_utc() - time::Duration::SECOND),
            );
            for i in 0..MAX_IN_FLIGHT - 1 {
                map.insert(format!("live-{i}"), live());
            }
        }
        auth.pending_slot()
            .expect("stale entry swept, one slot left")
            .insert("last".into(), live());
        assert!(matches!(
            auth.pending_slot().map(drop),
            Err(ExternalAuthError::Busy)
        ));
    }
}
