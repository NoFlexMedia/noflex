# Website

`site/` is noflex.dev: the marketing pages, the pages the app stores ask for (privacy, terms, support, account deletion), and the home of the Cast receiver. SvelteKit with every page prerendered, so the build is plain files for any static host.

## Commands

Run inside `nix develop` from the repo root:

```bash
just site install
just site dev       # http://localhost:5175
just check site     # svelte-check, then a build: the CI gate
just site build     # site/build, with cast/receiver/ copied to /cast/receiver/
just site deploy    # build, then `wrangler deploy` to Cloudflare; needs `nix develop .#site`
```

- **The build is the link checker.** Prerendering crawls every internal link and `#anchor`, and fails on a 404 or a missing id. Add the page or the id before linking to it.
- `nix build .#site` builds the same output, and `nix flake check` builds it too. A change to `package-lock.json` needs a new `npmDepsHash` in `flake.nix`.

## Layout

- `src/routes/` — one page each. `+page.svelte` is the home page; its sections are `src/lib/components/home/`.
- `src/lib/site.ts` — links (GitHub is the only contact), `DEMO_URL`, and the demo library's titles and tints. Link through `links`, never a hard-coded GitHub URL.
- `src/lib/actions.ts` — `tilt` (as `web/`'s), `reveal`, `scrollProgress`, and `trackPress`, which starts a `.btn`'s press highlight where the pointer went down.
- `src/lib/components/Doc.svelte` — the legal and help pages' layout: title block, sticky contents, `.prose`.
- `wrangler.toml` — a Worker of static assets alone, on the `noflex.dev` custom domain, serving `404.html` for a missing path. wrangler lives only in the `site` shell, as its closure is over 2 GiB.
- `static/_headers` — Cloudflare's response headers. `static/.well-known/security.txt` expires each year: renew `Expires`.
- `scripts/cast-receiver.js` copies `cast/receiver/` into the build after SvelteKit writes it, so the receiver is served as is (see [`cast/AGENTS.md`](../cast/AGENTS.md)).

## Design

- **Tokens are ported from `web/src/app.css`**, which is canonical, into `src/app.css`, with the same shared classes (`btn`, `chip`, `tabs`, `panel`). Change a value there first. Site-only classes are layout (`wrap`, `section`, `section-head`), `[data-reveal]` and `.prose`.
- **The site may move more than the app**: scroll-driven 3D (`PosterWall`, `Devices`), and demos that loop once they scroll into view. Each one still honors reduced motion, and every loop clears its timer when its component goes.
- **Prerendered HTML must read without JS.** `reveal` only hides what's below the fold once the page has hydrated, and a demo's markup shows a finished state before its loop starts.
- **The CSP allows style attributes** (`style-src-attr 'unsafe-inline'`), unlike `web/`'s: prerendering writes `style:` directives as attributes. Scripts and stylesheets stay `'self'`.
- Phosphor for icons, with `fill` for its brand logos. Simple Icons, through `SimpleIcon`, for the marks Phosphor lacks (the tech stack).

## Content

- **Every claim is one the code backs.** Feature copy, platform status and the privacy policy were checked against the source; when behavior changes, change the page. Platform status follows `solidtv/AGENTS.md` and `docs/PACKAGING.md`.
- **The privacy policy describes what leaves a device or a server.** Adding an outbound call, an SDK, a permission, or stored personal data means updating `/privacy` and, if it's about deletion, `/delete-account`.
- **Artwork is Blender Studio's open movies**, credited per poster on `/credits`. Tears of Steel's footage may not be used to advertise, so it shows only its poster: `stills` in `site.ts` leaves it out. Screenshots come from `just demo up`. Never publish one showing a real host's name or paths, as the admin status page does.
- Semantic search examples are real results from the demo's index, with their similarities.
