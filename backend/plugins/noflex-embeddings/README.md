# noflex-embeddings

First-party local embedding provider. Runs the Model2Vec static
embedding model [`minishlab/potion-base-8M`](https://huggingface.co/minishlab/potion-base-8M)
(7.5M params, 256-dim) inside the WASM component via
[`model2vec-rs`](https://crates.io/crates/model2vec-rs) — pure-Rust
inference, no external API. This is what fills NoFlex's vector index in
real deployments: the host embeds work texts at scan time and query
text at search time, and stores the returned vectors itself under the
manifest-claimed `potion-base-8m` namespace.

## Provisioning the model

The `.wasm` ships no weights. Download the three model files from
Hugging Face:

```sh
curl -LO https://huggingface.co/minishlab/potion-base-8M/resolve/main/model.safetensors
curl -LO https://huggingface.co/minishlab/potion-base-8M/resolve/main/tokenizer.json
curl -LO https://huggingface.co/minishlab/potion-base-8M/resolve/main/config.json
```

Place them under a `potion-base-8m/` directory inside the host's plugin
assets dir (the `[plugin] assets_dir` in the server config):

```
<assets_dir>/potion-base-8m/model.safetensors
<assets_dir>/potion-base-8m/tokenizer.json
<assets_dir>/potion-base-8m/config.json
```

The guest reads them through the capability-gated `host-assets.read-file`
import (capability `assets.read`, already declared in the manifest); the
host resolves the names, rejects traversal, and audit-logs the reads.
If `assets_dir` is unset the import is disabled and every `embed` call
fails with a plugin error.

## Load behavior

The model loads lazily on the first `embed` call and is cached in
instance state — per-call latency after that is tokenization plus a
matrix lookup. A call made before the files are in place returns a
plugin error (never a trap) and the next call retries, so dropping the
files in takes effect without a restart; a `.wasm` rebuild or plugin
reload also re-reads them.

## Versioning

Host and plugins are co-versioned: a wit-bindgen / wasmtime bump lands
across the WIT, the host runtime, and every plugin in one change — no
compatibility shims. See `../README.md` ("Support policy").

## Build

```sh
nix develop
cargo xtask plugin-build noflex-embeddings
```
