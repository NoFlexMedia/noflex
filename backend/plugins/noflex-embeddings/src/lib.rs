// SPDX-License-Identifier: AGPL-3.0-only
//
// Local embedding provider — first-party WASM plugin.
//
// Embeds with EmbeddingGemma 2, no external inference API (self-hosted
// invariant). The guest owns what's particular to the model: its prompts, its
// tokenizer, and cutting its output to the claimed dimensionality. The graph
// runs natively in the host, behind the capability-gated `host-inference`
// import, because WASM can't reach the vector instructions or accelerators a
// transformer needs. Neither weights nor tokenizer are compiled into the `.wasm`:
// a release bundle carries them in the host's plugin assets dir.
//
// The tokenizer loads lazily on the first `embed` call into instance state and
// is cached only on success: a call made before the files are in place fails
// with a plugin-error (never a trap), and a later call retries the load.

#![cfg(target_arch = "wasm32")]

use std::cell::OnceCell;

use tokenizers::{Tokenizer, TruncationParams};

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

use exports::noflex::plugin::{
    background,
    embedder::{self, EmbedInput},
};
use noflex::plugin::{host_assets, host_inference};

const TOKENIZER_ASSET: &str = "embeddinggemma-2/tokenizer.json";
const MODEL_ASSET: &str = "embeddinggemma-2/model_text.onnx";

/// The manifest's `[embedding] dimensions`. The model outputs 768 and is trained
/// so a prefix stands alone; on BEIR SciFact, nDCG@10 is 0.841 at 768, 0.842 here
/// and 0.828 at 256, and a vector here costs a third less to store and compare.
const DIMENSIONS: usize = 512;

/// Longest token sequence embedded. Inference time grows with length, and the
/// host caps document text well under this already.
const MAX_TOKENS: usize = 256;

thread_local! {
    // Single-threaded component instance, so no OnceLock/Sync needed.
    static TOKENIZER: OnceCell<Tokenizer> = const { OnceCell::new() };
}

struct Embeddings;

async fn ensure_tokenizer() -> Result<(), embedder::PluginError> {
    if TOKENIZER.with(|cell| cell.get().is_some()) {
        return Ok(());
    }
    let bytes = host_assets::read_file(TOKENIZER_ASSET.to_string())
        .await
        .map_err(|e| {
            embedder::PluginError::Internal(format!("embeddings: read tokenizer: {e:?}"))
        })?;
    let mut tokenizer = Tokenizer::from_bytes(&bytes)
        .map_err(|e| embedder::PluginError::Internal(format!("embeddings: load tokenizer: {e}")))?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: MAX_TOKENS,
            ..TruncationParams::default()
        }))
        .map_err(|e| embedder::PluginError::Internal(format!("embeddings: truncation: {e}")))?;
    // Concurrent embed calls may race the load; both copies are
    // identical, so losing the race is not an error.
    let _ = TOKENIZER.with(|cell| cell.set(tokenizer));
    Ok(())
}

/// The model's task prompts, which it was trained with and retrieves worse
/// without.
fn prompt(input: &EmbedInput) -> String {
    match input {
        EmbedInput::Query(query) => format!("task: search result | query: {query}"),
        EmbedInput::Document(document) => {
            format!("title: {} | text: {}", document.title, document.text)
        }
    }
}

fn tokenize(inputs: &[EmbedInput]) -> Result<Vec<Vec<u32>>, embedder::PluginError> {
    TOKENIZER.with(|cell| {
        let tokenizer = cell.get().ok_or_else(|| {
            embedder::PluginError::Internal("embeddings: tokenizer missing after load".into())
        })?;
        inputs
            .iter()
            .map(|input| {
                tokenizer
                    .encode(prompt(input), true)
                    .map(|encoding| encoding.get_ids().to_vec())
                    .map_err(|e| {
                        embedder::PluginError::Internal(format!("embeddings: tokenize: {e}"))
                    })
            })
            .collect()
    })
}

/// A shortened vector is only comparable once it's back to unit length.
fn shorten(mut vector: Vec<f32>) -> Result<Vec<f32>, embedder::PluginError> {
    if vector.len() < DIMENSIONS {
        return Err(embedder::PluginError::Internal(format!(
            "embeddings: model output {} dimensions, want at least {DIMENSIONS}",
            vector.len()
        )));
    }
    vector.truncate(DIMENSIONS);
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        vector.iter_mut().for_each(|v| *v /= norm);
    }
    Ok(vector)
}

impl embedder::Guest for Embeddings {
    async fn embed(inputs: Vec<EmbedInput>) -> Result<Vec<Vec<f32>>, embedder::PluginError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        ensure_tokenizer().await?;
        let tokens = tokenize(&inputs)?;
        host_inference::embed_tokens(MODEL_ASSET.to_string(), tokens)
            .await?
            .into_iter()
            .map(shorten)
            .collect()
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
