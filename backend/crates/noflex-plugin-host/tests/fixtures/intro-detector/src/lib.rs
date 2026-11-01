// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's intro-detector worker
// suite. `intro-detector::detect` reads the edition through
// `host-library.lookup-edition-by-id` — the import is the real thing
// under test here — and returns a deterministic detection derived from
// the first and last chapter positions when the edition carries
// chapters, `None` when it has none (or vanished). The background
// export is a stub: this fixture registers no `[[tasks]]`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "intro-detector-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, intro_detector};
use noflex::plugin::host_library;

struct Fixture;

impl intro_detector::Guest for Fixture {
    async fn detect(
        edition: String,
    ) -> Result<Option<intro_detector::IntroDetection>, intro_detector::PluginError> {
        let Some(summary) = host_library::lookup_edition_by_id(edition).await else {
            return Ok(None);
        };
        let (Some(first), Some(last)) = (summary.chapters.first(), summary.chapters.last()) else {
            return Ok(None);
        };
        Ok(Some(intro_detector::IntroDetection {
            intro_start_ms: Some(first.position_ms),
            intro_end_ms: Some(last.position_ms),
            credits_start_ms: summary.runtime_ms.map(|ms| ms - 60_000),
            confidence: 1.0,
        }))
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        let _ = (task, cursor, payload);
        Err(background::PluginError::BadInput(
            "fixture-intro-detector: no background tasks".into(),
        ))
    }
}

export!(Fixture);
