// SPDX-License-Identifier: AGPL-3.0-only
//
// Local static-embedding provider — first-party WASM plugin.
//
// Runs a Model2Vec static embedding model (minishlab/potion-base-8M,
// 256-dim) inside the component: pure-Rust inference via model2vec-rs,
// no external inference API (self-hosted invariant). The weights are
// not compiled into the `.wasm` — the admin provisions them under the
// host's plugin assets dir and the guest reads them through the
// capability-gated `host-assets` import.
//
// The model loads lazily on the first `embed` call into instance state
// and is cached only on success: a call made before the files are in
// place fails with a plugin-error (never a trap), and a later call
// retries the load.

#![cfg(target_arch = "wasm32")]

use std::cell::OnceCell;

use model2vec_rs::model::StaticModel;

wit_bindgen::generate!({
    path: "../../crates/noflex-plugin-wit/wit",
    world: "embedder-plugin",
    generate_all,
});

/// Embedded plugin manifest. wit-bindgen + the wasm32-wasip2 linker
/// preserves `#[link_section]` items as custom sections in the final
/// component binary, so the host can read this without a post-build step.
#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, embedder};
use noflex::plugin::host_assets;

const TOKENIZER_ASSET: &str = "potion-base-8m/tokenizer.json";
const WEIGHTS_ASSET: &str = "potion-base-8m/model.safetensors";
const CONFIG_ASSET: &str = "potion-base-8m/config.json";

/// The host caps batches at 64 texts; chunk anyway so peak tokenization
/// memory stays bounded if that cap ever moves.
const ENCODE_CHUNK: usize = 32;

thread_local! {
    // Single-threaded component instance, so no OnceLock/Sync needed.
    // Boxed: the model struct is ~1KB, and a boxed Err keeps
    // `OnceCell::set`'s Result small (clippy::result_large_err).
    static MODEL: OnceCell<Box<StaticModel>> = const { OnceCell::new() };
}

struct Embeddings;

async fn read_asset(name: &str) -> Result<Vec<u8>, embedder::PluginError> {
    host_assets::read_file(name.to_string())
        .await
        .map_err(|e| embedder::PluginError::Internal(format!("embeddings: read `{name}`: {e:?}")))
}

async fn ensure_model() -> Result<(), embedder::PluginError> {
    if MODEL.with(|cell| cell.get().is_some()) {
        return Ok(());
    }
    let tokenizer = read_asset(TOKENIZER_ASSET).await?;
    let weights = read_asset(WEIGHTS_ASSET).await?;
    let config = read_asset(CONFIG_ASSET).await?;
    let model = StaticModel::from_bytes(&tokenizer, &weights, &config, None)
        .map_err(|e| embedder::PluginError::Internal(format!("embeddings: load model: {e:#}")))?;
    // Concurrent embed calls may race the load; both copies are
    // identical, so losing the race is not an error.
    let _ = MODEL.with(|cell| cell.set(Box::new(model)));
    Ok(())
}

impl embedder::Guest for Embeddings {
    async fn embed(texts: Vec<String>) -> Result<Vec<Vec<f32>>, embedder::PluginError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        ensure_model().await?;
        MODEL.with(|cell| match cell.get() {
            Some(model) => {
                let mut vectors = Vec::with_capacity(texts.len());
                for chunk in texts.chunks(ENCODE_CHUNK) {
                    vectors.extend(model.encode(chunk));
                }
                Ok(vectors)
            }
            // ensure_model returned Ok, so this is unreachable in
            // practice — a plugin-error still beats a trap.
            None => Err(embedder::PluginError::Internal(
                "embeddings: model missing after successful load".into(),
            )),
        })
    }
}

// -------- Background hook (stub — this plugin registers no [[tasks]]) --------

impl background::Guest for Embeddings {
    async fn run_task(
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        let _ = (cursor, payload);
        Err(background::PluginError::BadInput(format!(
            "embeddings: unknown background task `{task}`"
        )))
    }
}

export!(Embeddings);
