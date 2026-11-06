# Rust Backend

All Rust lives in `backend/` under a single Cargo workspace. Run cargo from here, inside `nix develop`. First-party WASM plugins in `plugins/` sit **outside** the workspace — they target `wasm32-wasip2`.

## Build/Lint/Test Commands

```bash
cargo build --all-targets                              # Build everything
cargo test --workspace                                 # Run all tests (or: cargo nextest run --workspace)
cargo test -p <crate>                                  # Test one crate
cargo nextest run -p noflex-storage-turso --test conformance  # Storage conformance suite
cargo fmt --all                                        # Format (nightly rustfmt via $RUSTFMT)
cargo clippy --workspace --all-targets -- -D warnings  # Lint
cargo xtask plugin-build <name>                        # Build a plugin + embed its manifest
cargo xtask plugin-dev <name>                          # ...and symlink it into the local plugin dir
```

The dev shell routes `rustc` through a shared `sccache`; see [`docs/GIT_WORKTREES.md`](../docs/GIT_WORKTREES.md).

## Layout

A crate's `lib.rs` is the right starting point.

- `noflex-core` — domain enums, typed IDs, plugin provider traits (`src/providers/`), shared errors.
- `noflex-storage` — `Storage` trait, storage-shape domain types, query DTOs, conformance suite (`test_suite.rs`).
- `noflex-storage-<engine>` — one backend per engine (Turso today), owning its `migrations/`.
- `noflex-graphql` — canonical GraphQL schema and resolvers; the SDL snapshot lives in `tests/snapshots/schema.graphql`.
- `noflex-scanner` — filesystem walker, Plex-style filename/folder parser, local artwork discovery.
- `noflex-metadata` — shared metadata helpers (payload application, merging). Providers ship as plugins.
- `noflex-transcoder` — ffmpeg subprocess manager, playback decisions, HLS planning.
- `noflex-auth` — accounts, sessions, tokens.
- `noflex-plugin-wit` — the canonical WIT world; single source of truth for the plugin ABI.
- `noflex-plugin-host` — Wasmtime runtime: registry, dispatch, hot-reload, audit.
- `noflex-plugin-sdk` — plugin-author crate: bindings macro, caching and singleflight helpers.
- `noflex-server` — the binary: config, wiring, HTTP routes, background workers.

## Dependencies

- Versions are defined once in `backend/Cargo.toml` `[workspace.dependencies]`; crates use `{ workspace = true }`.
- List `noflex-*` crates first, then external dependencies.
- Time is `time` (`OffsetDateTime`). Never `chrono`.
- No ORMs. Storage backends write SQL and prepared statements directly.

## Code Style

- `cargo fmt` enforces nested, `StdExternalCrate`-grouped imports. Don't hand-format imports; always run `cargo fmt --all` after finishing edits.
- Use `#[derive(...)]` for common traits (`Clone`, `Debug`, `PartialEq`, ...). Shared state is `Arc<T>`.
- snake_case for functions and variables, CamelCase for types and enums.
- Public APIs take typed newtypes over `Uuid` (v7), never a raw `Uuid` — see `WorkId`.
- Prefer trait implementations, then inherent methods and associated functions, over standalone free functions.
- Document every `pub` and `pub(crate)` item, and significant private ones, with stdlib-bar Rustdoc: a precise contract for the caller — preconditions, errors, edge cases, ownership — in terse Markdown with intra-doc links. If the signature already says it, delete the sentence.
- No panicking `unwrap()`/`expect()` outside tests and obvious static initialization. Library code returns `Result`.
- Justify any `#[allow(clippy::...)]` with a one-line comment above it. No blanket allows.
- Unit tests live in an inline `#[cfg(test)] mod tests` at the bottom of the file; cross-crate and end-to-end tests live in the crate's `tests/`.

## Error Handling

| Use Case                  | Error Type                                        | Reason                                      |
| ------------------------- | ------------------------------------------------- | ------------------------------------------- |
| Library crates            | Custom `thiserror` enum                           | Stable API, specific variants               |
| Storage backends          | Engine errors mapped to the shared `StorageError` | Engine types never leave the backend crate  |
| GraphQL resolvers         | `async_graphql::Error`                            | Client-safe messages, `code` extensions     |
| Binary edge (server main) | `anyhow::Result`                                  | Simple propagation with `.context()`        |

Resolver errors must not leak internals (paths, SQL, stack traces). Log the detail before converting it.

## Logging

- Keep incidental values (IDs, counts, paths, states) in named `tracing` fields, not interpolated into messages: `tracing::warn!(plugin = %id, error = %e, "metadata fetch: dispatch failed")`. `%` for `Display`, `?` for `Debug`.
- Report a failure once along its propagation path: helpers return contextual errors, the handler logs. Keep breadcrumbs for errors that are retried or swallowed.
- Never log credentials, tokens, or full egress URLs.

## Architecture Invariants

Don't break these without an architecture discussion in the PR description.

### Storage

- The `Storage` trait is backend-agnostic: no SQL, engine types, or row types in its surface. Methods are operation-level (`upsert_work`, `search_hybrid`), never query-level (`execute`).
- Pagination uses opaque cursors, never `OFFSET`/`LIMIT`.
- Migrations are per-backend, applied in lexical order, idempotent.
- Model subtypes with Class Table Inheritance (CTI) whenever relevant. The parent table holds the shared columns and a `kind` discriminator; each subtype's table holds only what's specific to it, with a primary key that references the parent `ON DELETE CASCADE`. See `media_streams` and `video_streams` in `noflex-storage-turso`.
- Every backend passes the shared conformance suite. New `Storage` methods get conformance tests.
- Storage-shape domain types (`Library`, `Work`, `Edition`, `HistoryRow`, ...) derive `async_graphql::SimpleObject` in their home crate and are used directly by GraphQL — no `XxxGql` mirrors, `pub type` aliases, or re-tagging `From` impls. The trait surface itself stays GraphQL-free.

