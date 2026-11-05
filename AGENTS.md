# NoFlex

NoFlex is a self-hosted media server: a Rust backend whose only client surface is a GraphQL API, WASM plugins for every provider integration, a SvelteKit web client, and a Flutter app for phones, tablets, foldables and desktops.

## Read every `AGENTS.md` on your path

Each project subdirectory owns an `AGENTS.md` describing its stack, commands, and conventions. **Always read the `AGENTS.md` of every directory you work in** — including nested ones (a crate, a plugin, a component folder) — before making changes. This root file deliberately does not duplicate those details.

- [`backend/AGENTS.md`](backend/AGENTS.md) — Rust workspace, plugins, architecture invariants.
- [`web/AGENTS.md`](web/AGENTS.md) — SvelteKit client.
- [`flutter/AGENTS.md`](flutter/AGENTS.md) — Flutter app: signals, playback, downloads, test devices.

## Layout

- `backend/` — Cargo workspace: crates in `backend/crates/`, first-party WASM plugins in `backend/plugins/`, `cargo xtask` in `backend/xtask/`.
- `web/` — SvelteKit client, embedded into the server binary at build time.
- `flutter/` — Flutter app for Android, iOS, macOS, Windows and Linux.
- `nix/` — NixOS module. `flake.nix` at the root provides the dev shell, packages, and checks.
- `docs/` — operator and contributor docs, linked from `README.md`.

## Domain

- The data model is **Plex-style**: a _Work_ has one or more _Editions_ (Theatrical, Director's Cut), each with one or more _MediaFiles_ (versions: multi-disc parts, HDR and SDR encodes). Never conflate these levels.
- The **GraphQL API** is canonical. New features land there first; clients consume nothing else.
- **Tags are the one taxonomy.** Genres, moods, studios, people, ratings, quality badges, edition labels, and user tags share one `tags` table split by `namespace`. Add a namespace, never a taxonomy table.
- **Self-hosted.** No cloud services in core paths; optional plugins may reach the internet.

## Conventions

- The toolchain comes from the Nix flake — enter it with `nix develop` or `direnv`. Never `rustup`, never a system `cargo`, `node` or `flutter`. Need a tool? Add it to `flake.nix`, in the group its shell composes from.
- License is AGPL-3.0-only. Every source file starts with an SPDX header (`// SPDX-License-Identifier: AGPL-3.0-only`, or the file type's comment equivalent).
- Comments explain **why**, not what. Keep them minimal.
- No back-compat shims for hypothetical changes. Pre-1.0, breaking changes land cleanly.
- Only write high quality tests. Do not test trivial, noisy, or common behavior; match the testing conventions of the surrounding code.
- Commit to the current branch in small, logical commits: `area: imperative summary`, then a wrapped prose body explaining why.
- Never commit `target/`, `result*`, `data.db*`, `cache/`, `.cache/`, or `.env*`. Review your diff before committing.
- **Never credit yourself.** AI agents must not add `Co-Authored-By`, "Generated with", or any other AI attribution or self-reference to commits, pull requests, code, or docs.

## Quality gates

CI runs these; run them before committing (details in each subdirectory's `AGENTS.md`):

```sh
nix flake check                                          # repo root: release package + fmt
cd backend && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd web && npm run check
cd flutter && flutter analyze --no-pub && flutter test --no-pub
```
