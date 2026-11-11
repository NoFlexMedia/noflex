// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    io::Write,
    sync::{Arc, LazyLock},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use noflex_core::{
    SessionId, UserId,
    providers::{AuthCredentials, AuthProvider, AuthVerdict},
};
use noflex_storage::Storage;
use sha1::{Digest, Sha1};
use thiserror::Error;
use time::OffsetDateTime;

mod throttle;
pub use throttle::{ClientAddr, LoginThrottle};

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("jwt encode: {0}")]
    JwtEncode(String),
    #[error("jwt decode: {0}")]
    JwtDecode(String),
    #[error("argon2: {0}")]
    Argon2(String),
    #[error("invalid hash format")]
    InvalidHash,
    #[error("password exceeds the maximum length")]
    PasswordTooLong,
    #[error("password hash exceeds the maximum length")]
    PasswordHashTooLong,
    #[error("password worker unavailable")]
    PasswordWorker,
    #[error("JWT signing key I/O at {path}: {source}")]
    SigningKeyIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("JWT signing key must contain at least {minimum} bytes (found {actual})")]
    WeakSigningKey { minimum: usize, actual: usize },
    #[error("JWT signing key permissions are too broad at {path}: {mode:o}")]
    InsecureSigningKeyPermissions { path: std::path::PathBuf, mode: u32 },
    #[error("missing or malformed authorization header")]
    MissingAuth,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Principal {
    pub user_id: UserId,
    pub is_admin: bool,
    /// The session row backing an access token. `None` for API-key and
    /// media-token principals, which no session backs, and for an
    /// anonymous principal.
    pub session_id: Option<SessionId>,
}

impl Principal {
    #[allow(dead_code)]
    pub fn anonymous() -> Self {
        Self::default()
    }
    #[allow(dead_code)]
    pub fn is_authenticated(&self) -> bool {
        !self.user_id.into_uuid().is_nil()
    }
}

// The nil UUID marks an anonymous principal. `UserId::default()`
// generates a fresh v7 UUID, which `is_authenticated` would treat
// as logged-in — so this must not derive.
impl Default for Principal {
    fn default() -> Self {
        Self {
            user_id: UserId::from_uuid(uuid::Uuid::nil()),
            is_admin: false,
            session_id: None,
        }
    }
}

pub struct LocalAuthProvider<S: Storage + ?Sized> {
    storage: Arc<S>,
}

impl<S: Storage + ?Sized> LocalAuthProvider<S> {
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }
}

#[async_trait::async_trait]
impl<S: Storage + Send + Sync + ?Sized> AuthProvider for LocalAuthProvider<S> {
    fn id(&self) -> &str {
        "local"
    }

    async fn authenticate(
        &self,
        creds: &AuthCredentials,
    ) -> noflex_core::error::PluginResult<AuthVerdict> {
        if creds.username.len() > MAX_USERNAME_BYTES || creds.password.len() > MAX_PASSWORD_BYTES {
            return Ok(AuthVerdict::Rejected);
        }
        let user = self
            .storage
            .get_user_by_username(&creds.username)
            .await
            .map_err(|e| noflex_core::error::PluginError::Internal(e.to_string()))?;
        let Some(user) = user else {
            return Ok(AuthVerdict::NotMine);
        };
        match verify_password(&creds.password, &user.password_hash).await {
            Ok(()) => {
                upgrade_legacy_hash(self.storage.as_ref(), &user, &creds.password).await;
                Ok(AuthVerdict::Authenticated {
                    user_id: user.id,
                    display_name: user.shown_name().to_string(),
                    is_admin: user.is_admin,
                })
            }
            Err(_) => Ok(AuthVerdict::Rejected),
        }
    }
}

/// Replace a legacy password hash on `user` with Argon2id once
/// `plaintext` has been verified against it. No-op for current hashes.
/// Best-effort: a failed rehash is logged and the login still succeeds,
/// since the legacy hash stays valid.
pub async fn upgrade_legacy_hash<S: Storage + ?Sized>(
    storage: &S,
    user: &noflex_storage::User,
    plaintext: &str,
) {
    if !password_needs_rehash(&user.password_hash) {
        return;
    }
    let hash = match hash_password(plaintext).await {
        Ok(hash) => hash,
        Err(error) => {
            tracing::warn!(user = %user.id, %error, "legacy password rehash failed");
            return;
        }
    };
    let mut upgraded = user.clone();
    upgraded.password_hash = hash;
    if let Err(error) = storage.upsert_user(&upgraded).await {
        tracing::warn!(user = %user.id, %error, "persisting rehashed password failed");
    }
}

pub const MAX_USERNAME_BYTES: usize = 256;
pub const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_PASSWORD_HASH_BYTES: usize = 4096;
const PASSWORD_WORKERS: usize = 4;

static PASSWORD_SEMAPHORE: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(PASSWORD_WORKERS)));

async fn run_password_job<T, F>(
    semaphore: Arc<tokio::sync::Semaphore>,
    job: F,
) -> Result<T, AuthError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AuthError> + Send + 'static,
{
    let permit = semaphore
        .acquire_owned()
        .await
        .map_err(|_| AuthError::PasswordWorker)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    })
    .await
    .map_err(|_| AuthError::PasswordWorker)?
}

pub async fn hash_password(plaintext: &str) -> Result<String, AuthError> {
    if plaintext.len() > MAX_PASSWORD_BYTES {
        return Err(AuthError::PasswordTooLong);
    }
    let plaintext = plaintext.as_bytes().to_vec();
    run_password_job(PASSWORD_SEMAPHORE.clone(), move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(&plaintext, &salt)
            .map(|h| h.to_string())
            .map_err(|e| AuthError::Argon2(e.to_string()))
    })
    .await
}