### GraphQL

- Domain logic lives behind services and traits, never in HTTP handlers.
- A mutation that removes something returns what it removed, never a `Boolean`, so clients can drop it from their caches by id. Make it nullable where removing nothing isn't an error.
- Schema changes regenerate `noflex-graphql/tests/snapshots/schema.graphql` (`NOFLEX_UPDATE_SNAPSHOTS=1 cargo test -p noflex-graphql --test it schema_snapshot`); the drift test fails otherwise, and the web and Flutter clients validate against it.

### Search

- Modes are lexical, vector, and hybrid (default). Hybrid fuses both with reciprocal rank fusion.
- Embeddings are computed locally unless an administrator configures an OpenAI-compatible endpoint, which is never a default. Every backend implements `upsert_embedding` and `search_vector`.
- **"Is it still indexing" is `Work.indexing`, never `metadataStatus`.** A work with no match stays `Unconfirmed` for good. `IndexingProgress` is in memory, fed by scan events and the metadata worker, and a rescan or a retry doesn't count as new work. Zero complexity, so cards can afford it.
- **Removing an optimized version doesn't stop the planner wanting it.** It's remade if a mark or a prediction still wants it; that's deliberate, and the admin dialog shows who and takes back marks. `enqueue_optimized_version` refreshes `plan_json` on every plan, so `targets` says who it's for now. Read counts through `optimized_version_stats`: `list_optimized_versions` carries every row's index and probe.
- A vector index belongs to one model. Anything that changes a model's output changes its namespace, and the sweep reindexes; old vectors are pruned only once the new index is complete.

### Plugins

- **Plugins are WASM components.** First-party providers (TMDb, TVDB, MusicBrainz) ship as `.wasm` too; the server has no native `MetadataProvider`, so first-party code exercises the plugin ABI on every startup.
- **One instance per plugin** (WASI 0.3 native async). Plugins own their concurrency, batching, and rate limiting.
- **Plugins never see `Storage`.** Data access goes through narrow, named host imports (`host-library.lookup-work-by-id`). Add an import per query type; never a general query interface.
- **Plugins own egress.** The host never fetches provider URLs itself; network access goes through `host-net`, allowlisted and audit-logged. The one exception is the embeddings endpoint an administrator configures, whose URL comes from them and not from a plugin.
- **Models run in the host.** Plugins bring the tokenizer and prompts and call `host-inference`; the operator chooses the device. WASM can't reach an accelerator, and a plugin holding the GPU couldn't coordinate with transcodes.
- **Provider-exclusive fetch.** `fetch()` runs on exactly one candidate per work; fields never mix across providers. Fall back to the next candidate only on `fetch()` failure.
- **Credit follows the data.** `metadata.attribution()` is stored per plugin in `provider_attributions` when it loads and outlives the plugin. `Work.metadata_provider` and `Image.provider` say who to credit; every provider-written row sets them.
- **TMDb and TheTVDB ship shared API keys** (`SHARED_API_KEY`), so metadata needs no setup. An operator's `api_key` secret overrides them.
- **Trust by default** for now: plugins in the search path load immediately, every capability use audit-logged.
- Args and returns are owned, serializable, and `Send` — they cross the WIT boundary. No `&dyn Trait`, no host-pinned lifetimes.
- Plugins may use any license; see [`docs/PLUGIN_LICENSING.md`](../docs/PLUGIN_LICENSING.md).

### Plugin ABI versioning

- The WIT in `noflex-plugin-wit/wit/` is canonical; each provider trait in `noflex-core/src/providers/` mirrors one hook (`background` is host plumbing and has no trait).
- Pre-1.0, the WIT, host runtime, SDK, every shipped plugin, and the fixture guests under `noflex-plugin-host/tests/fixtures/` move together in **one co-versioned change**: bump `noflex-plugin-wit`'s `VERSION`/`API_MINOR` and every manifest's `noflex_api_version`. No shims, no mixed host/guest versions. wasmtime and wit-bindgen bumps follow the same rule.
- From 1.0, the trait surface is semver-stable: add methods with default implementations; change a signature by shipping `FooProvider2`, deprecating the old trait, and removing it after two minor versions. Review the `cargo public-api` diff of `noflex-core` whenever a provider trait changes.

## Recipes

### Adding a storage backend

1. New crate `crates/noflex-storage-<engine>` depending on `noflex-storage` and the engine's client.
2. Implement `Storage` and `Transaction`; map engine errors to `StorageError` at the boundary.
3. Add migrations in the crate's `migrations/`.
4. Implement lexical, vector, and hybrid search with whatever the engine offers.
5. Add `tests/conformance.rs` running `noflex_storage::test_suite` — it must pass before merge.
6. Add a config variant in `noflex-server` so operators can select it.

### Adding a first-party plugin

1. New `cdylib` crate under `plugins/<name>/`, using the same `wit-bindgen` version and features as the existing plugins.
2. Declare `[package.metadata.noflex-plugin]` in its `Cargo.toml` (`plugin_id`, `hooks`, `capabilities`, ...).
3. Implement the WIT `Guest` traits; stub `background::run_task` if you register no tasks.
4. Test parsing against recorded fixtures — no network in tests.
5. `cargo xtask plugin-build <name>`, then `cargo xtask plugin-dev <name>` to hot-reload it into a running server.

See [`docs/PLUGINS.md`](../docs/PLUGINS.md) for hooks, host imports, and manifest fields.
