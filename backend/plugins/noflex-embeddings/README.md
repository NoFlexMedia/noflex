# noflex-embeddings

First-party local embedding provider. Embeds with
[EmbeddingGemma 2](https://huggingface.co/google/embeddinggemma-2), truncated to
512 dimensions. The guest holds the model's prompts and tokenizer; the graph runs
natively in the host through the `host-inference` import, because WASM can't reach
the vector instructions or accelerators it needs.

## Why this export

The weights are
[`arankays/embeddinggemma-2-text-int8-cw-onnx`](https://huggingface.co/arankays/embeddinggemma-2-text-int8-cw-onnx),
a community export, chosen by measurement on BEIR SciFact (AVX2 Threadripper, 4 and
16 threads, documents around 230 tokens):

| Runtime and weights                   | Size    | Docs/s at 4 / 16 | Query at 4 / 16 | Cosine to fp32, min |
| ------------------------------------- | ------- | ---------------- | --------------- | ------------------- |
| ONNX Runtime, this export             | 274 MB  | 5.5 / 13.5       | 29 / 23 ms      | 0.9993              |
| OpenVINO, this export                 | 274 MB  | 5.2 / 12.8       | 27 / 19 ms      | 0.9993              |
| llama.cpp, GGUF Q8                    | 310 MB  | 3.9 / 10.1       | 29 / 15 ms      | 0.9991              |
| ONNX Runtime, onnx-community fp32     | 1084 MB | 5.6 / 14.0       | 30 / 22 ms      | reference           |
| ONNX Runtime, onnx-community `q4`     | 174 MB  | 5.5 / —          | 30 / — ms       | 0.949               |
| OpenVINO, onnx-community fp32         | 1084 MB | 5.9 / —          | 33 / — ms       | 0.686               |

On an RTX 4090 through CUDA this export does 434 documents a second and answers a
query in 6.4 ms, about 32 times the 16-thread CPU. nDCG@10 is 0.841 at 768
dimensions, the same as fp32, then 0.842 at the 512 stored here and 0.828 at 256.

llama.cpp uses about half the memory, but offers no NPU path and needs a C++ build.
OpenVINO reaches only Intel accelerators. ONNX Runtime keeps one graph and one Rust
API across CUDA, OpenVINO, QNN and CoreML. The stock onnx-community export silently
gives wrong vectors under OpenVINO, which is why this export is the one to use: it
rewrites the offending `RotaryEmbedding` nodes and stays correct on every runtime.

The host sets ONNX Runtime's `session.disable_quant_qdq`. Unset, this export takes
227 ms per query rather than 29.

## Provisioning the model

A release bundle includes it. From source:

```sh
cargo xtask model-fetch <assets_dir>
```

which verifies each file against the hashes pinned in `packaging/model.toml`. Set
`[plugin] assets_dir` to the same directory. Without the files every `embed` call
fails with a plugin error and search stays lexical; dropping them in takes effect
on the next call. The model unloads after 15 idle minutes and reloads in about a
second.

## Build

```sh
nix develop
cargo xtask plugin-build noflex-embeddings
```