pub async fn verify_password(plaintext: &str, stored_hash: &str) -> Result<(), AuthError> {
    if plaintext.len() > MAX_PASSWORD_BYTES {
        return Err(AuthError::PasswordTooLong);
    }
    if stored_hash.len() > MAX_PASSWORD_HASH_BYTES {
        return Err(AuthError::PasswordHashTooLong);
    }
    let plaintext = plaintext.as_bytes().to_vec();
    let stored_hash = stored_hash.to_owned();
    run_password_job(PASSWORD_SEMAPHORE.clone(), move || {
        if let Some(rest) = stored_hash.strip_prefix(JELLYFIN_SHA1_PREFIX) {
            return verify_jellyfinsha1(&plaintext, rest);
        }
        let parsed = PasswordHash::new(&stored_hash).map_err(|_| AuthError::InvalidHash)?;
        Argon2::default()
            .verify_password(&plaintext, &parsed)
            .map_err(|_| AuthError::InvalidHash)
    })
    .await
}

/// Prefix of a legacy Jellyfin SHA-1 hash imported from a Jellyfin
/// database.
const JELLYFIN_SHA1_PREFIX: &str = "JELLYFIN_SHA1:";

/// Whether `stored_hash` uses a legacy scheme that should be replaced
/// by a fresh [`hash_password`] result after the next successful
/// [`verify_password`].
pub fn password_needs_rehash(stored_hash: &str) -> bool {
    stored_hash.starts_with(JELLYFIN_SHA1_PREFIX)
}

