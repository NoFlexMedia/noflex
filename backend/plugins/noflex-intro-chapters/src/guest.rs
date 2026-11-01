// SPDX-License-Identifier: AGPL-3.0-only
//
// Intro-chapters guest: implements `intro-detector` by reading the
// edition's chapters, runtime, and existing markers through
// `host-library.lookup-edition-by-id` and running the pure heuristic in
// `crate::detect` over them. The background export is a stub — this
// plugin registers no `[[tasks]]`, so the host never calls it.
// Compiled only for wasm32 — the host-testable heuristic lives in
// `crate::detect`.

use exports::noflex::plugin::{background, intro_detector};
use noflex::plugin::{host_library, host_log};

use crate::detect::{self, Chapter, ExistingMarkers};

wit_bindgen::generate!({
    path: "../../crates/noflex-plugin-wit/wit",
    world: "intro-detector-plugin",
    generate_all,
});

/// Embedded plugin manifest. wit-bindgen + the wasm32-wasip2 linker
/// preserves `#[link_section]` items as custom sections in the final
/// component binary, so the host can read this without a post-build step.
#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

struct IntroChapters;

impl intro_detector::Guest for IntroChapters {
    async fn detect(
        edition: String,
    ) -> Result<Option<intro_detector::IntroDetection>, intro_detector::PluginError> {
        let Some(summary) = host_library::lookup_edition_by_id(edition.clone()).await else {
            // A vanished edition is not an error — the host persists
            // the outcome itself and "nothing found" is the honest one.
            host_log::log(
                host_log::Level::Debug,
                "intro-chapters",
                &format!("intro-chapters: edition {edition} not found, skipping"),
                &[],
            );
            return Ok(None);
        };
        let chapters: Vec<Chapter> = summary
            .chapters
            .iter()
            .map(|c| Chapter {
                position_ms: c.position_ms,
                title: c.title.clone(),
            })
            .collect();
        let existing = ExistingMarkers {
            intro_start_ms: summary.intro_start_ms,
            intro_end_ms: summary.intro_end_ms,
            credits_start_ms: summary.credits_start_ms,
        };
        let result = detect::detect(&chapters, summary.runtime_ms, existing);
        if let Some(d) = &result {
            host_log::log(
                host_log::Level::Info,
                "intro-chapters",
                &format!(
                    "intro-chapters: {} `{}` → intro {:?}–{:?}, credits {:?} (confidence {})",
                    edition,
                    summary.name,
                    d.intro_start_ms,
                    d.intro_end_ms,
                    d.credits_start_ms,
                    d.confidence
                ),
                &[],
            );
        }
        Ok(result.map(|d| intro_detector::IntroDetection {
            intro_start_ms: d.intro_start_ms,
            intro_end_ms: d.intro_end_ms,
            credits_start_ms: d.credits_start_ms,
            confidence: d.confidence,
        }))
    }
}

// -------- Background hook (stub — this plugin registers no [[tasks]]) --------

impl background::Guest for IntroChapters {
    async fn run_task(
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        let _ = (cursor, payload);
        host_log::log(
            host_log::Level::Debug,
            "intro-chapters",
            &format!("intro-chapters: ignoring unexpected background task `{task}`"),
            &[],
        );
        Ok(background::TaskOutcome::Done)
    }
}

export!(IntroChapters);
