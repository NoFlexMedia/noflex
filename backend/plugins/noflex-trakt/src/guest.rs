// SPDX-License-Identifier: AGPL-3.0-only
//
// Trakt guest: implements `user-link` (device authorization flow),
// `scrobbler` (Trakt /scrobble/*), the `poll-link` background task that
// drives device-flow polling, and the `sync` task (`crate::sync_task`). All HTTP goes through the
// capability-gated `host-net` import; per-user tokens live host-side
// and move only through `host-secrets.read-for-user` /
// `write-for-user`. Compiled only for wasm32 — the host-testable
// mapping lives in `crate::api`.
//
// Device-link flow: `link-start` POSTs `/oauth/device/code`, enqueues a
// `poll-link` run carrying the device code, and returns the activation
// URL with the code the user types there. Each `poll-link` run waits
// Trakt's interval on the monotonic clock, polls `/oauth/device/token`
// once, and answers `more(cursor)` while the user hasn't authorized; the
// host keeps one such chain per user. The cursor counts down the code's
// lifetime, so an abandoned link stops when the code expires; `slow_down`
// lengthens the wait. `unlink` marks the user's device code revoked, so a
// poll already in flight can't hand tokens back after they unlinked.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use exports::noflex::plugin::{background, scrobbler, user_link};
use noflex::plugin::{
    host_log, host_net, host_secrets, host_tasks,
    types::{ExternalId, ExternalIdKind, PluginError, ScrobbleAction, ScrobbleEvent, WorkKind},
};

use crate::api::{self, Action, IdKind, MediaKind, PollAnswer, PollCursor, Scrobble};

// The canonical `scrobbler-plugin` world plus `host-user-state` for history
// sync and WASI 0.3's monotonic clock, whose async `wait-for` paces
// device-token polls and POSTs without blocking the instance's other calls.
wit_bindgen::generate!({
    inline: r#"
        package noflex:trakt;

        world trakt {
            include noflex:plugin/scrobbler-plugin@0.2.0;
            import noflex:plugin/host-user-state@0.2.0;
            import wasi:clocks/monotonic-clock@0.3.0;
        }
    "#,
    path: "../../crates/noflex-plugin-wit/wit",
    world: "noflex:trakt/trakt",
    generate_all,
});

/// Embedded plugin manifest. wit-bindgen + the wasm32-wasip2 linker
/// preserves `#[link_section]` items as custom sections in the final
/// component binary, so the host can read this without a post-build step.
#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

const TIMEOUT_MS: u32 = 10_000;
const NANOS_PER_SEC: u64 = 1_000_000_000;

thread_local! {
    // In-flight device links, user id → device code. `link-start`
    // inserts, `link-complete` validates its state against the map,
    // and the poll task removes on success or terminal failure. The
    // cursor is what survives instance recycling; the map is the
    // single-instance coherence check on top of it.
    static PENDING: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    // Device codes whose user unlinked while their poll may still be in
    // flight. Only this instance's polls can race an unlink: after a
    // restart the host no longer has the chain.
    static REVOKED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

struct Trakt;

pub(crate) fn log(level: host_log::Level, message: &str) {
    host_log::log(level, "trakt", message, &[]);
}

fn revoked(device_code: &str) -> bool {
    REVOKED.with(|revoked| revoked.borrow().contains(device_code))
}

/// End `cursor`'s chain.
fn finish(cursor: &PollCursor) -> background::TaskOutcome {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.get(&cursor.user) == Some(&cursor.device_code) {
            pending.remove(&cursor.user);
        }
    });
    REVOKED.with(|revoked| revoked.borrow_mut().remove(&cursor.device_code));
    background::TaskOutcome::Done
}

async fn read_client_credentials() -> Result<(String, String), PluginError> {
    let client_id = host_secrets::read("client_id".into())
        .await
        .map_err(|e| PluginError::Unavailable(format!("trakt: client_id unreadable: {e:?}")))?;
    let client_secret = host_secrets::read("client_secret".into())
        .await
        .map_err(|e| PluginError::Unavailable(format!("trakt: client_secret unreadable: {e:?}")))?;
    Ok((client_id, client_secret))
}

fn post(url: &str, headers: Vec<(String, String)>, body: String) -> host_net::HttpRequest {
    host_net::HttpRequest {
        method: "POST".into(),
        url: url.into(),
        headers,
        body: Some(body.into_bytes()),
        timeout_ms: Some(TIMEOUT_MS),
    }
}

// -------- user-link hook --------

