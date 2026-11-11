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

Common tasks are [`just`](https://just.systems) recipes. `just` lists them all, and `just <group>` lists one group.

## Try the sample library

`just demo up` gives you a working server without touching your own data. It downloads Blender's open movies (about 2 GB, once, and resumable), builds everything, and starts a server at `http://127.0.0.1:28793`. Sign in as `admin` with the password `noflex-demo-pass`. Titles get real metadata from TMDb and TheTVDB.

To test multi-server features, ask for more servers:

```sh
just demo up --servers 3   # on :28793, :28794 and :28795
```

Each server has a share of the films, and Sintel is on all of them. Every server's admin is already linked to the others, so clients see them all after signing in to one.

Clients connect to linked servers directly, so the links have to use an address the client can reach. That matters when the client isn't on this machine:

```sh
just demo up --servers 2 --bind 0.0.0.0 --advertise 10.0.2.2       # for an Android emulator
just demo up --servers 2 --bind 0.0.0.0 --advertise 192.168.1.20   # for a phone on your network
```

The password is public, so only bind beyond this machine on a network you trust.

Running `just demo up` again rebuilds and restarts the servers and keeps their data, unless you change the options, which starts afresh. `just demo` lists the rest: `status`, `logs`, `down`, `reset`, and `clean` to delete the films too. Everything lives under `.cache/`.

## Run the server

To run against your own data:

```sh
just server run
```

The server listens on `http://127.0.0.1:28793`. Open it in a browser: the first visit walks you through creating the admin account, then adding a library.

Configuration is optional for local development; see [`CONFIGURATION.md`](CONFIGURATION.md) for every key.

## Run the web client

The server embeds a built client. To iterate on the client with hot reload, run Vite alongside the server:

```sh
just web install
just web dev     # http://localhost:5173, proxying the API to :28793
```

See [`web/README.md`](../web/README.md) for how builds are embedded.

Semantic search also needs the embedding model; `just server model-fetch <dir>`
downloads it, then point `plugin.assets_dir` at `<dir>`.

## Run the app

The Flutter app lives in `flutter/`. It asks for your server's address on first launch.

```sh
just app setup       # fetches dependencies and generates the GraphQL types
just app run         # the Linux desktop, or `just app run <device>`
```

From an Android emulator the server on your machine is at `10.0.2.2:28793`, and emulators need one extra build flag, which `just app run-emulator` passes; see "Test devices" in [`flutter/AGENTS.md`](../flutter/AGENTS.md).

## Run the TV app

The TV app lives in `solidtv/`, and runs in a browser while you work on it. Arrow keys, Enter and Backspace stand in for the remote.

```sh
just tv install
just tv dev          # http://localhost:5174, pointed at http://127.0.0.1:28793
```

Packaging for Samsung, LG, Android TV, RDK and Apple TV, and what each needs, is in [`PACKAGING.md`](PACKAGING.md#tv-app).

## Install plugins

Metadata providers, login providers, scrobblers, and embeddings are all WASM plugins. Build one and hot-reload it into your running server:

```sh
just plugin build tmdb   # by plugin id or crate name
just plugin dev tmdb     # symlinks into $XDG_CONFIG_HOME/noflex/plugins/
```

Plugins that need an API key read it from `$XDG_CONFIG_HOME/noflex/secrets/<plugin-id>.toml`. TMDb and TheTVDB ship with shared keys, so they don't need one. Writing your own? Read [`PLUGINS.md`](PLUGINS.md).

## Check your work

```sh
just check crate <name>   # format, lint and test one crate, while you work
just check all            # every backend and client gate
just check nix            # nix flake check, which CI also runs
```

The GraphQL schema is code-first. When you change it, the drift test in `noflex-graphql` fails until you regenerate the SDL snapshot the web client validates against; commit it with the change:

```sh
just server snapshot
```
