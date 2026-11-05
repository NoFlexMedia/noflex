# NoFlex

_Your media, your server_ 🎬📺🎵

NoFlex is a self-hosted media server for movies, shows, music, audiobooks, and photos, written from scratch in Rust.

> **Status: pre-alpha.** NoFlex is under active development and not yet ready to hold your library.

## Layout & Tech Stack

**🦀 Backend — Rust.** A single Cargo workspace in `backend/` holds the server, scanner, transcoder, storage, and plugin runtime.

**🔗 API — GraphQL, code-first.** The schema is authored in Rust with `async-graphql`, and it is the only client surface: no REST layer, no compatibility shims. Subscriptions ride the same endpoint over WebSocket.

**🧩 Plugins — WebAssembly components.** Metadata and artwork providers, external login, scrobblers, notifications, and embeddings are all `.wasm` plugins running in Wasmtime, first-party ones included.

**🌐 Web — SvelteKit.** `web/` is a Svelte 5 client using Houdini for GraphQL, embedded into the server binary and served from the same origin.

**🗃️ Storage — Turso.** An in-process, SQLite-compatible engine by default, behind a backend-agnostic `Storage` trait.

**❄️ Nix.** One flake provides the dev shell, the packages, CI checks, and a hardened NixOS module.

## Philosophy

NoFlex takes inspiration from Plex and Jellyfin, then makes a few choices of its own.

- **Plex-style data model.** A _work_ has one or more _editions_ (Theatrical, Director's Cut), each with one or more _media files_ (4K HDR and 1080p versions, multi-disc parts). The scanner understands Plex naming conventions natively.
- **One canonical API.** Every client — ours or yours — talks GraphQL. The Rust domain type _is_ the GraphQL type, so the API never drifts from the model.
- **Everything external is a plugin.** Plugins are sandboxed, capability-gated, and audit-logged; they can't touch the database or the network except through narrow host imports. Because first-party providers use the same boundary, the plugin API is exercised on every startup.
- **Self-hosted, end to end.** No accounts with us, no relay, no cloud inference unless you configure it. Search is hybrid — full-text plus vector search over embeddings computed locally while your server is idle — and media bytes never leave your network unless you send them.
- **One icon language.** Every client draws its icons from [Phosphor](https://phosphoricons.com) (`phosphor-svelte` on the web), never hand-drawn SVGs, so the same glyph means the same thing everywhere.
- **Pluggable storage.** Turso is the default; another engine is a crate that passes the shared conformance suite, not a fork.

## Getting Started

Read [`docs/GETTING_STARTED.md`](docs/GETTING_STARTED.md). The short version:

```bash
nix develop
cd backend && cargo run -p noflex-server   # then open http://127.0.0.1:8096
```

## Documentation

- [`docs/GETTING_STARTED.md`](docs/GETTING_STARTED.md) — build, run, and test from source.
- [`docs/CONFIGURATION.md`](docs/CONFIGURATION.md) — every server setting, in TOML and environment variables.
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) — NixOS, remote access, backups, hardware transcoding.
- [`docs/PACKAGING.md`](docs/PACKAGING.md) — release bundles, and why we build our own ffmpeg.
- [`docs/PLUGINS.md`](docs/PLUGINS.md) — the plugin model, and how to write your own.
- [`docs/PLUGIN_LICENSING.md`](docs/PLUGIN_LICENSING.md) — how AGPL applies (and doesn't) to plugins.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how changes land.

## Git Worktrees

Interested in having multiple checkouts or working on multiple branches at once? We support this!
Read [`docs/GIT_WORKTREES.md`](docs/GIT_WORKTREES.md).

## License

[AGPL-3.0-only](LICENSE). Plugins may use any license; see [`docs/PLUGIN_LICENSING.md`](docs/PLUGIN_LICENSING.md).