impl user_link::Guest for Trakt {
    async fn link_start(user: String) -> Result<user_link::LinkChallenge, PluginError> {
        let (client_id, _) = read_client_credentials().await?;
        let request = post(
            api::DEVICE_CODE_URL,
            api::api_headers(&client_id, None),
            api::device_code_body(&client_id),
        );
        let response = host_net::request(request)
            .await
            .map_err(|e| PluginError::Unavailable(format!("trakt: device-code request: {e:?}")))?;
        if response.status != 200 {
            return Err(PluginError::Unavailable(format!(
                "trakt: device-code answered {}",
                response.status
            )));
        }
        let device = api::parse_device_code(&response.body).map_err(PluginError::Internal)?;

        let replaced = PENDING.with(|pending| {
            pending
                .borrow_mut()
                .insert(user.clone(), device.device_code.clone())
        });
        // A restarted link supersedes the old one, whose poll the host's new
        // chain for this user replaces.
        if let Some(old) = replaced {
            REVOKED.with(|revoked| revoked.borrow_mut().insert(old));
        }
        // The host binds the run to this call's user, so the chain can
        // write their tokens and nobody else's.
        host_tasks::enqueue(
            "poll-link".into(),
            Some(PollCursor::new(user, &device).encode()),
        )
        .await
        .map_err(|e| {
            PluginError::Unavailable(format!("trakt: could not schedule the link poll: {e:?}"))
        })?;

        Ok(user_link::LinkChallenge {
            auth_url: device.verification_url,
            // Opaque handle the host binds to the user; the device code
            // is exactly the handle Trakt knows this link by.
            state: device.device_code,
            user_code: Some(device.user_code),
        })
    }

    async fn link_complete(
        user: String,
        state: String,
        callback_query: String,
    ) -> Result<Vec<user_link::UserSecret>, PluginError> {
        // Device flow has no redirect back to the host: the poll-link
        // task delivers the tokens out-of-band via write-for-user. The
        // only meaningful validation here is that `state` names a link
        // this instance actually started; beyond that there is nothing
        // to exchange — "linked, nothing to store".
        let _ = callback_query;
        let known = PENDING.with(|pending| pending.borrow().get(&user) == Some(&state));
        if !known {
            return Err(PluginError::BadInput(format!(
                "trakt: no pending device link for user {user} with that state"
            )));
        }
        Ok(Vec::new())
    }

    async fn unlink(user: String) -> Result<(), PluginError> {
        if let Some(device_code) = PENDING.with(|pending| pending.borrow_mut().remove(&user)) {
            REVOKED.with(|revoked| revoked.borrow_mut().insert(device_code));
        }
        let Ok(token) = host_secrets::read_for_user(user.clone(), "access_token".into()).await
        else {
            return Ok(());
        };
        let (client_id, client_secret) = read_client_credentials().await?;
        let request = post(
            api::REVOKE_URL,
            api::api_headers(&client_id, None),
            api::revoke_body(&token, &client_id, &client_secret),
        );
        match host_net::request(request).await {
            Ok(response) if response.status == 200 => Ok(()),
            Ok(response) => Err(PluginError::Unavailable(format!(
                "trakt: revoke answered {}",
                response.status
            ))),
            Err(e) => Err(PluginError::Unavailable(format!(
                "trakt: revoke failed: {e:?}"
            ))),
        }
    }
}

// -------- background hook (poll-link) --------

