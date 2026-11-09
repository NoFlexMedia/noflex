// SPDX-License-Identifier: AGPL-3.0-only
//
// Login, logout, and refresh mutations. `login` verifies credentials
// against the password hash and mints a `Session` row plus a token
// pair: a short-lived access JWT carrying the session id, and a
// long-lived refresh token whose SHA-256 lives on that row. Revoking the
// row invalidates its access and refresh tokens. The same message
// is returned for an unknown user and a bad password — no enumeration.
// Refresh tokens rotate on every use and the session remembers every
// hash it rotated away from, so a replayed old token revokes the whole
// session (RFC 9700 §4.14.2 reuse detection).
//
// A device several people share keeps a session each. `lockSession` leaves
// one waiting for its user's PIN when they step away, and `unlockSession`
// is the only way back in. The PIN never signs anyone in on its own: it
// opens a session whose refresh token the device already holds, so a short
// one can't be guessed from anywhere else.

use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions as _, Object, SimpleObject};
use noflex_auth::Principal;
use noflex_core::{SessionId, UserId};
use noflex_storage::{
    Storage,
    canonical::canonical_timestamp,
    domain::{ActivityKind, RefreshTokenLookup, RetiredRefreshToken, User},
};
use time::OffsetDateTime;

/// The token-pair-plus-user payload `login`, `refreshSession`, and
/// `pollDeviceCode` all return — one shape regardless of which flow
/// minted the session.
#[derive(SimpleObject)]
pub(crate) struct LoginResult {
    pub(crate) token: String,
    pub(crate) refresh_token: String,
    pub(crate) user: User,
}

/// The shared "credentials don't match" message. Identical for unknown
/// users and wrong passwords to avoid leaking which usernames exist.
const INVALID_CREDENTIALS: &str = "invalid username or password";

/// The shared "refresh rejected" message; `extensions.code` tells a
/// client whether its session survived.
const INVALID_REFRESH: &str = "invalid refresh token";
/// Unknown, expired, revoked, and user-less sessions, indistinguishable.
const REFRESH_INVALID: &str = "REFRESH_TOKEN_INVALID";
const REFRESH_SUPERSEDED: &str = "REFRESH_TOKEN_SUPERSEDED";
const REFRESH_REUSED: &str = "REFRESH_TOKEN_REUSED";
/// The session is waiting for its user's PIN; `unlockSession` opens it.
const SESSION_LOCKED: &str = "SESSION_LOCKED";
const PIN_INCORRECT: &str = "PIN_INCORRECT";
const PIN_ATTEMPTS_EXHAUSTED: &str = "PIN_ATTEMPTS_EXHAUSTED";

/// Wrong PINs a session takes before it's revoked. Few, because a PIN is
/// short: past them, whoever is trying signs in properly.
const PIN_ATTEMPTS: u32 = 5;
/// Tiles a sign-in screen can ask about at once.
const MAX_REMEMBERED_PROFILES: usize = 16;

/// How long the immediately-previous refresh token is refused without
/// revoking its session. One client firing two refreshes with the same
/// token (a retry after a lost response, tabs sharing storage) loses the
/// CAS on the second, and logging the user out for that would punish a
/// race, not a theft. The window is kept to a few seconds because inside
/// it an attacker who refreshed first is not detected when the
/// legitimate client follows; any later replay, and any replay of an
/// older token, revokes the session.
const REFRESH_REUSE_GRACE: time::Duration = time::Duration::seconds(5);

/// Tracing target for security events operators may route to an audit sink.
const SECURITY_TARGET: &str = "noflex::security";

fn refresh_rejected(code: &'static str) -> async_graphql::Error {
    async_graphql::Error::new(INVALID_REFRESH).extend_with(|_, ext| ext.set("code", code))
}

