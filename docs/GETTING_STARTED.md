# Getting Started

Everything you need to build and run NoFlex from source.

## Dev environment

The Nix flake is the only supported toolchain. It pins Rust (from `backend/rust-toolchain.toml`), nightly `rustfmt`, Node.js, ffmpeg, SQLite, the `wasm32-wasip2` target and WASM tools, the cargo extensions CI uses, and Flutter with an Android SDK.

```sh
nix develop      # or: direnv allow, with the committed .envrc
```

That default shell has everything, which means several gigabytes of Android SDK. If you only work on one side, pick a lighter one:

```sh
nix develop .#no-flutter     # server and web
nix develop .#flutter-only   # just the app
```

Under direnv, put `export NOFLEX_SHELL=no-flutter` in `.envrc.local`.

The shell also routes `rustc` through a shared `sccache`. If you juggle several branches, read [`GIT_WORKTREES.md`](GIT_WORKTREES.md) — it makes builds across worktrees share one cache.

## Run the server

The Cargo workspace lives in `backend/`:

```sh
cd backend
cargo run -p noflex-server
```

The server listens on `http://127.0.0.1:8096`. Open it in a browser: the first visit walks you through creating the admin account, then adding a library.

Configuration is optional for local development; see [`CONFIGURATION.md`](CONFIGURATION.md) for every key.

## Run the web client

The server embeds a built client. To iterate on the client with hot reload, run Vite alongside the server:

```sh
cd web
npm install
npm run dev      # http://localhost:5173, proxying the API to :8096
```

See [`web/README.md`](../web/README.md) for how builds are embedded.

Semantic search also needs the embedding model; `cargo xtask model-fetch <dir>`
downloads it, then point `plugin.assets_dir` at `<dir>`.

## Run the app

The Flutter app lives in `flutter/`. It asks for your server's address on first launch.

```sh
cd flutter
flutter pub get
dart run build_runner build   # generates the GraphQL types
flutter run -d linux          # or an Android device or emulator
```

From an Android emulator the server on your machine is at `10.0.2.2:8096`, and emulators need one extra build flag; see "Test devices" in [`flutter/AGENTS.md`](../flutter/AGENTS.md).

## Install plugins

Metadata providers, login providers, scrobblers, and embeddings are all WASM plugins. Build one and hot-reload it into your running server:

```sh
cd backend
cargo xtask plugin-build tmdb   # by plugin id or crate name
cargo xtask plugin-dev tmdb     # symlinks into $XDG_CONFIG_HOME/noflex/plugins/
```

Plugins that call an API read their keys from `$XDG_CONFIG_HOME/noflex/secrets/<plugin-id>.toml`. Writing your own? Read [`PLUGINS.md`](PLUGINS.md).

## Check your work

CI runs the Rust gates and `nix flake check`; run the client check too:

```sh
cd backend
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # or: cargo nextest run --workspace
cd ../web && npm run check
cd .. && nix flake check
```

The GraphQL schema is code-first. When you change it, the drift test in `noflex-graphql` fails until you regenerate the SDL snapshot the web client validates against; commit it with the change:

```sh
cd backend
NOFLEX_UPDATE_SNAPSHOTS=1 cargo test -p noflex-graphql --test it schema_snapshot
```
