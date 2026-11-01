// SPDX-License-Identifier: AGPL-3.0-only
//
// Webhook guest: implements the `notifier` hook by POSTing every event
// to the operator-configured URL through the capability-gated
// `host-net` import (guests have no socket access). Compiled only for
// wasm32 — the host-testable mapping lives in `crate::payload`.

use std::cell::OnceCell;

use exports::noflex::plugin::{background, notifier};
use noflex::plugin::{
    host_config, host_log, host_net, host_secrets,
    types::{NotifyEvent, PluginError},
};

use crate::payload::{self, Config};

wit_bindgen::generate!({
    path: "../../crates/noflex-plugin-wit/wit",
    world: "notifier-plugin",
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

thread_local! {
    // Config is read once and cached: the host re-instantiates the
    // plugin on config change, so a stale cache is impossible. `None`
    // means "parsed once, unusable" — misconfig is an operator error,
    // logged once, and every notify no-ops rather than trapping the
    // instance.
    static CONFIG: OnceCell<Option<Config>> = const { OnceCell::new() };
}

struct Webhook;

fn log(level: host_log::Level, message: &str) {
    host_log::log(level, "webhook", message, &[]);
}

fn config() -> Option<Config> {
    CONFIG.with(|cell| {
        cell.get_or_init(|| match payload::parse_config(&host_config::read()) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                log(
                    host_log::Level::Error,
                    &format!("webhook: unusable config, every notify is a no-op: {e}"),
                );
                None
            }
        })
        .clone()
    })
}

impl From<NotifyEvent> for payload::Event {
    fn from(event: NotifyEvent) -> Self {
        use payload::Event as E;
        match event {
            NotifyEvent::WorkAdded(e) => E::WorkAdded {
                work: e.work,
                title: e.title,
            },
            NotifyEvent::WorkPlayed(e) => E::WorkPlayed {
                user: e.user,
                work: e.work,
                title: e.title,
            },
            NotifyEvent::ScanStarted(e) => E::ScanStarted {
                library_name: e.library_name,
            },
            NotifyEvent::ScanCompleted(e) => E::ScanCompleted {
                library_name: e.library_name,
                added: e.added,
                updated: e.updated,
                removed: e.removed,
            },
            NotifyEvent::Favorite(e) => E::Favorite {
                user: e.user,
                work: e.work,
                title: e.title,
                favorite: e.favorite,
            },
            NotifyEvent::ProgressAtStop(e) => E::ProgressAtStop {
                user: e.user,
                work: e.work,
                title: e.title,
                position_ms: e.position_ms,
            },
            NotifyEvent::PlaybackStarted(e) => E::PlaybackStarted {
                user: e.user,
                work: e.work,
                title: e.title,
                media_file: e.media_file,
            },
            NotifyEvent::PlaybackStopped(e) => E::PlaybackStopped {
                user: e.user,
                work: e.work,
                title: e.title,
                media_file: e.media_file,
            },
            NotifyEvent::Custom(e) => E::Custom {
                event: e.event,
                payload_json: e.payload_json,
            },
        }
    }
}

impl notifier::Guest for Webhook {
    async fn notify(event: NotifyEvent) -> Result<(), PluginError> {
        let Some(cfg) = config() else {
            return Ok(());
        };
        let event = payload::Event::from(event);

        // A missing secret downgrades to an unauthenticated POST instead
        // of failing the notification — webhooks are lossy by design.
        let auth = match &cfg.auth_secret_key {
            Some(key) => match host_secrets::read(key.clone()).await {
                Ok(value) => Some(value),
                Err(e) => {
                    log(
                        host_log::Level::Warn,
                        &format!(
                            "webhook: secret `{key}` unreadable, sending without Authorization: {e:?}"
                        ),
                    );
                    None
                }
            },
            None => None,
        };

        let request = host_net::HttpRequest {
            method: "POST".into(),
            url: cfg.url.clone(),
            headers: payload::headers(auth.as_deref()),
            body: Some(event.body().into_bytes()),
            timeout_ms: Some(TIMEOUT_MS),
        };
        // Delivery failures are logged, never propagated: a dead webhook
        // endpoint must not turn into notifier errors on the host's
        // dispatch loop.
        match host_net::request(request).await {
            Ok(resp) if resp.status >= 400 => log(
                host_log::Level::Warn,
                &format!("webhook: endpoint answered {}", resp.status),
            ),
            Ok(_) => {}
            Err(e) => log(
                host_log::Level::Error,
                &format!("webhook: POST failed: {e:?}"),
            ),
        }
        Ok(())
    }
}

// -------- Background hook (stub — this plugin registers no [[tasks]]) --------

impl background::Guest for Webhook {
    async fn run_task(
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        let _ = (cursor, payload);
        Err(background::PluginError::BadInput(format!(
            "webhook: unknown background task `{task}`"
        )))
    }
}

export!(Webhook);