/// RFC 9700 §4.14.2 reuse response: a rotated-away token came back, so
/// either the client or an attacker holds a stolen copy. Revoking the
/// session logs out both; the revocation is audit-logged to tracing and
/// the activity log. Activity-log failure is logged, never surfaced, so
/// the revocation stands either way.
async fn revoke_reused_session(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    retired: &RetiredRefreshToken,
    now: OffsetDateTime,
) -> async_graphql::Result<()> {
    let session = &retired.session;
    storage
        .revoke_session(session.id)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    let presented_from = ctx
        .data_opt::<noflex_auth::ClientAddr>()
        .map(|addr| addr.0.to_string());
    tracing::warn!(
        target: SECURITY_TARGET,
        user = %session.user_id,
        session = %session.id,
        retired_at = %canonical_timestamp(retired.retired_at),
        presented_from = presented_from.as_deref().unwrap_or("unknown"),
        "refresh token reuse detected; session revoked"
    );
    let detail = serde_json::json!({
        "retiredAt": canonical_timestamp(retired.retired_at),
        "client": session.client,
        "device": session.device,
        "presentedFrom": presented_from,
    })
    .to_string();
    if let Err(error) = storage
        .append_activity(
            ActivityKind::RefreshTokenReuse,
            Some(&session.user_id.to_string()),
            Some(&session.id.to_string()),
            Some(&detail),
            now,
        )
        .await
    {
        tracing::error!(
            target: SECURITY_TARGET,
            session = %session.id,
            %error,
            "recording refresh token reuse in the activity log failed"
        );
    }
    Ok(())
}

/// Argon2id hash verified against when the username doesn't exist, so
/// login timing can't distinguish "no such user" from "wrong password".
/// Computed once; the password behind it is irrelevant and never used.
static DUMMY_PASSWORD_HASH: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// The schema's shared failed-credential limiter.
pub(crate) fn login_throttle(
    ctx: &Context<'_>,
) -> async_graphql::Result<Arc<noflex_auth::LoginThrottle>> {
    ctx.data::<Arc<noflex_auth::LoginThrottle>>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("auth not configured"))
}

/// The client-facing refusal for a throttled credential check.
pub(crate) fn too_many_attempts(retry_after: std::time::Duration) -> async_graphql::Error {
    async_graphql::Error::new("too many failed attempts; try again later")
        .extend_with(|_, ext| ext.set("retryAfterSecs", retry_after.as_secs().max(1)))
}

/// Mint a session row + token pair for `user`, recording the request's
/// [`ClientAddr`](noflex_auth::ClientAddr) when the transport published
/// one. Every sign-in flow (login, setup wizard, device code, external
/// provider) goes through here so all issue identical, revocable sessions.
pub(crate) async fn mint_session(
    ctx: &Context<'_>,
    user: &User,
    client: Option<String>,
    device: Option<String>,
) -> async_graphql::Result<(String, String)> {
    let storage = ctx
        .data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))?;
    let jwt = ctx
        .data::<Arc<noflex_auth::JwtConfig>>()
        .map_err(|_| async_graphql::Error::new("auth not configured"))?;
    let now = OffsetDateTime::now_utc();
    let refresh_token = noflex_auth::generate_refresh_token();
    let session = noflex_storage::domain::Session {
        id: SessionId::new(),
        user_id: user.id,
        client,
        device,
        device_id: None,
        app_version: None,
        remote_addr: ctx
            .data_opt::<noflex_auth::ClientAddr>()
            .map(|addr| addr.0.to_string()),
        created_at: now,
        last_seen_at: now,
        expires_at: jwt.session_expiry(now, now),
        refresh_token_hash: Some(noflex_auth::hash_refresh_token(&refresh_token)),
        locked_at: None,
    };
    storage
        .create_session(&session)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    let access_token = noflex_auth::issue_jwt(jwt, user.id, user.is_admin, session.id)
        .map_err(|e| async_graphql::Error::new(format!("jwt: {e}")))?;
    Ok((access_token, refresh_token))
}

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

/// A refresh token that named a live session: its current one, unexpired,
/// with a user behind it. What `refreshSession` and `unlockSession` share.
struct PresentedRefreshToken {
    session: noflex_storage::domain::Session,
    user: User,
    hash: String,
    now: OffsetDateTime,
}