fn verify_jellyfinsha1(plaintext: &[u8], hex_digest: &str) -> Result<(), AuthError> {
    use subtle::ConstantTimeEq as _;

    let expected = hex::decode(hex_digest).map_err(|_| AuthError::InvalidHash)?;
    let mut hasher = Sha1::new();
    hasher.update(plaintext);
    let actual = hasher.finalize();
    if expected.len() == actual.len() && bool::from(actual.as_slice().ct_eq(&expected)) {
        Ok(())
    } else {
        Err(AuthError::InvalidHash)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Claims {
    pub sub: String,
    pub is_admin: bool,
    pub exp: i64,
    /// Session row this token was minted against. Required: every
    /// access token must be revocable through its session.
    pub sid: String,
}

#[derive(Clone)]
pub struct JwtConfig {
    secret: Vec<u8>,
    /// Lifetime of a short-lived access JWT. Every request re-checks the
    /// token's session in storage, so revocation is immediate; this only
    /// bounds how long a token leaked off the wire stays usable.
    pub access_token_ttl_secs: i64,
    /// Idle lifetime of the session's refresh token, stamped into
    /// `sessions.expires_at` at login and on every rotation.
    pub refresh_token_ttl_secs: i64,
    /// Absolute lifetime of a session counted from `created_at`; rotation
    /// never extends `expires_at` past it.
    pub max_session_lifetime_secs: i64,
}

const DEFAULT_ACCESS_TTL_SECS: i64 = 3600;
const DEFAULT_REFRESH_TTL_SECS: i64 = 30 * 24 * 3600;
// Sliding refresh expiry alone lets an active session, or a stolen refresh
// token that keeps rotating ahead of reuse detection, live forever; a hard
// cap forces a fresh password (or provider) check periodically.
const DEFAULT_MAX_SESSION_LIFETIME_SECS: i64 = 90 * 24 * 3600;
const MIN_SIGNING_KEY_BYTES: usize = 32;

impl JwtConfig {
    /// Load the signing key from `<data_dir>/jwt.key`, generating it on
    /// first start. A new key is written to a 0600 temp file, fsynced,
    /// and renamed into place, so a crash never leaves a short or empty
    /// key behind. An existing key with group/other permission bits is
    /// tightened to 0600 (with a warning) when the process may chmod it.
    ///
    /// # Errors
    ///
    /// [`AuthError::SigningKeyIo`] when the directory or key can't be
    /// created or read; [`AuthError::InsecureSigningKeyPermissions`]
    /// when a too-broad key can't be tightened (e.g. owned by another
    /// user); [`AuthError::WeakSigningKey`] when the key is shorter than
    /// 32 bytes.
    pub fn for_server(data_dir: &std::path::Path) -> Result<Self, AuthError> {
        let key_path = data_dir.join("jwt.key");
        let io_err = |path: &std::path::Path| {
            let path = path.to_owned();
            move |source| AuthError::SigningKeyIo { path, source }
        };
        std::fs::create_dir_all(data_dir).map_err(io_err(data_dir))?;

        let secret = match std::fs::metadata(&key_path) {
            Ok(_) => {
                restrict_key_permissions(&key_path)?;
                std::fs::read(&key_path).map_err(io_err(&key_path))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                write_new_key(data_dir, &key_path)?
            }
            Err(source) => {
                return Err(AuthError::SigningKeyIo {
                    path: key_path,
                    source,
                });
            }
        };
        if secret.len() < MIN_SIGNING_KEY_BYTES {
            return Err(AuthError::WeakSigningKey {
                minimum: MIN_SIGNING_KEY_BYTES,
                actual: secret.len(),
            });
        }
        Ok(Self {
            secret,
            access_token_ttl_secs: DEFAULT_ACCESS_TTL_SECS,
            refresh_token_ttl_secs: DEFAULT_REFRESH_TTL_SECS,
            max_session_lifetime_secs: DEFAULT_MAX_SESSION_LIFETIME_SECS,
        })
    }

    pub fn dev(secret: &[u8]) -> Result<Self, AuthError> {
        if secret.len() < MIN_SIGNING_KEY_BYTES {
            return Err(AuthError::WeakSigningKey {
                minimum: MIN_SIGNING_KEY_BYTES,
                actual: secret.len(),
            });
        }
        Ok(Self {
            secret: secret.to_vec(),
            access_token_ttl_secs: DEFAULT_ACCESS_TTL_SECS,
            refresh_token_ttl_secs: DEFAULT_REFRESH_TTL_SECS,
            max_session_lifetime_secs: DEFAULT_MAX_SESSION_LIFETIME_SECS,
        })
    }

    /// When a session created at `created_at` and refreshed at `now` stops
    /// refreshing: the idle TTL from `now`, capped at the absolute lifetime.
    pub fn session_expiry(
        &self,
        created_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> OffsetDateTime {
        let idle = now + time::Duration::seconds(self.refresh_token_ttl_secs);
        let cap = created_at + time::Duration::seconds(self.max_session_lifetime_secs);
        idle.min(cap)
    }
}

fn write_new_key(
    data_dir: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<Vec<u8>, AuthError> {
    let temp_path = data_dir.join(format!(".jwt.key.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let bytes = rand_bytes();
    let written = options
        .open(&temp_path)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp_path, key_path));
    if let Err(source) = written {
        let _ = std::fs::remove_file(&temp_path);
        return Err(AuthError::SigningKeyIo {
            path: key_path.to_owned(),
            source,
        });
    }
    // Persist the rename itself; without this a crash can roll the
    // directory entry back even though the file data was synced.
    #[cfg(unix)]
    std::fs::File::open(data_dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|source| AuthError::SigningKeyIo {
            path: data_dir.to_owned(),
            source,
        })?;
    Ok(bytes.to_vec())
}

#[cfg(unix)]
fn restrict_key_permissions(key_path: &std::path::Path) -> Result<(), AuthError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let mode = std::fs::metadata(key_path)
        .map_err(|source| AuthError::SigningKeyIo {
            path: key_path.to_owned(),
            source,
        })?
        .mode()
        & 0o777;
    if mode & 0o077 == 0 {
        return Ok(());
    }
    // chmod succeeds only for the file's owner (or root), which is
    // exactly the "ours to fix" case; anyone else's key stays an error.
    match std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600)) {
        Ok(()) => {
            tracing::warn!(
                path = %key_path.display(),
                mode = format_args!("{mode:o}"),
                "JWT signing key permissions were too broad; tightened to 0600"
            );
            Ok(())
        }
        Err(_) => Err(AuthError::InsecureSigningKeyPermissions {
            path: key_path.to_owned(),
            mode,
        }),
    }
}

#[cfg(not(unix))]
fn restrict_key_permissions(_key_path: &std::path::Path) -> Result<(), AuthError> {
    Ok(())
}

fn rand_bytes() -> [u8; 32] {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf
}

/// Mint a fresh refresh token: 256 bits of OS randomness, hex-encoded.
/// The plaintext goes to the client exactly once; only its SHA-256 is
/// ever stored.
pub fn generate_refresh_token() -> String {
    hex::encode(rand_bytes())
}

/// SHA-256 hex of a refresh token — the lookup key stored on the
/// session row. A DB leak then yields hashes of high-entropy tokens,
/// which are useless without the preimages.
pub fn hash_refresh_token(token: &str) -> String {
    sha256_hex(token)
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// Prefix marking a credential as an API key wherever tokens are
/// accepted — it's how `principal_from_headers` picks the lookup path.
pub const API_KEY_PREFIX: &str = "nfk_";

/// Mint a fresh API key: `nfk_` + 256 bits of OS randomness,
/// hex-encoded. The plaintext goes to the client exactly once; only
/// its SHA-256 is ever stored.
pub fn generate_api_key() -> String {
    format!("{API_KEY_PREFIX}{}", hex::encode(rand_bytes()))
}

/// SHA-256 hex of an API key — the lookup key stored on the key row.
pub fn hash_api_key(key: &str) -> String {
    sha256_hex(key)
}

/// Mint an access JWT for `user_id` bound to `session`. Requests carrying
/// it authenticate only while that session row exists and is unexpired.
pub fn issue_jwt(
    cfg: &JwtConfig,
    user_id: UserId,
    is_admin: bool,
    session: SessionId,
) -> Result<String, AuthError> {
    let exp = OffsetDateTime::now_utc().unix_timestamp() + cfg.access_token_ttl_secs;
    let claims = Claims {
        sub: user_id.to_string(),
        is_admin,
        exp,
        sid: session.to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&cfg.secret),
    )
    .map_err(|e| AuthError::JwtEncode(e.to_string()))
}

pub fn verify_jwt(cfg: &JwtConfig, token: &str) -> Result<Claims, AuthError> {
    let validation = Validation::new(Algorithm::HS256);
    let data = decode::<Claims>(token, &DecodingKey::from_secret(&cfg.secret), &validation)
        .map_err(|e| AuthError::JwtDecode(e.to_string()))?;
    Ok(data.claims)
}

/// Claims for a media token: a narrow, user-bound grant to fetch one
/// byte target, minted next to stream/segment/image URLs so the byte
/// handlers don't need a header credential. Deliberately shares no
/// required fields with [`Claims`] beyond `sub`/`exp`, so the two token
/// families can't be confused.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MediaClaims {
    pub kind: String,
    pub sub: String,
    pub tgt: String,
    pub exp: usize,
    /// Session the token was minted for; absent for session-less
    /// callers (API keys, server-internal jobs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
}

