// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's post-scan suite.
// `post-scan.on-scan-complete` appends a one-line encoding of the call
// (library id plus the three work-id lists) to an instance-global log;
// the host reads the log back through the `report` background task —
// the same observation channel fixture-notifier uses.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "post-scan-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use std::sync::Mutex;

use exports::noflex::plugin::{background, post_scan};

static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Fixture;

impl post_scan::Guest for Fixture {
    async fn on_scan_complete(
        library: String,
        added: Vec<String>,
        updated: Vec<String>,
        removed: Vec<String>,
    ) -> Result<(), post_scan::PluginError> {
        let line = format!(
            "scan-complete:{}:{}|{}|{}",
            library,
            added.join(","),
            updated.join(","),
            removed.join(",")
        );
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
                "fixture-post-scan: unknown task `{other}`"
            ))),
        }
    }
}

export!(Fixture);