impl PresentedRefreshToken {
    /// # Errors
    ///
    /// `refreshSession`'s rejections, bar `SESSION_LOCKED`: whether a lock
    /// matters is the caller's to say. A replayed token revokes its session
    /// on the way out, and so does finding one expired.
    async fn resolve(ctx: &Context<'_>, refresh_token: &str) -> async_graphql::Result<Self> {
        let storage = storage(ctx)?;
        let jwt = ctx
            .data::<Arc<noflex_auth::JwtConfig>>()
            .map_err(|_| async_graphql::Error::new("auth not configured"))?;

        let hash = noflex_auth::hash_refresh_token(refresh_token);
        let now = OffsetDateTime::now_utc();
        let session = match storage
            .session_by_refresh_token(&hash)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        {
            RefreshTokenLookup::Current(session) => session,
            RefreshTokenLookup::Unknown => return Err(refresh_rejected(REFRESH_INVALID)),
            RefreshTokenLookup::Retired(retired) => {
                if retired.latest && now - retired.retired_at < REFRESH_REUSE_GRACE {
                    return Err(refresh_rejected(REFRESH_SUPERSEDED));
                }
                revoke_reused_session(ctx, storage, &retired, now).await?;
                return Err(refresh_rejected(REFRESH_REUSED));
            }
        };

        let lifetime_over =
            session.created_at + time::Duration::seconds(jwt.max_session_lifetime_secs) <= now;
        if session.expires_at <= now || lifetime_over {
            storage
                .revoke_session(session.id)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            return Err(refresh_rejected(REFRESH_INVALID));
        }
        let user = match storage
            .get_user(session.user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        {
            Some(user) => user,
            // The FK cascade should make this unreachable; if a backend
            // ever lets it through, the orphaned session dies here.
            None => {
                storage
                    .revoke_session(session.id)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
                return Err(refresh_rejected(REFRESH_INVALID));
            }
        };
        Ok(Self {
            session,
            user,
            hash,
            now,
        })
    }

    /// Swap the presented token for a fresh pair on the same session.
    async fn rotate(self, ctx: &Context<'_>) -> async_graphql::Result<LoginResult> {
        let storage = storage(ctx)?;
        let jwt = ctx
            .data::<Arc<noflex_auth::JwtConfig>>()
            .map_err(|_| async_graphql::Error::new("auth not configured"))?;
        let Self {
            session,
            user,
            hash,
            now,
        } = self;
        let new_refresh = noflex_auth::generate_refresh_token();
        let rotated = storage
            .rotate_refresh_token(
                session.id,
                &hash,
                &noflex_auth::hash_refresh_token(&new_refresh),
                jwt.session_expiry(session.created_at, now),
                now,
            )
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if !rotated {
            // The token was current a moment ago, so this is a lost race
            // (or a concurrent revocation), never evidence of theft.
            return Err(refresh_rejected(REFRESH_SUPERSEDED));
        }

        let token = noflex_auth::issue_jwt(jwt, user.id, user.is_admin, session.id)
            .map_err(|e| async_graphql::Error::new(format!("jwt: {e}")))?;
        Ok(LoginResult {
            token,
            refresh_token: new_refresh,
            user,
        })
    }
}

/// Count a wrong PIN against `session` and say what's left, revoking the
/// session once it has taken [`PIN_ATTEMPTS`].
async fn wrong_pin(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    session: &noflex_storage::domain::Session,
) -> async_graphql::Result<async_graphql::Error> {
    let failures = storage
        .record_pin_failure(session.id)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    if failures < PIN_ATTEMPTS {
        let left = PIN_ATTEMPTS - failures;
        return Ok(
            async_graphql::Error::new("that PIN isn't right").extend_with(|_, ext| {
                ext.set("code", PIN_INCORRECT);
                ext.set("attemptsLeft", left);
            }),
        );
    }
    storage
        .revoke_session(session.id)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    tracing::warn!(
        target: SECURITY_TARGET,
        user = %session.user_id,
        session = %session.id,
        presented_from = ctx
            .data_opt::<noflex_auth::ClientAddr>()
            .map(|addr| addr.0.to_string())
            .as_deref()
            .unwrap_or("unknown"),
        "too many wrong PINs; session revoked"
    );
    Ok(
        async_graphql::Error::new("too many wrong PINs; sign in again")
            .extend_with(|_, ext| ext.set("code", PIN_ATTEMPTS_EXHAUSTED)),
    )
}

/// Someone a device still holds a session for, as its sign-in screen shows
/// them: enough to draw a tile, and nothing a tile doesn't need.
#[derive(SimpleObject)]
pub(crate) struct RememberedProfile {
    user_id: UserId,
    /// What to call them, as `User.name`.
    name: String,
    /// Whether choosing them takes a PIN, through `unlockSession`.
    has_pin: bool,
    /// Their picture, as `User.avatarUrl`. It keeps loading while the
    /// session is locked, since it's what the lock screen shows.
    avatar_url: Option<String>,
}

/// Auth half of the query root; merged into `Query` in `lib.rs`.
#[derive(Default)]
pub struct AuthQuery;

#[Object]
impl AuthQuery {
    /// Who the sessions behind `refreshTokens` belong to, in the order
    /// given: what a shared device draws its "Who's watching?" from. Needs
    /// no authentication, as each token is its own. Null for a token that
    /// isn't a live session's current one, whose tile then needs signing in
    /// again; a locked session still answers. Nothing is rotated or revoked.
    async fn remembered_profiles(
        &self,
        ctx: &Context<'_>,
        refresh_tokens: Vec<String>,
    ) -> async_graphql::Result<Vec<Option<RememberedProfile>>> {
        if refresh_tokens.len() > MAX_REMEMBERED_PROFILES {
            return Err(async_graphql::Error::new(format!(
                "at most {MAX_REMEMBERED_PROFILES} profiles at a time"
            )));
        }
        let storage = storage(ctx)?;
        let signer = ctx.data_opt::<Arc<dyn noflex_storage::domain::MediaUrlSigner>>();
        let now = OffsetDateTime::now_utc();
        let mut profiles = Vec::with_capacity(refresh_tokens.len());
        for token in &refresh_tokens {
            let lookup = storage
                .session_by_refresh_token(&noflex_auth::hash_refresh_token(token))
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let RefreshTokenLookup::Current(session) = lookup else {
                profiles.push(None);
                continue;
            };
            let user = storage
                .get_user(session.user_id)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let Some(user) = user.filter(|_| session.expires_at > now) else {
                profiles.push(None);
                continue;
            };
            profiles.push(Some(RememberedProfile {
                user_id: user.id,
                name: user.shown_name().to_string(),
                has_pin: user.has_pin,
                // Signed for no session, or a locked one would refuse its
                // own lock screen the picture.
                avatar_url: user
                    .avatar_url_for(signer.map(Arc::as_ref), user.id, None)
                    .map_err(async_graphql::Error::new)?,
            }));
        }
        Ok(profiles)
    }
}

/// Auth half of the mutation root; merged into `Mutation` in `lib.rs`.
#[derive(Default)]
pub struct AuthMutation;

#[Object]
impl AuthMutation {
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn login(
        &self,
        ctx: &Context<'_>,
        username: String,
        password: String,
        client: Option<String>,
        device: Option<String>,
    ) -> async_graphql::Result<LoginResult> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;

