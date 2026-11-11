// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's domain-event dispatcher
// suite. `notifier::notify` appends a one-line encoding of each received
// event to an instance-global log; the host reads the log back through
// the `report` background task (the established observation channel —
// same trick fixture-tasks uses with `metadata.search`). Any string
// field containing "boom" panics the guest, so tests can force an
// instance recycle from the dispatcher path.
//
// The log is instance-global on purpose: a trap re-instantiates the
// component and zeroes it, which is exactly what lets the host test
// prove the dispatcher kept delivering to the *recycled* instance.

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
use noflex::plugin::types::NotifyEvent;

static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Fixture;

/// Flat, assertion-friendly one-line encoding of a notify-event.
fn encode(event: &NotifyEvent) -> String {
    match event {
        NotifyEvent::WorkAdded(e) => format!("work-added:{}:{}", e.work, e.title),
        NotifyEvent::WorkPlayed(e) => format!("work-played:{}:{}:{}", e.user, e.work, e.title),
        NotifyEvent::Favorite(e) => {
            format!("favorite:{}:{}:{}:{}", e.user, e.work, e.title, e.favorite)
        }
        NotifyEvent::ProgressAtStop(e) => format!(
            "progress-at-stop:{}:{}:{}:{}",
            e.user, e.work, e.title, e.position_ms
        ),
        NotifyEvent::PlaybackStarted(e) => format!(
            "playback-started:{}:{}:{}:{}",
            e.user, e.work, e.title, e.media_file
        ),
        NotifyEvent::PlaybackStopped(e) => format!(
            "playback-stopped:{}:{}:{}:{}",
            e.user, e.work, e.title, e.media_file
        ),
    }
}

impl notifier::Guest for Fixture {
    async fn notify(event: NotifyEvent) -> Result<(), notifier::PluginError> {
        let line = encode(&event);
        if line.contains("boom") {
            panic!("fixture-notifier: boom");
        }
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
                "fixture-notifier: unknown task `{other}`"
            ))),
        }
    }
}

export!(Fixture);
