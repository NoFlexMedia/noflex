// SPDX-License-Identifier: AGPL-3.0-only
//
// Trakt guest: implements `user-link` (device authorization flow),
// `scrobbler` (Trakt /scrobble/*), and the `poll-link` background task
// that drives device-flow polling. All HTTP goes through the
// capability-gated `host-net` import; per-user tokens live host-side
// and move only through `host-secrets.read-for-user` /
// `write-for-user`. Compiled only for wasm32 — the host-testable
// mapping lives in `crate::api`.
//
// Device-link flow: `link-start` POSTs `/oauth/device/code`, remembers
// the device code in instance state (single instance, interior
// mutability), enqueues a `poll-link` run, and returns the activation
// URL. `poll-link` then polls `/oauth/device/token`: HTTP 400 means
// "user hasn't authorized yet" and is answered with `more(cursor)`;
// the scheduler re-invokes promptly with that cursor. The cursor
// carries an attempt counter — polling gives up after
// `MAX_POLL_ATTEMPTS` so an abandoned link cannot poll forever
// (Trakt's device codes expire after ~10 minutes; the host paces
// prompt re-invocations, so the cap is expressed in attempts, not
// wall time). Terminal Trakt answers (404 invalid code, 410 expired,
// 418 denied) end the poll immediately with `done`. Only one poll
// chain is live at a time: a new `link-start` enqueue takes priority
// over the previous chain's cursor.
//
// The user-code wart: `link-challenge` has no field for the code the
// user must type at https://trakt.tv/activate, so `link-start` logs it
// at info level and returns only the URL. The WIT is not bent to fit
// Trakt; the log line is the user-facing channel until the WIT grows
// an instructions field.

use std::{cell::RefCell, collections::HashMap};

use exports::noflex::plugin::{background, scrobbler, user_link};
use noflex::plugin::{
    host_log, host_net, host_secrets, host_tasks,
    types::{ExternalIdKind, PluginError, ScrobbleAction, ScrobbleEvent, WorkKind},
};

use crate::api::{self, Action, IdKind, MediaKind, PollCursor};