        if username.len() > noflex_auth::MAX_USERNAME_BYTES
            || password.len() > noflex_auth::MAX_PASSWORD_BYTES
        {
            return Err(async_graphql::Error::new(INVALID_CREDENTIALS));
        }
        let throttle = login_throttle(ctx)?;
        let client_addr = ctx.data_opt::<noflex_auth::ClientAddr>().copied();
        throttle
            .check(&username, client_addr)
            .map_err(too_many_attempts)?;
        let user = storage
            .get_user_by_username(&username)
            .await
            .map_err(|_| async_graphql::Error::new("authentication failed"))?;
        let Some(user) = user else {
            // Verify against a dummy hash anyway so unknown users cost
            // the same Argon2id run as a wrong password.
            let dummy_hash = DUMMY_PASSWORD_HASH
                .get_or_try_init(|| noflex_auth::hash_password("timing-equalizer"))
                .await
                .map_err(|_| async_graphql::Error::new("authentication failed"))?;
            let _ = noflex_auth::verify_password(&password, dummy_hash).await;
            throttle.record_failure(&username, client_addr);
            return Err(async_graphql::Error::new(INVALID_CREDENTIALS));
        };
        if noflex_auth::verify_password(&password, &user.password_hash)
            .await
            .is_err()
        {
            throttle.record_failure(&username, client_addr);
            return Err(async_graphql::Error::new(INVALID_CREDENTIALS));
        }
        throttle.record_success(&username);
        noflex_auth::upgrade_legacy_hash(storage.as_ref(), &user, &password).await;

        let (token, refresh_token) = mint_session(ctx, &user, client, device).await?;
        Ok(LoginResult {
            token,
            refresh_token,
            user,
        })
    }