/// One paced `/oauth/device/token` poll: wait the cursor's interval, then
/// ask once. `more(cursor)` keeps the chain going with what's left of the
/// code's lifetime.
async fn poll_once(mut cursor: PollCursor) -> background::TaskOutcome {
    if revoked(&cursor.device_code) {
        return finish(&cursor);
    }
    let Some(wait) = cursor.next_wait() else {
        log(
            host_log::Level::Info,
            &format!("trakt: device link for user {} expired", cursor.user),
        );
        return finish(&cursor);
    };
    wasi::clocks::monotonic_clock::wait_for(wait * NANOS_PER_SEC).await;
    if revoked(&cursor.device_code) {
        return finish(&cursor);
    }

    let Ok((client_id, client_secret)) = read_client_credentials().await else {
        // Misconfig is terminal, not transient: no point retrying.
        log(
            host_log::Level::Error,
            "trakt: client credentials unreadable, abandoning device-link poll",
        );
        return finish(&cursor);
    };

    let request = post(
        api::DEVICE_TOKEN_URL,
        api::api_headers(&client_id, None),
        api::device_token_body(&cursor.device_code, &client_id, &client_secret),
    );
    let response = match host_net::request(request).await {
        Ok(response) => response,
        Err(e) => {
            // Transient egress failure — keep polling until the code expires.
            log(
                host_log::Level::Warn,
                &format!("trakt: device-token poll failed: {e:?}"),
            );
            return background::TaskOutcome::More(cursor.encode());
        }
    };

    match PollAnswer::from_status(response.status) {
        PollAnswer::Authorized => {
            // The user may have unlinked while the request was out.
            if revoked(&cursor.device_code) {
                return finish(&cursor);
            }
            match api::parse_token_pair(&response.body) {
                Ok(pair) => match store_token_pair(&cursor.user, &pair).await {
                    Ok(()) => {
                        log(
                            host_log::Level::Info,
                            &format!("trakt: user {} linked", cursor.user),
                        );
                        // The first sync starts now rather than at the next
                        // tick; this run is bound to the user, so it is too.
                        if let Err(e) =
                            host_tasks::enqueue("sync".into(), Some(cursor.user.clone())).await
                        {
                            log(
                                host_log::Level::Warn,
                                &format!("trakt: could not start the first sync: {e:?}"),
                            );
                        }
                    }
                    Err(e) => log(
                        host_log::Level::Error,
                        &format!(
                            "trakt: could not persist tokens for user {}: {e:?}",
                            cursor.user
                        ),
                    ),
                },
                Err(e) => log(
                    host_log::Level::Error,
                    &format!("trakt: unparsable token response, abandoning link: {e}"),
                ),
            }
            finish(&cursor)
        }
        PollAnswer::Pending => background::TaskOutcome::More(cursor.encode()),
        PollAnswer::SlowDown => {
            cursor.slow_down();
            background::TaskOutcome::More(cursor.encode())
        }
        PollAnswer::Ended => {
            log(
                host_log::Level::Info,
                &format!(
                    "trakt: device link for user {} ended (status {})",
                    cursor.user, response.status
                ),
            );
            finish(&cursor)
        }
        PollAnswer::Unexpected => {
            log(
                host_log::Level::Warn,
                &format!("trakt: unexpected device-token status {}", response.status),
            );
            background::TaskOutcome::More(cursor.encode())
        }
    }
}

async fn store_token_pair(user: &str, pair: &api::TokenPair) -> Result<(), PluginError> {
    host_secrets::write_for_user(
        user.into(),
        "access_token".into(),
        pair.access_token.clone(),
    )
    .await?;
    host_secrets::write_for_user(
        user.into(),
        "refresh_token".into(),
        pair.refresh_token.clone(),
    )
    .await?;
    Ok(())
}

impl background::Guest for Trakt {
    async fn run_task(
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        if task == "sync" {
            return crate::sync_task::run(cursor, payload).await;
        }
        if task != "poll-link" {
            return Err(background::PluginError::BadInput(format!(
                "trakt: unknown background task `{task}`"
            )));
        }
        // A dynamic enqueue starts a fresh poll chain; a re-invocation
        // continues the cursor's chain; a scheduled tick has neither.
        let raw = payload.or(cursor);
        let chain = raw.and_then(|raw| match PollCursor::decode(&raw) {
            Ok(chain) => Some(chain),
            Err(e) => {
                log(
                    host_log::Level::Warn,
                    &format!("trakt: dropping unparsable poll-link state: {e}"),
                );
                None
            }
        });
        match chain {
            Some(chain) => Ok(poll_once(chain).await),
            None => Ok(background::TaskOutcome::Done),
        }
    }
}

// -------- scrobbler hook --------

impl From<ScrobbleAction> for Action {
    fn from(action: ScrobbleAction) -> Self {
        match action {
            ScrobbleAction::Started => Action::Started,
            ScrobbleAction::Paused => Action::Paused,
            ScrobbleAction::Stopped => Action::Stopped,
        }
    }
}

fn trakt_ids(ids: &[ExternalId]) -> Vec<(IdKind, &str)> {
    ids.iter()
        .map(|id| {
            let kind = match id.kind {
                ExternalIdKind::Tmdb => IdKind::Tmdb,
                ExternalIdKind::Tvdb => IdKind::Tvdb,
                ExternalIdKind::Imdb => IdKind::Imdb,
                _ => IdKind::Other,
            };
            (kind, id.value.as_str())
        })
        .collect()
}

fn scrobble_request(
    event: &ScrobbleEvent,
    client_id: &str,
    token: &str,
) -> Option<host_net::HttpRequest> {
    let kind = match event.kind {
        WorkKind::Movie => MediaKind::Movie,
        WorkKind::Episode => MediaKind::Episode,
        _ => MediaKind::Other,
    };
    let ids = trakt_ids(&event.external_ids);
    let show_ids = trakt_ids(&event.series_external_ids);
    let action = Action::from(event.action);
    let body = api::scrobble_body(&Scrobble {
        kind,
        title: &event.title,
        year: event.year,
        ids: &ids,
        show_ids: &show_ids,
        season: event.season_number,
        number: event.episode_number,
        progress: api::progress_percent(action, event.position_ms, event.runtime_ms),
    })?;
    Some(post(
        &format!("{}/scrobble/{}", api::API_BASE, action.endpoint()),
        api::api_headers(client_id, Some(token)),
        body,
    ))
}

