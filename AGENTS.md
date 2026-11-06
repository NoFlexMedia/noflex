# NoFlex

NoFlex is a self-hosted media server: a Rust backend whose only client surface is a GraphQL API, WASM plugins for every provider integration, a SvelteKit web client, and a Flutter app for phones, tablets, foldables and desktops.

## Read every `AGENTS.md` on your path

Each project subdirectory owns an `AGENTS.md` describing its stack, commands, and conventions. **Always read the `AGENTS.md` of every directory you work in** — including nested ones (a crate, a plugin, a component folder) — before making changes. This root file deliberately does not duplicate those details.

- [`backend/AGENTS.md`](backend/AGENTS.md) — Rust workspace, plugins, architecture invariants.
- [`web/AGENTS.md`](web/AGENTS.md) — SvelteKit client.
- [`flutter/AGENTS.md`](flutter/AGENTS.md) — Flutter app: signals, playback, downloads, test devices.

### Writing an `AGENTS.md`

[`backend/AGENTS.md`](backend/AGENTS.md) is the model.

- Bullets over prose: succinct and information-dense, one rule or fact per bullet. A heading gets a sentence or two of lead-in at most.
- Lead with the rule and name the exact symbol, file, or command. Add the reason in a clause when it isn't obvious.
- Record what the code can't tell the reader: invariants, constraints, gotchas, measured results.
- A settled decision gets a line, not its history. Keep background only where it still changes what the reader does.
- Reach for `###` subheadings, tables, and code blocks when they make a section easier to scan.

## Layout

- `backend/` — Cargo workspace: crates in `backend/crates/`, first-party WASM plugins in `backend/plugins/`, `cargo xtask` in `backend/xtask/`.
- `web/` — SvelteKit client, embedded into the server binary at build time.
- `flutter/` — Flutter app for Android, iOS, macOS, Windows and Linux.
- `nix/` — NixOS module. `flake.nix` at the root provides the dev shell, packages, and checks.
- `justfile`, `tasks/`, `scripts/tasks/` — the command interface: one `mod` per group, with anything longer than a few lines in a script.
- `docs/` — operator and contributor docs, linked from `README.md`.

## Domain

- The data model has three levels: a _Work_ has one or more _Editions_ (Theatrical, Director's Cut), each with one or more _MediaFiles_ (versions: multi-disc parts, HDR and SDR encodes). Never conflate these levels.
- The **GraphQL API** is canonical. New features land there first; clients consume nothing else.
- **Tags are the one taxonomy.** Genres, moods, studios, people, ratings, quality badges, edition labels, and user tags share one `tags` table split by `namespace`. Add a namespace, never a taxonomy table.
- **Self-hosted.** No cloud services in core paths; optional plugins may reach the internet.
- **Complete hardware and codec support.** Aim for full hardware acceleration on the server and every client. Prioritize open codecs like AV1: treat them as first-class, never as a fallback.

## Design language

NoFlex has one design language, and every client designs around it rather than around its platform's look. These principles are the bar for new UI; the web client is the reference where the clients differ.

- **As close as is practical.** Clients don't have to match pixel for pixel. Where a platform has an easy, native-ish way to get the effect (a Material ripple, predictive back, iOS swipe), take it rather than bending over backwards to reproduce the web's. Native touches sit on top of our tokens and components; they don't replace them with a stock Material or Cupertino look.
- **One set of tokens.** `web/src/app.css` is canonical: dark only, one accent, Inter, Phosphor, shared radii, easings, and durations. Clients port it (`flutter/lib/theme.dart`) instead of inventing values.
- **One product.** Destinations, terms, and structure carry across clients. Adapt to the room and input available, not to a different idea of the app.
- **Artwork leads.** Chrome stays dark and quiet so posters and backdrops carry the color, as with the page tint sampled from artwork (`--ambient`).
- **Everything answers.** Controls respond the moment they're touched: a press sinks, a hover lifts, an indicator springs into place, a poster carries into its detail page. See `WorkCard` and `.tabs .indicator`.
- **Motion is quick and purposeful.** It shows where something came from or that an input landed, and never makes the viewer wait. Default to `--ease-out`, keep `--ease-spring` for moments of physical response, and honor reduced-motion settings.
- **Loading rarely looks like loading.** Paint from cache first and preload on intent. Delay progress indicators so instant work shows none (`NavProgress`). Hold the content's shape with skeletons, and let artwork resolve in from a placeholder (`Artwork`).
- **Every state is designed.** Empty and failed states say what happened and offer the next step (`State`, `NfState`). User-triggered failures surface as toasts.

## Conventions

- The toolchain comes from the Nix flake — enter it with `nix develop` or `direnv`. Never `rustup`, never a system `cargo`, `node` or `flutter`. Need a tool? Add it to `flake.nix`, in the group its shell composes from.
- `just` is the entry point for common tasks: `just` lists every recipe, `just <group>` one group. A command people run more than once gets a recipe.
- `just demo up [--servers N]` starts throwaway servers with a real library and linked to each other. Use it to see a change working, rather than building a library by hand.
- Shell scripts pass `shfmt --indent 2` and `shellcheck`, justfiles `just --fmt` (`just check tasks`).
- License is AGPL-3.0-only. Every source file starts with an SPDX header (`// SPDX-License-Identifier: AGPL-3.0-only`, or the file type's comment equivalent).
- Comments explain **why**, not what. Keep them minimal.
- No back-compat shims for hypothetical changes. Pre-1.0, breaking changes land cleanly.
- Only write high quality tests. Do not test trivial, noisy, or common behavior; match the testing conventions of the surrounding code.
- Commit to the current branch in small, logical commits: `area: imperative summary`, then a wrapped prose body explaining why.
- Never commit `target/`, `result*`, `data.db*`, `cache/`, `.cache/`, or `.env*`. Review your diff before committing.
- **Never credit yourself.** AI agents must not add `Co-Authored-By`, "Generated with", or any other AI attribution or self-reference to commits, pull requests, code, or docs.

## Quality gates

CI runs these; run them before committing (details in each subdirectory's `AGENTS.md`). `just check` has them as recipes, along with `just check crate <name>` for one crate:

```sh
nix flake check                                          # repo root: release package + fmt
cd backend && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd web && npm run check
cd flutter && flutter analyze --no-pub && flutter test --no-pub
```