/// A verified media token. `session`, when set, must still be live for
/// the token to be honoured — see [`verify_media_token`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaToken {
    pub target: String,
    pub user: UserId,
    pub session: Option<SessionId>,
}

const MEDIA_TOKEN_KIND: &str = "media";

/// Mint a media token granting `user` access to `target` for `ttl`,
/// bound to `session` when the caller has one so revoking the session
/// also revokes the token.
///
/// The token only replaces the header credential: byte routes still run
/// `user`'s access checks on every request.
pub fn issue_media_token(
    cfg: &JwtConfig,
    user: UserId,
    session: Option<SessionId>,
    target: &str,
    ttl: std::time::Duration,
) -> Result<String, AuthError> {
    let exp = OffsetDateTime::now_utc().unix_timestamp()
        + i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
    encode_media_token(cfg, user, session, target, exp)
}

/// Mint a media token that is byte-identical for every call with the
/// same `user`, `session` and `target` within one UTC hour of `now`, so
/// URLs that embed it stay stable for HTTP caching.
///
/// Expires 25 hours after the start of `now`'s UTC hour: always at
/// least 24 hours of validity.
pub fn issue_bucketed_media_token(
    cfg: &JwtConfig,
    user: UserId,
    session: Option<SessionId>,
    target: &str,
    now: OffsetDateTime,
) -> Result<String, AuthError> {
    const HOUR: i64 = 60 * 60;
    let ts = now.unix_timestamp();
    let bucket = ts - ts.rem_euclid(HOUR);
    encode_media_token(cfg, user, session, target, bucket + 25 * HOUR)
}

fn encode_media_token(
    cfg: &JwtConfig,
    user: UserId,
    session: Option<SessionId>,
    target: &str,
    exp: i64,
) -> Result<String, AuthError> {
    let claims = MediaClaims {
        kind: MEDIA_TOKEN_KIND.to_string(),
        sub: user.to_string(),
        tgt: target.to_string(),
        exp: usize::try_from(exp).unwrap_or(usize::MAX),
        sid: session.map(|s| s.to_string()),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&cfg.secret),
    )
    .map_err(|e| AuthError::JwtEncode(e.to_string()))
}

/// Verify a media token's signature, expiry, and shape (`sub` a user
/// ID, `sid` absent or a session ID). Access JWTs are rejected: they
/// lack the required `kind`/`tgt` fields, and the `kind` check keeps the
/// families disjoint even if the claim sets ever converge. The caller
/// must still check that `user` exists, that `session` (if any) is live
/// and theirs, and that `user` may access `target`.
pub fn verify_media_token(cfg: &JwtConfig, token: &str) -> Option<MediaToken> {
    let validation = Validation::new(Algorithm::HS256);
    let data =
        decode::<MediaClaims>(token, &DecodingKey::from_secret(&cfg.secret), &validation).ok()?;
    if data.claims.kind != MEDIA_TOKEN_KIND {
        return None;
    }
    let user = UserId::from_uuid(data.claims.sub.parse().ok()?);
    let session = match data.claims.sid {
        Some(sid) => Some(SessionId::from_uuid(sid.parse().ok()?)),
        None => None,
    };
    Some(MediaToken {
        target: data.claims.tgt,
        user,
        session,
    })
}

/// Resolve the request credential to a principal. An `X-Api-Key`
/// header always takes the API-key path; `Bearer`/`X-Emby-Token`
/// credentials starting with [`API_KEY_PREFIX`] do too, and anything
/// else is verified as an access JWT. Unknown, revoked, or
/// unresolvable credentials yield an anonymous principal — never an
/// error — so handlers keep owning the 401 decision.
pub async fn principal_from_headers(
    headers: &HeaderMap,
    cfg: &JwtConfig,
    storage: &dyn Storage,
) -> Principal {
    if let Some(key) = headers.get("X-Api-Key").and_then(|h| h.to_str().ok()) {
        return principal_from_api_key(storage, key).await;
    }
    let Some(token) = bearer_or_emby_token(headers) else {
        return Principal::default();
    };
    if token.starts_with(API_KEY_PREFIX) {
        return principal_from_api_key(storage, token).await;
    }
    principal_from_jwt_checked(storage, cfg, token).await
}

fn bearer_or_emby_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .or_else(|| headers.get("X-Emby-Token").and_then(|h| h.to_str().ok()))
}

fn principal_from_jwt(cfg: &JwtConfig, token: &str) -> Principal {
    let Ok(claims) = verify_jwt(cfg, token) else {
        return Principal::default();
    };
    let (Ok(user_id), Ok(session_id)) = (
        claims.sub.parse::<uuid::Uuid>(),
        claims.sid.parse::<uuid::Uuid>(),
    ) else {
        return Principal::default();
    };
    Principal {
        user_id: UserId::from_uuid(user_id),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(session_id)),
    }
}

async fn principal_from_jwt_checked(
    storage: &dyn Storage,
    cfg: &JwtConfig,
    token: &str,
) -> Principal {
    let principal = principal_from_jwt(cfg, token);
    if !principal.is_authenticated() {
        return principal;
    }
    let Some(session_id) = principal.session_id else {
        return Principal::default();
    };
    let Ok(Some(user)) = storage.get_user(principal.user_id).await else {
        return Principal::default();
    };
    let Ok(Some(session)) = storage.get_session(session_id).await else {
        return Principal::default();
    };
    // A locked session is waiting for its user's PIN: until then what it
    // issued is worth nothing, however long the access token has left.
    if session.user_id != user.id
        || session.expires_at <= OffsetDateTime::now_utc()
        || session.locked_at.is_some()
    {
        return Principal::default();
    }
    Principal {
        user_id: user.id,
        is_admin: user.is_admin,
        session_id: principal.session_id,
    }
}

