// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for the host's subtitle search: a provider whose
// quota is used up, so every call is `rate-limited` for an hour. Searched
// beside fixture-subtitle, it shows one provider failing doesn't hide
// another's results.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "subtitle-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{
    background,
    subtitle::{self, SubtitleMatch, SubtitleQuery},
};
use noflex::plugin::types::PluginError;

const RESET_SECS: u32 = 3600;

struct Fixture;

impl subtitle::Guest for Fixture {
    async fn search(_query: SubtitleQuery) -> Result<Vec<SubtitleMatch>, PluginError> {
        Err(PluginError::RateLimited(RESET_SECS))
    }

    async fn download(
        _handle: String,
    ) -> Result<(String, wit_bindgen::StreamReader<u8>), PluginError> {
        Err(PluginError::RateLimited(RESET_SECS))
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        Err(background::PluginError::BadInput(format!(
            "fixture-subtitle-quota: unknown task `{task}`"
        )))
    }
}

export!(Fixture);