/// One scrobble POST. On 401 the access token is refreshed (the new
/// pair persisted host-side) and the request retried once; the second
/// failure is logged and dropped — scrobbling is lossy by design.
async fn send_scrobble(event: &ScrobbleEvent, client_id: &str, token: &str) {
    let Some(request) = scrobble_request(event, client_id, token) else {
        log(
            host_log::Level::Debug,
            &format!(
                "trakt: skipping `{}` — not scrobbleable or no usable external ids",
                event.title
            ),
        );
        return;
    };
    match host_net::request(request).await {
        Ok(resp) if resp.status == 401 => {
            let Some(fresh) = refresh_access_token(&event.user, client_id).await else {
                return;
            };
            let Some(retry) = scrobble_request(event, client_id, &fresh) else {
                return;
            };
            match host_net::request(retry).await {
                Ok(resp) if resp.status >= 400 => log(
                    host_log::Level::Warn,
                    &format!(
                        "trakt: scrobble of `{}` failed after refresh: {}",
                        event.title, resp.status
                    ),
                ),
                Ok(_) => {}
                Err(e) => log(
                    host_log::Level::Error,
                    &format!(
                        "trakt: scrobble of `{}` failed after refresh: {e:?}",
                        event.title
                    ),
                ),
            }
        }
        Ok(resp) if resp.status >= 400 => log(
            host_log::Level::Warn,
            &format!(
                "trakt: scrobble of `{}` answered {}",
                event.title, resp.status
            ),
        ),
        Ok(_) => {}
        Err(e) => log(
            host_log::Level::Error,
            &format!("trakt: scrobble of `{}` failed: {e:?}", event.title),
        ),
    }
}

/// Refresh the user's access token via the refresh_token grant,
/// persisting the rotated pair host-side. Returns the new access token;
/// any failure is logged and yields `None` (the caller drops the
/// scrobble).
pub(crate) async fn refresh_access_token(user: &str, client_id: &str) -> Option<String> {
    let client_secret = match host_secrets::read("client_secret".into()).await {
        Ok(secret) => secret,
        Err(e) => {
            log(
                host_log::Level::Warn,
                &format!("trakt: cannot refresh for {user}: {e:?}"),
            );
            return None;
        }
    };
    let refresh_token = match host_secrets::read_for_user(user.into(), "refresh_token".into()).await
    {
        Ok(token) => token,
        Err(e) => {
            log(
                host_log::Level::Warn,
                &format!("trakt: no refresh token for {user}: {e:?}"),
            );
            return None;
        }
    };
    let request = post(
        api::TOKEN_URL,
        api::api_headers(client_id, None),
        api::refresh_body(&refresh_token, client_id, &client_secret),
    );
    let pair = match host_net::request(request).await {
        Ok(resp) if resp.status == 200 => match api::parse_token_pair(&resp.body) {
            Ok(pair) => pair,
            Err(e) => {
                log(
                    host_log::Level::Error,
                    &format!("trakt: unparsable refresh response for {user}: {e}"),
                );
                return None;
            }
        },
        Ok(resp) => {
            log(
                host_log::Level::Warn,
                &format!("trakt: refresh for {user} answered {}", resp.status),
            );
            return None;
        }
        Err(e) => {
            log(
                host_log::Level::Error,
                &format!("trakt: refresh for {user} failed: {e:?}"),
            );
            return None;
        }
    };
    if let Err(e) = store_token_pair(user, &pair).await {
        log(
            host_log::Level::Error,
            &format!("trakt: could not persist refreshed tokens for {user}: {e:?}"),
        );
        return None;
    }
    Some(pair.access_token)
}

impl scrobbler::Guest for Trakt {
    async fn scrobble(event: ScrobbleEvent) -> Result<(), PluginError> {
        let token =
            match host_secrets::read_for_user(event.user.clone(), "access_token".into()).await {
                Ok(token) => token,
                Err(e) => {
                    // Unlinked user or revoked capability — the host skips
                    // those users already, but never turn it into a hook
                    // error.
                    log(
                        host_log::Level::Debug,
                        &format!(
                            "trakt: no access token for user {}, skipping: {e:?}",
                            event.user
                        ),
                    );
                    return Ok(());
                }
            };
        // trakt-api-key is required on every call; an unreadable
        // client_id is operator misconfig, logged once per event.
        let client_id = match host_secrets::read("client_id".into()).await {
            Ok(client_id) => client_id,
            Err(e) => {
                log(
                    host_log::Level::Warn,
                    &format!("trakt: client_id unreadable, skipping scrobble: {e:?}"),
                );
                return Ok(());
            }
        };
        send_scrobble(&event, &client_id, &token).await;
        Ok(())
    }
}

export!(Trakt);
