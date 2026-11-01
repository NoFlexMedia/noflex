// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's `host-net` suite. A
// `custom` notify-event whose payload is `METHOD URL [body-len]` drives
// a host-performed HTTP request from inside the guest (an optional
// third field generates a body of that many bytes, for the size-cap
// tests). The outcome — status + body, or the plugin-error variant —
// is appended to an instance-global log the host reads back through
// the `report` background task, so tests exercise the real wasm
// boundary end to end.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "notifier-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use std::sync::Mutex;

use exports::noflex::plugin::{background, notifier};
use noflex::plugin::{
    host_net,
    types::{NotifyEvent, PluginError},
};

static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Fixture;

/// Run the guest-side `host-net.request` call and encode the outcome as
/// one assertion-friendly line.
async fn drive(payload: &str) -> String {
    let mut parts = payload.split_whitespace();
    let (Some(method), Some(url)) = (parts.next(), parts.next()) else {
        return "err:fixture:malformed-payload".into();
    };
    let body = parts
        .next()
        .and_then(|n| n.parse::<usize>().ok())
        .map(|n| vec![b'x'; n]);
    let req = host_net::HttpRequest {
        method: method.to_string(),
        url: url.to_string(),
        headers: vec![],
        body,
        timeout_ms: None,
    };
    match host_net::request(req).await {
        Ok(resp) => format!("ok:{}:{}", resp.status, String::from_utf8_lossy(&resp.body)),
        Err(PluginError::CapabilityDenied(m)) => format!("err:capability-denied:{m}"),
        Err(PluginError::BadInput(m)) => format!("err:bad-input:{m}"),
        Err(PluginError::Unavailable(m)) => format!("err:unavailable:{m}"),
        Err(other) => format!("err:other:{other:?}"),
    }
}

impl notifier::Guest for Fixture {
    async fn notify(event: NotifyEvent) -> Result<(), PluginError> {
        let NotifyEvent::Custom(custom) = event else {
            return Ok(());
        };
        if custom.event != "net" {
            return Ok(());
        }
        let line = drive(&custom.payload_json).await;
        RECEIVED.lock().expect("received log poisoned").push(line);
        Ok(())
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        match task.as_str() {
            // Hand the whole log back as the cursor; `more` is just the
            // transport, the host test never re-invokes.
            "report" => Ok(background::TaskOutcome::More(
                RECEIVED.lock().expect("received log poisoned").join("\n"),
            )),
            other => Err(background::PluginError::BadInput(format!(
                "fixture-net: unknown task `{other}`"
            ))),
        }
    }
}

export!(Fixture);