wit_bindgen::generate!({
    path: "../../crates/noflex-plugin-wit/wit",
    world: "scrobbler-plugin",
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

/// Upper bound on `/oauth/device/token` polls for one link. Trakt
/// device codes live ~10 minutes; this caps the prompt re-invocation
/// chain regardless of the host's pacing.
const MAX_POLL_ATTEMPTS: u32 = 120;

thread_local! {
    // In-flight device links, user id → device code. `link-start`
    // inserts, `link-complete` validates its state against the map,
    // and the poll task removes on success or terminal failure. The
    // cursor is what survives instance recycling; the map is the
    // single-instance coherence check on top of it.
    static PENDING: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

struct Trakt;

fn log(level: host_log::Level, message: &str) {
    host_log::log(level, "trakt", message, &[]);
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

        PENDING.with(|pending| {
            pending
                .borrow_mut()
                .insert(user.clone(), device.device_code.clone());
        });
        let enqueue = host_tasks::enqueue(
            "poll-link".into(),
            Some(PollCursor::new(user.clone(), device.device_code.clone()).encode()),
        )
        .await;
        if let Err(e) = enqueue {
            // The challenge still stands, but without the poll the link
            // can never complete — say so loudly.
            log(
                host_log::Level::Error,
                &format!("trakt: could not schedule poll-link, link will not complete: {e:?}"),
            );
        }

        // The WIT link-challenge has no user-code field (see module
        // docs): the code goes to the log, the URL to the caller.
        log(
            host_log::Level::Info,
            &format!(
                "trakt: user {user} linking — open {} and enter code {}",
                device.verification_url, device.user_code
            ),
        );
        Ok(user_link::LinkChallenge {
            auth_url: device.verification_url,
            // Opaque handle the host binds to the user; the device code
            // is exactly the handle Trakt knows this link by.
            state: device.device_code,
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
}

// -------- background hook (poll-link) --------

/// One `/oauth/device/token` poll. Returns the outcome the scheduler
/// should record; `PollCursor` in `more(...)` carries the bumped
/// attempt counter so an abandoned link dies at `MAX_POLL_ATTEMPTS`.
async fn poll_once(mut cursor: PollCursor) -> background::TaskOutcome {
    cursor.attempt += 1;
    if cursor.attempt > MAX_POLL_ATTEMPTS {
        log(
            host_log::Level::Warn,
            &format!(
                "trakt: giving up on device link for user {} after {} attempts",
                cursor.user, MAX_POLL_ATTEMPTS
            ),
        );
        PENDING.with(|pending| pending.borrow_mut().remove(&cursor.user));
        return background::TaskOutcome::Done;
    }

    let Ok((client_id, client_secret)) = read_client_credentials().await else {
        // Misconfig is terminal, not transient: no point retrying.
        log(
            host_log::Level::Error,
            "trakt: client credentials unreadable, abandoning device-link poll",
        );
        PENDING.with(|pending| pending.borrow_mut().remove(&cursor.user));
        return background::TaskOutcome::Done;
    };

    let request = post(
        api::DEVICE_TOKEN_URL,
        api::api_headers(&client_id, None),
        api::device_token_body(&cursor.device_code, &client_id, &client_secret),
    );
    let response = match host_net::request(request).await {
        Ok(response) => response,
        Err(e) => {
            // Transient egress failure — keep polling until the cap.
            log(
                host_log::Level::Warn,
                &format!(
                    "trakt: device-token poll failed (attempt {}): {e:?}",
                    cursor.attempt
                ),
            );
            return background::TaskOutcome::More(cursor.encode());
        }
    };

    match response.status {
        200 => match api::parse_token_pair(&response.body) {
            Ok(pair) => {
                if let Err(e) = store_token_pair(&cursor.user, &pair).await {
                    log(
                        host_log::Level::Error,
                        &format!(
                            "trakt: could not persist tokens for user {}: {e:?}",
                            cursor.user
                        ),
                    );
                } else {
                    log(
                        host_log::Level::Info,
                        &format!("trakt: user {} linked", cursor.user),
                    );
                }
                PENDING.with(|pending| pending.borrow_mut().remove(&cursor.user));
                background::TaskOutcome::Done
            }
            Err(e) => {
                log(
                    host_log::Level::Error,
                    &format!("trakt: unparsable token response, abandoning link: {e}"),
                );
                PENDING.with(|pending| pending.borrow_mut().remove(&cursor.user));
                background::TaskOutcome::Done
            }
        },
        // 400 = authorization_pending: the user hasn't entered the code
        // yet. 429 = slow_down: same retry, Trakt is pacing us.
        400 | 429 => background::TaskOutcome::More(cursor.encode()),
        // 404 invalid code, 410 expired, 418 denied — terminal.
        404 | 410 | 418 => {
            log(
                host_log::Level::Info,
                &format!(
                    "trakt: device link for user {} ended (status {})",
                    cursor.user, response.status
                ),
            );
            PENDING.with(|pending| pending.borrow_mut().remove(&cursor.user));
            background::TaskOutcome::Done
        }
        other => {
            log(
                host_log::Level::Warn,
                &format!(
                    "trakt: unexpected device-token status {other} (attempt {})",
                    cursor.attempt
                ),
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
        if task != "poll-link" {
            return Err(background::PluginError::BadInput(format!(
                "trakt: unknown background task `{task}`"
            )));
        }
        // A dynamic enqueue starts a fresh poll chain; a scheduled tick
        // continues the cursor's chain or has nothing to do.
        let chain = match payload {
            Some(raw) => match PollCursor::decode(&raw) {
                Ok(fresh) => Some(fresh),
                Err(e) => {
                    log(
                        host_log::Level::Warn,
                        &format!("trakt: dropping unparsable poll-link payload: {e}"),
                    );
                    None
                }
            },
            None => cursor.and_then(|raw| match PollCursor::decode(&raw) {
                Ok(chain) => Some(chain),
                Err(e) => {
                    log(
                        host_log::Level::Warn,
                        &format!("trakt: dropping unparsable poll-link cursor: {e}"),
                    );
                    None
                }
            }),
        };
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
    let ids: Vec<(IdKind, &str)> = event
        .external_ids
        .iter()
        .map(|id| {
            let kind = match id.kind {
                ExternalIdKind::Tmdb => IdKind::Tmdb,
                ExternalIdKind::Tvdb => IdKind::Tvdb,
                ExternalIdKind::Imdb => IdKind::Imdb,
                _ => IdKind::Other,
            };
            (kind, id.value.as_str())
        })
        .collect();
    let action = Action::from(event.action);
    let progress = api::progress_percent(action, event.position_ms, event.runtime_ms);
    let body = api::scrobble_body(kind, &event.title, event.year, &ids, progress)?;
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
async fn refresh_access_token(user: &str, client_id: &str) -> Option<String> {
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
