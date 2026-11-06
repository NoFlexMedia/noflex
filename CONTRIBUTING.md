# Contributing

Thanks for your interest in NoFlex! This page covers how changes land. For building and running from source, start with [`docs/GETTING_STARTED.md`](docs/GETTING_STARTED.md); for code conventions, read the `AGENTS.md` in the root and in each directory you touch — they're written for people as much as for tools.

## Workflow

- Work in the Nix dev shell (`nix develop`). It is the only supported toolchain. The default shell has everything, including Flutter and several gigabytes of Android SDK. If you don't touch the app, use `nix develop .#no-flutter`; if you only touch the app, `nix develop .#flutter-only`. With direnv, set `export NOFLEX_SHELL=no-flutter` in `.envrc.local`. CI uses two smaller shells that leave out the emulator and its system images: `flutter-check` for analysis and tests, and `flutter-android` for building APKs.
- Keep commits small and focused: `area: imperative summary` (`scanner: keep year-like numbers in movie titles`), then a body explaining **why**.
- Before opening a pull request, run the gates CI runs:

  ```sh
  just check all
  just check nix
  ```

- Schema changes include the regenerated GraphQL snapshot; see [`docs/GETTING_STARTED.md`](docs/GETTING_STARTED.md#check-your-work).
- Architecture changes — anything that bends an invariant in [`backend/AGENTS.md`](backend/AGENTS.md) — need a discussion in the pull request description first.

## Storage backends

A storage backend is a crate implementing the `Storage` trait from `noflex-storage`. Postgres or MySQL support is a contribution seam, not a fork. A backend is accepted once it passes the shared conformance suite:

```rust
// backend/crates/noflex-storage-<engine>/tests/conformance.rs
#[tokio::test]
async fn conformance() {
    let store = MyStore::open_temp().await.unwrap();
    noflex_storage::test_suite::run_all(&store).await;
}
```

The full recipe is in [`backend/AGENTS.md`](backend/AGENTS.md#adding-a-storage-backend).

## Plugin API

The plugin ABI is the WIT world in `backend/crates/noflex-plugin-wit/`; each provider trait in `noflex-core/src/providers/` mirrors one hook.

- **Before 1.0**, changes to the WIT, manifest schema, and traits land cleanly as one co-versioned change: the WIT, host runtime, SDK, every first-party plugin, and the test fixture guests move together, and the API version is bumped. No compatibility shims.
- **From 1.0**, the trait surface is semver-stable. New methods get default implementations; a changed signature ships as a new trait (`MetadataProvider2`), and the old one is deprecated and removed after two minor versions.

User-visible plugin API changes update [`docs/PLUGINS.md`](docs/PLUGINS.md).

## Licensing

NoFlex is [AGPL-3.0-only](LICENSE). By contributing, you agree your contribution is licensed the same way. Every source file carries an SPDX header:

```rust
// SPDX-License-Identifier: AGPL-3.0-only
```

Plugins are independent works and may use any license; see [`docs/PLUGIN_LICENSING.md`](docs/PLUGIN_LICENSING.md).
