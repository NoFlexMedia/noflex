<!-- SPDX-License-Identifier: AGPL-3.0-only -->
# NoFlex Web

The SvelteKit client for NoFlex. It's a static single-page app that the server embeds and serves from its own origin, so deep links work and there is no separate API host to expose.

## Development

From `web/`, inside `nix develop`:

```sh
npm install
npm run dev     # http://localhost:5173, proxying the API to http://127.0.0.1:8096
npm run check
npm run build
```

Point the proxy elsewhere in `vite.config.ts` if your backend isn't on the default port.

`npm run build` writes `build/`, which the next Rust build of `noflex-server` embeds. If the server was first built before `build/` existed, Cargo isn't watching it yet: touch `web/package.json` (or set `NOFLEX_WEB_DIR`) once to re-embed. Without a build, the server serves a development placeholder. Nix builds and embeds the client automatically.

Working on the client? Read [`AGENTS.md`](AGENTS.md) for its layout and conventions.
