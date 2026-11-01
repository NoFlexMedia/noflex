// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's user-link suite.
// `link-start` returns a canned challenge; `link-complete` returns
// canned per-user secrets. A `callback_query` of `read=<key>` instead
// exercises `host-secrets.read-for-user` from inside the guest and
// reports the outcome back as a `read-result` secret, so host tests can
// drive the granted / capability-denied / unavailable / bad-input paths
// through the real wasmtime dispatch. A `callback_query` of
// `enqueue-write=<user>` enqueues the `write-token` background task, whose
// run writes `access-token` for `<user>` via `write-for-user` — proving the
// task runs bound to the user whose call enqueued it, and only that user.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "user-link-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, user_link};
use noflex::plugin::{host_secrets, host_tasks, types::PluginError};

/// The state `link-start` issues; `link-complete` rejects anything else.
const FIXTURE_STATE: &str = "fixture-state-123";

struct Fixture;

impl user_link::Guest for Fixture {
    async fn link_start(user: String) -> Result<user_link::LinkChallenge, PluginError> {
        Ok(user_link::LinkChallenge {
            auth_url: format!("https://example.invalid/authorize?user={user}"),
            state: FIXTURE_STATE.into(),
        })
    }

    async fn link_complete(
        user: String,
        state: String,
        callback_query: String,
    ) -> Result<Vec<user_link::UserSecret>, PluginError> {
        if let Some(target) = callback_query.strip_prefix("enqueue-write=") {
            let outcome = match host_tasks::enqueue("write-token".into(), Some(target.into())).await
            {
                Ok(()) => "ok:enqueued".to_string(),
                Err(e) => format!("err:{e:?}"),
            };
            return Ok(vec![user_link::UserSecret {
                key: "read-result".into(),
                value: outcome,
            }]);
        }
        let requested = callback_query
            .strip_prefix("read-as=")
            .and_then(|request| request.split_once(':'))
            .map(|(requested_user, key)| (requested_user.to_string(), key));
        let requested = requested.or_else(|| {
            callback_query
                .strip_prefix("read=")
                .map(|key| (user.clone(), key))
        });
        if let Some((requested_user, key)) = requested {
            let outcome = match host_secrets::read_for_user(requested_user, key.to_string()).await {
                Ok(value) => format!("ok:{value}"),
                Err(PluginError::CapabilityDenied(_)) => "err:capability-denied".into(),
                Err(PluginError::Unavailable(_)) => "err:unavailable".into(),
                Err(PluginError::BadInput(_)) => "err:bad-input".into(),
                Err(_) => "err:other".into(),
            };
            return Ok(vec![user_link::UserSecret {
                key: "read-result".into(),
                value: outcome,
            }]);
        }
        if state != FIXTURE_STATE {
            return Err(PluginError::BadInput(format!(
                "fixture-user-link: unexpected state `{state}`"
            )));
        }
        Ok(vec![
            user_link::UserSecret {
                key: "access-token".into(),
                value: "fixture-token-abc".into(),
            },
            user_link::UserSecret {
                key: "refresh-token".into(),
                value: "fixture-refresh-def".into(),
            },
        ])
    }
}

// -------- Background hook: `write-token` --------

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        _cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        if task != "write-token" {
            return Err(PluginError::BadInput(format!(
                "fixture-user-link: unknown task `{task}`"
            )));
        }
        // Scheduled ticks carry no payload: nothing to do.
        let Some(user) = payload else {
            return Ok(background::TaskOutcome::Done);
        };
        host_secrets::write_for_user(user, "access-token".into(), "from-background".into()).await?;
        Ok(background::TaskOutcome::Done)
    }
}

export!(Fixture);