async fn principal_from_api_key(storage: &dyn Storage, key: &str) -> Principal {
    let hash = hash_api_key(key);
    let Ok(Some(api_key)) = storage.api_key_by_hash(&hash).await else {
        return Principal::default();
    };
    if api_key.revoked_at.is_some() {
        return Principal::default();
    }
    let Ok(Some(user)) = storage.get_user(api_key.user_id).await else {
        return Principal::default();
    };
    Principal {
        user_id: user.id,
        is_admin: user.is_admin,
        session_id: None,
    }
}

/// Inject a `Principal` into the request extensions. Credential
/// headers win; when none is present an `?api_key=` query parameter is
/// honoured for byte-serving clients that can't set headers. `?token=`
/// is deliberately not read here — scoped media tokens are validated
/// by the byte handlers themselves.
///
/// Storage is pulled from request extensions; if the server hasn't
/// published it, every credential resolves anonymous rather than
/// failing the request.
pub async fn auth_middleware(
    headers: HeaderMap,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let cfg = req
        .extensions()
        .get::<JwtConfig>()
        .cloned()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let storage = req.extensions().get::<Arc<dyn Storage>>().cloned();

    let has_credential_header = headers.contains_key(header::AUTHORIZATION)
        || headers.contains_key("X-Emby-Token")
        || headers.contains_key("X-Api-Key");
    let query_key = if has_credential_header {
        None
    } else {
        req.uri()
            .query()
            .and_then(|q| query_param(q, "api_key"))
            .map(str::to_owned)
    };

    let principal = match (storage.as_deref(), query_key) {
        (Some(store), Some(key)) => principal_from_api_key(store, &key).await,
        (Some(store), None) => principal_from_headers(&headers, &cfg, store).await,
        // Without storage neither revocation nor the current role can be
        // checked, so the token's own claims are never trusted.
        (None, _) => Principal::default(),
    };
    req.extensions_mut().insert(principal);
    Ok(next.run(req).await)
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then_some(v)
    })
}

/// Extractor that 401s when the request has no authenticated
/// principal. Use as a handler argument to require auth.
pub struct AuthRequired(pub Principal);

impl<S> axum::extract::FromRequestParts<S> for AuthRequired
where
    S: Send + Sync + 'static,
{
    type Rejection = axum::http::StatusCode;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let p = parts
            .extensions
            .get::<Principal>()
            .copied()
            .unwrap_or_default();
        if p.is_authenticated() {
            Ok(AuthRequired(p))
        } else {
            Err(axum::http::StatusCode::UNAUTHORIZED)
        }
    }
}

/// Extractor that 403s unless the request is from an admin. Use
/// after AuthRequired in a handler signature.
pub struct AdminRequired(pub Principal);

impl<S> axum::extract::FromRequestParts<S> for AdminRequired
where
    S: Send + Sync + 'static,
{
    type Rejection = axum::http::StatusCode;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let p = parts
            .extensions
            .get::<Principal>()
            .copied()
            .unwrap_or_default();
        if p.is_authenticated() && p.is_admin {
            Ok(AdminRequired(p))
        } else if !p.is_authenticated() {
            Err(axum::http::StatusCode::UNAUTHORIZED)
        } else {
            Err(axum::http::StatusCode::FORBIDDEN)
        }
    }
}

#[cfg(test)]
mod tests {
    use noflex_core::UserRole;
    use noflex_storage::{User, domain::ApiKey};
    use noflex_storage_turso::TursoStore;

    use super::*;

    const TEST_SECRET: &[u8] = b"unit-test-secret-32-bytes-please";

    async fn temp_storage() -> TursoStore {
        let store = TursoStore::open_temp().await.expect("temp store");
        store.migrate().await.expect("migrate");
        store
    }

    fn test_user(name: &str, is_admin: bool) -> User {
        User {
            id: UserId::new(),
            username: name.to_string(),
            display_name: None,
            password_hash: String::new(),
            is_admin,
            role: if is_admin {
                UserRole::Admin
            } else {
                UserRole::User
            },
            created_at: OffsetDateTime::now_utc(),
            avatar_updated_at: None,
            has_pin: false,
        }
    }

    async fn make_session(store: &TursoStore, user_id: UserId) -> SessionId {
        let now = OffsetDateTime::now_utc();
        let id = SessionId::new();
        store
            .create_session(&noflex_storage::domain::Session {
                id,
                user_id,
                client: None,
                device: None,
                device_id: None,
                app_version: None,
                remote_addr: None,
                created_at: now,
                last_seen_at: now,
                expires_at: now + time::Duration::hours(1),
                refresh_token_hash: None,
                locked_at: None,
            })
            .await
            .unwrap();
        id
    }

    async fn make_api_key(store: &TursoStore, user_id: UserId) -> (String, ApiKey) {
        let key = generate_api_key();
        let row = store
            .create_api_key(user_id, "test", &hash_api_key(&key))
            .await
            .expect("create api key");
        (key, row)
    }

    #[tokio::test]
    async fn argon2_round_trip() {
        let hash = hash_password("hunter2").await.expect("hash");
        verify_password("hunter2", &hash).await.expect("verify");
        assert!(verify_password("wrong", &hash).await.is_err());
    }