    /// Exchange a live refresh token for a fresh token pair. The
    /// session row is rotated in place — same id, new hash, new
    /// expiry — so the presented token dies the moment it succeeds,
    /// and a revoked or expired session can never refresh again. Expiry
    /// slides by the refresh TTL but never past the session's absolute
    /// lifetime, after which the user must sign in again.
    ///
    /// Every rejection carries the message "invalid refresh token" and
    /// an `extensions.code`: `REFRESH_TOKEN_INVALID` (unknown, expired,
    /// or revoked), `REFRESH_TOKEN_SUPERSEDED` (a concurrent refresh with
    /// the same token won; the session lives on under the winner's
    /// token), `REFRESH_TOKEN_REUSED` (a rotated-away token was
    /// replayed; the whole session has been revoked), or `SESSION_LOCKED`
    /// (the session waits for its user's PIN; `unlockSession` opens it).
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn refresh_session(
        &self,
        ctx: &Context<'_>,
        refresh_token: String,
    ) -> async_graphql::Result<LoginResult> {
        let presented = PresentedRefreshToken::resolve(ctx, &refresh_token).await?;
        if presented.session.locked_at.is_some() {
            return Err(refresh_rejected(SESSION_LOCKED));
        }
        presented.rotate(ctx).await
    }

    /// Leave the session behind `refreshToken` waiting for its user's PIN,
    /// as a shared device does when someone steps away from their profile.
    /// Until `unlockSession`, its access token authenticates nothing and it
    /// can't refresh. Returns whether the session is locked now: `false`
    /// for a token that isn't a live session's current one, and for a user
    /// without a PIN, whose session nothing would need to open.
    async fn lock_session(
        &self,
        ctx: &Context<'_>,
        refresh_token: String,
    ) -> async_graphql::Result<bool> {
        let storage = storage(ctx)?;
        let hash = noflex_auth::hash_refresh_token(&refresh_token);
        let RefreshTokenLookup::Current(session) = storage
            .session_by_refresh_token(&hash)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        else {
            return Ok(false);
        };
        let now = OffsetDateTime::now_utc();
        if session.expires_at <= now {
            return Ok(false);
        }
        if session.locked_at.is_some() {
            return Ok(true);
        }
        let has_pin = storage
            .get_user(session.user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .is_some_and(|user| user.has_pin);
        if !has_pin {
            return Ok(false);
        }
        storage
            .lock_session(session.id, now)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(true)
    }

    /// Open the session behind `refreshToken` with its user's PIN and
    /// rotate it, as `refreshSession` does. A device checks the PIN this way
    /// whether or not the session got as far as being locked, and a user
    /// without a PIN is let straight through.
    ///
    /// Rejections are `refreshSession`'s, and two of its own:
    /// `PIN_INCORRECT`, with `extensions.attemptsLeft`, and
    /// `PIN_ATTEMPTS_EXHAUSTED` once the fifth wrong PIN in a row has
    /// revoked the session, leaving a full sign-in as the way back.
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn unlock_session(
        &self,
        ctx: &Context<'_>,
        refresh_token: String,
        pin: String,
    ) -> async_graphql::Result<LoginResult> {
        let storage = storage(ctx)?;
        let presented = PresentedRefreshToken::resolve(ctx, &refresh_token).await?;
        let session = &presented.session;
        let pin_hash = storage
            .user_pin_hash(session.user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if let Some(pin_hash) = pin_hash {
            let correct = crate::profile::valid_pin(&pin)
                && noflex_auth::verify_password(&pin, &pin_hash).await.is_ok();
            if !correct {
                return Err(wrong_pin(ctx, storage, session).await?);
            }
        }
        storage
            .unlock_session(session.id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        presented.rotate(ctx).await
    }

    /// Revoke the session backing the caller's token, killing its access
    /// and refresh tokens. Returns `true` when a session was revoked,
    /// `false` for an API-key caller (no session to revoke). Requires
    /// authentication.
    async fn logout(&self, ctx: &Context<'_>) -> async_graphql::Result<bool> {
        let principal = ctx
            .data::<Principal>()
            .map_err(|_| async_graphql::Error::new("authentication required"))?;
        if !principal.is_authenticated() {
            return Err(async_graphql::Error::new("authentication required"));
        }
        let Some(session_id) = principal.session_id else {
            return Ok(false);
        };
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        storage
            .revoke_session(session_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(true)
    }
}
