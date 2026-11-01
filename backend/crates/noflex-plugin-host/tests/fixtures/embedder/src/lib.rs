// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's embedder suite. Vectors
// are deterministic and text-sensitive — byte sums bucketed by position
// mod 8, scaled to [0,1] — so tests can assert stability across calls
// and divergence across texts without shipping a real model. The
// sentinel text "asset-check" reads `fixture-model.txt` through
// host-assets and reports the byte length in vector[0], proving the
// capability-gated import round-trips (and surfacing the host's
// plugin-error verbatim when it doesn't).

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "embedder-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, embedder};
use noflex::plugin::host_assets;

const DIMENSIONS: usize = 8;
const ASSET_SENTINEL: &str = "asset-check";

struct Fixture;

fn embed_text(text: &str) -> Vec<f32> {
    let mut buckets = [0u64; DIMENSIONS];
    for (i, b) in text.bytes().enumerate() {
        buckets[i % DIMENSIONS] += u64::from(b);
    }
    buckets
        .iter()
        .map(|sum| (sum % 256) as f32 / 255.0)
        .collect()
}

impl embedder::Guest for Fixture {
    async fn embed(texts: Vec<String>) -> Result<Vec<Vec<f32>>, embedder::PluginError> {
        let mut out = Vec::with_capacity(texts.len());
        for text in &texts {
            let mut vector = embed_text(text);
            if text == ASSET_SENTINEL {
                let bytes = host_assets::read_file("fixture-model.txt".to_string()).await?;
                vector[0] = bytes.len() as f32;
            }
            out.push(vector);
        }
        Ok(out)
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        Err(background::PluginError::BadInput(format!(
            "fixture-embedder: unknown task `{task}`"
        )))
    }
}

export!(Fixture);