    #[tokio::test]
    async fn jellyfinsha1_legacy() {
        let hash = format!("JELLYFIN_SHA1:{}", {
            let mut h = Sha1::new();
            h.update(b"jellyfin");
            hex::encode(h.finalize())
        });
        verify_password("jellyfin", &hash)
            .await
            .expect("legacy verify");
        verify_password("jellyfin", &hash.to_uppercase())
            .await
            .expect("uppercase legacy verify");
        assert!(verify_password("wrong", &hash).await.is_err());
        assert!(
            verify_password("jellyfin", "JELLYFIN_SHA1:abc")
                .await
                .is_err()
        );
        assert!(password_needs_rehash(&hash));
        assert!(!password_needs_rehash(
            &hash_password("x").await.expect("hash")
        ));
    }

    #[tokio::test]
    async fn legacy_hash_is_upgraded_on_successful_auth() {
        let store = Arc::new(temp_storage().await);
        let mut user = test_user("legacy", false);
        user.password_hash = format!("JELLYFIN_SHA1:{}", {
            let mut h = Sha1::new();
            h.update(b"pw");
            hex::encode(h.finalize())
        });
        store.upsert_user(&user).await.expect("upsert");
        let provider = LocalAuthProvider::new(store.clone());
        let verdict = provider
            .authenticate(&AuthCredentials {
                username: "legacy".into(),
                password: "pw".into(),
                device: None,
                client: None,
            })
            .await
            .expect("authenticate");
        assert!(matches!(verdict, AuthVerdict::Authenticated { .. }));
        let stored = store.get_user(user.id).await.expect("get").expect("user");
        assert!(stored.password_hash.starts_with("$argon2id$"));
        verify_password("pw", &stored.password_hash)
            .await
            .expect("verify upgraded hash");
    }

    #[tokio::test]
    async fn oversized_passwords_are_rejected_before_dispatch() {
        let oversized = "x".repeat(MAX_PASSWORD_BYTES + 1);
        assert!(matches!(
            hash_password(&oversized).await,
            Err(AuthError::PasswordTooLong)
        ));
        assert!(matches!(
            verify_password(&oversized, "not-a-hash").await,
            Err(AuthError::PasswordTooLong)
        ));
    }

