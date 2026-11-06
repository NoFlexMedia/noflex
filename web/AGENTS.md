# Web Client

`web/` is the SvelteKit client (Svelte 5 runes, static adapter with an `index.html` fallback), talking to the server only through GraphQL via [Houdini](https://houdinigraphql.com). The server embeds `build/` at compile time and serves it same-origin.

## Commands

Run inside `nix develop` from `web/`:

```bash
npm install
npm run dev     # Vite dev server; proxies the API to http://127.0.0.1:8096
npm run check   # svelte-kit sync + svelte-check — CI gate
npm run build   # writes build/, which the next noflex-server build embeds
```

## Layout

- `src/routes/` — one route per page. `+page.ts` loaders start the page's query so hover-preloaded links paint from the cache.
- `src/components/` — shell, cards, rails, player, and one folder per feature (`admin/`, `account/`, `detail/`, `livetv/`).
- `src/components/ui/` — bits-ui primitives (dialog, menu, select, tabs, tooltip, segmented) wrapped in the app's styles. Reach for these before writing a new control.
- `src/lib/` — session, playback, and UI state, plus helpers: `ui.svelte.ts` (`toast`, `attempt`, `confirmAction`, `poll`), `graphql.ts` (store helpers), `format.ts` (display formatting).
- `src/lib/graphql/*.gql` — shared fragments and documents used by loaders and `lib/`. Component-only documents sit inline in `graphql(...)`.
- `$codecs` — `lib/ts/codecSupport.ts`, shared with `solidtv/`. See [`lib/AGENTS.md`](../lib/AGENTS.md).
- `src/client.ts` — the Houdini client: auth headers, one refresh-and-retry, transport errors as GraphQL errors. `src/lib/live.ts` carries subscriptions over `graphql-ws`.
- `src/app.css` — design tokens (`--bg`, `--text-2`, `--accent`, `--r`, ...) and shared classes (`btn`, `field`, `chip`, ...).

## Conventions

- **Import through aliases.** Anything outside the current feature folder comes from `$components/...` or `$lib/...`, never `../../`. Relative imports are only for files within one feature folder (`admin/` ↔ `admin/status/`).
- Components are Svelte 5 with runes (`$state`, `$derived`, `$effect`, `$props`) and plain-JS `<script>` blocks; `src/lib/` modules are TypeScript.
- Every file starts with an SPDX header — `<!-- SPDX-License-Identifier: AGPL-3.0-only -->` in `.svelte`.
- GraphQL documents are validated against the server's committed SDL snapshot (`backend/crates/noflex-graphql/tests/snapshots/schema.graphql`). A backend schema change regenerates that snapshot first; then update documents here.
- Home lists 24 `WorkFull`s, so each field there costs 24 times over. What only the title page draws goes on `WorkDetail` instead, as `attributions` does.
- `Artwork` handles a title still indexing, and asks `lib/indexing.svelte.ts` to watch it. One `worksById` covers the screen, and the answer goes through the normalized cache, so nothing else has to refetch. A linked server's title isn't followed.
- Wrap user-triggered mutations in `attempt(...)` so failures surface as toasts and successes can confirm with one.
- **Removals are declarative.** Select `id @Session_delete` on what the mutation returns, or `...Preferences_remove` for a type keyed on something else, and tag the query's field with `@list`; a connection needs `edges { node }` for that. Refetch only where a row leaving mid-action has side effects: `leaveWatchRoom` would otherwise re-trigger the page's auto-join.
- A list returned by a mutation doesn't update a query: Houdini stores it under the mutation's own key. Only records it can key update, and `cache.get(...).delete()` repaints no list without `@list` (`Session` rows stayed on screen until a reload).
- Icons are [Phosphor](https://phosphoricons.com), imported per icon from `phosphor-svelte/lib/<Name>` (deep imports keep dev builds fast) and rendered directly — no wrapper component, no hand-drawn icons. The root `IconContext` defaults them to 20px and `aria-hidden`; pass `weight` for emphasis (`"fill"` for transport controls and ratings) and an `aria-label` plus `aria-hidden="false"` when an icon carries meaning alone. Import the `*Icon` alias when a name shadows a global (`ImageIcon`). Data that picks an icon holds the component, not a name string.
- Device brands Phosphor lacks are [Simple Icons](https://simple-icons.org), through `DeviceIcon` and nowhere else; see the design language in the root `AGENTS.md`. It has no per-icon entry points, so its named imports are what tree-shaking keeps.
- Hand-drawn SVG is for the particular non-icon graphics an icon set can't provide: the logo (`BrandMark`), data drawings (`admin/status/Spark`), and progress rings (the up-next countdown in `Player`).
- Style with the tokens in `app.css`; don't hard-code colors or radii.
- The CSP is strict (`style-src 'self'`, no `data:`). Never write static `style="…"` attributes — use `style:prop={…}` directives, which Svelte applies through the CSSOM.
- Auth and refresh tokens live in `sessionStorage`, never `localStorage`, and are sent only to same-origin routes. Linked servers' keys stay in memory and go only to the server that issued them.
- API responses use paths relative to the server that sent them. Pass any URL you render through `mediaUrl` (`$lib/tokens.js`), so it works while a linked server is active.
- A smart transcode mark is a hard signal to prepare a title next (`markForSmartTranscoding`). "Mark for smart transcoding" is for every device with it on, "Smart transcode for this device" for this browser alone; the queue is Account → Smart transcoding (`SmartTranscodingTab`). An offline copy is different: the original repackaged as one file to save.
- A dropdown's `SubContent` goes in its own `DropdownMenu.Portal`, or the parent menu's `overflow` clips it to nothing.
- Layouts must work at phone width; check the mobile nav and sheets when you change a page.