    #[tokio::test]
    async fn password_jobs_wait_for_a_bounded_slot_before_spawning() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let active = Arc::new(AtomicUsize::new(0));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first_active = active.clone();
        let first = tokio::spawn(run_password_job(semaphore.clone(), move || {
            first_active.fetch_add(1, Ordering::SeqCst);
            release_rx.recv().map_err(|_| AuthError::PasswordWorker)?;
            first_active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }));
        while active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        let second_active = active.clone();
        let second = tokio::spawn(run_password_job(semaphore, move || {
            second_active.fetch_add(1, Ordering::SeqCst);
            second_active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }));
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert_eq!(active.load(Ordering::SeqCst), 1);
        release_tx.send(()).expect("release first job");
        first.await.expect("first join").expect("first job");
        second.await.expect("second join").expect("second job");
    }

    fn test_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("noflex-auth-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).expect("temp dir");
        path
    }

    #[test]
    fn server_jwt_key_is_persisted_and_reused() {
        let temp = test_dir();
        let first = JwtConfig::for_server(&temp).expect("create config");
        let second = JwtConfig::for_server(&temp).expect("load config");
        assert_eq!(first.secret, second.secret);
        assert_eq!(first.secret.len(), MIN_SIGNING_KEY_BYTES);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(temp.join("jwt.key"))
                .expect("key metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let leftovers = std::fs::read_dir(&temp)
            .expect("read temp dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "jwt.key")
            .count();
        assert_eq!(leftovers, 0, "no temp files left behind");
        std::fs::remove_dir_all(temp).expect("remove temp dir");
    }

    #[test]
    fn server_jwt_rejects_weak_or_unreadable_keys() {
        let weak = test_dir();
        std::fs::write(weak.join("jwt.key"), b"short").expect("weak key");
        assert!(matches!(
            JwtConfig::for_server(&weak),
            Err(AuthError::WeakSigningKey { .. })
        ));
        std::fs::remove_dir_all(weak).expect("remove weak temp dir");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let broad = test_dir();
            std::fs::write(broad.join("jwt.key"), [1_u8; MIN_SIGNING_KEY_BYTES]).expect("key");
            std::fs::set_permissions(
                broad.join("jwt.key"),
                std::fs::Permissions::from_mode(0o644),
            )
            .expect("permissions");
            // Our own over-broad key is tightened in place, not rejected.
            JwtConfig::for_server(&broad).expect("tighten own key");
            let mode = std::fs::metadata(broad.join("jwt.key"))
                .expect("key metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
            std::fs::remove_dir_all(broad).expect("remove broad-key temp dir");
        }

        let unreadable = test_dir();
        std::fs::create_dir(unreadable.join("jwt.key")).expect("key directory");
        assert!(matches!(
            JwtConfig::for_server(&unreadable),
            Err(AuthError::SigningKeyIo { .. })
        ));
        std::fs::remove_dir_all(unreadable).expect("remove unreadable temp dir");

        let root = test_dir();
        let not_a_directory = root.join("data");
        std::fs::write(&not_a_directory, b"file").expect("data file");
        assert!(matches!(
            JwtConfig::for_server(&not_a_directory),
            Err(AuthError::SigningKeyIo { .. })
        ));
        std::fs::remove_dir_all(root).expect("remove create-failure temp dir");
    }

    #[test]
    fn development_jwt_rejects_weak_keys() {
        assert!(matches!(
            JwtConfig::dev(b"short"),
            Err(AuthError::WeakSigningKey { .. })
        ));
    }

    #[test]
    fn jwt_round_trip() {
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user_id = UserId::new();
        let token = issue_jwt(&cfg, user_id, true, SessionId::new()).expect("issue");
        let claims = verify_jwt(&cfg, &token).expect("verify");
        assert!(claims.is_admin);
        assert_eq!(claims.sub, user_id.to_string());
    }

    #[test]
    fn jwt_carries_session_claim() {
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user_id = UserId::new();
        let session_id = SessionId::new();
        let token = issue_jwt(&cfg, user_id, false, session_id).expect("issue");
        let claims = verify_jwt(&cfg, &token).expect("verify");
        assert_eq!(claims.sid, session_id.to_string());
    }

    #[tokio::test]
    async fn access_token_without_session_claim_is_anonymous() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = test_user("sessionless", true);
        store.upsert_user(&user).await.unwrap();
        #[derive(serde::Serialize)]
        struct Sessionless {
            sub: String,
            is_admin: bool,
            exp: i64,
        }
        let claims = Sessionless {
            sub: user.id.to_string(),
            is_admin: true,
            exp: OffsetDateTime::now_utc().unix_timestamp() + 60,
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET),
        )
        .unwrap();
        assert!(verify_jwt(&cfg, &token).is_err());
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert!(
            !principal_from_headers(&headers, &cfg, &store)
                .await
                .is_authenticated()
        );
    }

    #[test]
    fn refresh_token_hash_is_deterministic_and_unique() {
        let a = generate_refresh_token();
        let b = generate_refresh_token();
        assert_ne!(a, b, "fresh tokens must differ");
        assert_eq!(a.len(), 64, "32 bytes hex-encoded");
        assert_eq!(hash_refresh_token(&a), hash_refresh_token(&a));
        assert_ne!(hash_refresh_token(&a), hash_refresh_token(&b));
        assert_eq!(hash_refresh_token(&a).len(), 64, "sha256 hex");
    }

    #[tokio::test]
    async fn principal_from_bearer_header() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = test_user("jwt-user", false);
        store.upsert_user(&user).await.unwrap();
        let user_id = user.id;
        let session_id = make_session(&store, user_id).await;
        let token = issue_jwt(&cfg, user_id, false, session_id).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let p = principal_from_headers(&headers, &cfg, &store).await;
        assert_eq!(p.user_id, user_id);
        assert!(!p.is_admin);
        assert!(p.is_authenticated());
        assert_eq!(p.session_id, Some(session_id));
    }

    #[tokio::test]
    async fn token_for_unknown_session_is_anonymous() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = test_user("ghost-session", false);
        store.upsert_user(&user).await.unwrap();
        let token = issue_jwt(&cfg, user.id, false, SessionId::new()).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert!(
            !principal_from_headers(&headers, &cfg, &store)
                .await
                .is_authenticated()
        );
    }

    #[test]
    fn session_expiry_is_capped_at_the_absolute_lifetime() {
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let created = OffsetDateTime::now_utc();
        assert_eq!(
            cfg.session_expiry(created, created),
            created + time::Duration::seconds(cfg.refresh_token_ttl_secs)
        );
        let late = created + time::Duration::seconds(cfg.max_session_lifetime_secs - 60);
        assert_eq!(
            cfg.session_expiry(created, late),
            created + time::Duration::seconds(cfg.max_session_lifetime_secs)
        );
    }

    #[test]
    fn anonymous_principal_is_not_authenticated() {
        let p = Principal::anonymous();
        assert!(!p.is_authenticated());
        assert_eq!(p.session_id, None);
    }

    #[tokio::test]
    async fn principal_from_session_token() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = test_user("session-user", true);
        store.upsert_user(&user).await.unwrap();
        let user_id = user.id;
        let session_id = SessionId::new();
        store
            .create_session(&noflex_storage::domain::Session {
                id: session_id,
                user_id,
                client: None,
                device: None,
                device_id: None,
                app_version: None,
                remote_addr: None,
                created_at: OffsetDateTime::now_utc(),
                last_seen_at: OffsetDateTime::now_utc(),
                expires_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
                refresh_token_hash: None,
                locked_at: None,
            })
            .await
            .unwrap();
        let token = issue_jwt(&cfg, user_id, true, session_id).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let p = principal_from_headers(&headers, &cfg, &store).await;
        assert_eq!(p.session_id, Some(session_id));
        // Locked, the token it issued is worth nothing until it's opened again.
        store
            .lock_session(session_id, OffsetDateTime::now_utc())
            .await
            .unwrap();
        assert!(
            !principal_from_headers(&headers, &cfg, &store)
                .await
                .is_authenticated()
        );
        store.unlock_session(session_id).await.unwrap();
        assert!(
            principal_from_headers(&headers, &cfg, &store)
                .await
                .is_authenticated()
        );
        store.revoke_session(session_id).await.unwrap();
        assert!(
            !principal_from_headers(&headers, &cfg, &store)
                .await
                .is_authenticated()
        );
    }

    #[tokio::test]
    async fn jwt_uses_the_current_user_role() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let mut user = test_user("role-change", true);
        store.upsert_user(&user).await.unwrap();
        let session_id = make_session(&store, user.id).await;
        let token = issue_jwt(&cfg, user.id, true, session_id).unwrap();
        user.is_admin = false;
        user.role = UserRole::User;
        store.upsert_user(&user).await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert!(
            !principal_from_headers(&headers, &cfg, &store)
                .await
                .is_admin
        );
    }

    #[test]
    fn api_key_format() {
        let a = generate_api_key();
        let b = generate_api_key();
        assert_ne!(a, b);
        assert!(a.starts_with(API_KEY_PREFIX));
        assert_eq!(a.len(), API_KEY_PREFIX.len() + 64, "32 bytes hex-encoded");
        assert_eq!(hash_api_key(&a), hash_api_key(&a));
        assert_eq!(hash_api_key(&a).len(), 64, "sha256 hex");
    }

    #[tokio::test]
    async fn api_key_round_trip() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = test_user("alice", true);
        store.upsert_user(&user).await.expect("upsert user");
        let (key, _row) = make_api_key(&store, user.id).await;

        let mut headers = HeaderMap::new();
        headers.insert("X-Api-Key", key.parse().unwrap());
        let p = principal_from_headers(&headers, &cfg, &store).await;
        assert_eq!(p.user_id, user.id);
        assert!(p.is_admin);
        assert!(p.is_authenticated());
        assert_eq!(p.session_id, None);

        // A Bearer credential with the nfk_ prefix takes the same path.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {key}").parse().unwrap(),
        );
        let p = principal_from_headers(&headers, &cfg, &store).await;
        assert_eq!(p.user_id, user.id);
    }

    #[tokio::test]
    async fn revoked_api_key_is_anonymous() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = test_user("bob", false);
        store.upsert_user(&user).await.expect("upsert user");
        let (key, row) = make_api_key(&store, user.id).await;
        assert!(store.revoke_api_key(row.id).await.expect("revoke"));

        let mut headers = HeaderMap::new();
        headers.insert("X-Api-Key", key.parse().unwrap());
        let p = principal_from_headers(&headers, &cfg, &store).await;
        assert!(!p.is_authenticated());
    }

    #[tokio::test]
    async fn unknown_api_key_is_anonymous() {
        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let mut headers = HeaderMap::new();
        headers.insert("X-Api-Key", generate_api_key().parse().unwrap());
        let p = principal_from_headers(&headers, &cfg, &store).await;
        assert!(!p.is_authenticated());
    }

    #[test]
    fn media_token_round_trip() {
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = UserId::new();
        let session = SessionId::new();
        let token = issue_media_token(
            &cfg,
            user,
            Some(session),
            "work/abc/segment/0",
            std::time::Duration::from_secs(60),
        )
        .expect("issue");
        assert_eq!(
            verify_media_token(&cfg, &token),
            Some(MediaToken {
                target: "work/abc/segment/0".to_string(),
                user,
                session: Some(session),
            })
        );
    }

    #[test]
    fn bucketed_media_token_is_stable_within_the_hour() {
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = UserId::new();
        let start = OffsetDateTime::now_utc()
            .replace_minute(0)
            .and_then(|t| t.replace_second(0))
            .and_then(|t| t.replace_nanosecond(0))
            .expect("hour start");
        let a = issue_bucketed_media_token(&cfg, user, None, "img:work:x", start).expect("a");
        let b = issue_bucketed_media_token(
            &cfg,
            user,
            None,
            "img:work:x",
            start + time::Duration::minutes(59),
        )
        .expect("b");
        let c = issue_bucketed_media_token(
            &cfg,
            user,
            None,
            "img:work:x",
            start + time::Duration::HOUR,
        )
        .expect("c");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(
            verify_media_token(&cfg, &a),
            Some(MediaToken {
                target: "img:work:x".to_string(),
                user,
                session: None,
            })
        );
        let bound =
            issue_bucketed_media_token(&cfg, user, Some(SessionId::new()), "img:work:x", start)
                .expect("session-bound");
        assert_ne!(a, bound);
        let other = issue_bucketed_media_token(&cfg, UserId::new(), None, "img:work:x", start)
            .expect("other user");
        assert_ne!(a, other);
    }

    #[test]
    fn expired_media_token_is_rejected() {
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let exp = usize::try_from(OffsetDateTime::now_utc().unix_timestamp() - 3600).unwrap();
        let claims = MediaClaims {
            kind: "media".to_string(),
            sub: UserId::new().to_string(),
            tgt: "x".to_string(),
            exp,
            sid: None,
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&cfg.secret),
        )
        .expect("encode");
        assert!(verify_media_token(&cfg, &token).is_none());
    }

    #[test]
    fn media_and_access_tokens_are_not_interchangeable() {
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let access = issue_jwt(&cfg, UserId::new(), true, SessionId::new()).expect("issue access");
        assert!(verify_media_token(&cfg, &access).is_none());

        let media = issue_media_token(
            &cfg,
            UserId::new(),
            None,
            "t",
            std::time::Duration::from_secs(60),
        )
        .expect("issue media");
        assert!(verify_jwt(&cfg, &media).is_err());
    }

    #[tokio::test]
    async fn middleware_resolves_api_key_query_param() {
        use axum::{Extension, Router, middleware, routing::get};
        use tower::ServiceExt;

        let store = temp_storage().await;
        let cfg = JwtConfig::dev(TEST_SECRET).expect("JWT config");
        let user = test_user("carol", false);
        store.upsert_user(&user).await.expect("upsert user");
        let (key, _row) = make_api_key(&store, user.id).await;
        let storage: Arc<dyn Storage> = Arc::new(store);

        let app = Router::new()
            .route(
                "/bytes",
                get(|Extension(p): Extension<Principal>| async move {
                    p.user_id.into_uuid().to_string()
                }),
            )
            .layer(middleware::from_fn(auth_middleware))
            .layer(Extension(storage))
            .layer(Extension(cfg));

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/bytes?api_key={key}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], user.id.into_uuid().to_string().as_bytes());

        // No credential at all: anonymous principal reaches the handler.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/bytes")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], uuid::Uuid::nil().to_string().as_bytes());
    }
}
