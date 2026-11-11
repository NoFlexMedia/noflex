# Plugins

Every integration NoFlex has with the outside world — metadata and artwork providers, external login, scrobblers, notifications, embeddings — is a **plugin**: a standalone WebAssembly component (`.wasm`) implementing one or more hook interfaces from the WIT world in [`noflex-plugin-wit`](../backend/crates/noflex-plugin-wit/wit/). The server loads plugins at startup, hot-reloads them on change, and dispatches into them when it needs work done.

First-party plugins get no special treatment: they ship as `.wasm` and cross the same boundary yours do.

## First-party plugins

They live in `backend/plugins/`, outside the main Cargo workspace because they build for `wasm32-wasip2`.

| Path                           | Plugin ID     | Purpose                                                                 |
| ------------------------------ | ------------- | ----------------------------------------------------------------------- |
| `noflex-metadata-tmdb/`        | `tmdb`        | The Movie Database — movies and series metadata and artwork             |
| `noflex-metadata-tvdb/`        | `tvdb`        | TheTVDB — series and movie metadata and artwork                         |
| `noflex-metadata-musicbrainz/` | `musicbrainz` | MusicBrainz — albums, artists, tracks                                   |
| `noflex-auth-oidc/`            | `oidc`        | OpenID Connect login and local-account linking                          |
| `noflex-embeddings/`           | `embeddings`  | Local Model2Vec embeddings that fill the vector search index            |
| `noflex-webhook/`              | `webhook`     | Outbound webhooks for domain events (Gotify, ntfy, Discord, Slack, ...) |
| `noflex-trakt/`                | `trakt`       | Trakt scrobbling, linked per user by device flow                        |

Some have their own README with setup steps (`noflex-embeddings`, `noflex-webhook`).

## Hooks

| Hook            | When it fires                                                    | Returns                                      |
| --------------- | ---------------------------------------------------------------- | -------------------------------------------- |
| `metadata`      | Identifying and refreshing works                                 | `MetadataMatch`es, a full `MetadataPayload`  |
| `image`         | A work's artwork, a credited person's photo, a hand-picked image | `(ImageMeta, stream<u8>)`; `ImageCandidate`s |
| `post-scan`     | After a library scan completes                                   | unit                                         |
| `notifier`      | Library events and plugin lifecycle                              | unit                                         |
| `user-link`     | A user links or unlinks an upstream account (OAuth, device flow) | `LinkChallenge`; per-user secrets            |
| `external-auth` | Interactive login through an external IdP                        | authorization URL; `VerifiedIdentity`        |
| `scrobbler`     | Playback start, progress at stop, mark played                    | unit                                         |
| `embedder`      | Work texts at scan time, queries at search time                  | one vector per text                          |
| `background`    | A manifest `[[tasks]]` schedule or `host-tasks.enqueue`          | `done` / `more(cursor)`                      |

A plugin implements any subset. Every world exports `background`; stub it if you register no tasks. The host checks the manifest's `hooks` against the component's actual exports at load and rejects a plugin claiming a hook it doesn't export. `subtitle` exists in the WIT but isn't served; a manifest declaring it is rejected.

Hooks are `async func`s on the WASI 0.3 native-async ABI: one instance serves many concurrent calls, and binary payloads cross the boundary as `stream<u8>`.

## Write a plugin

1. **Enter the dev shell** (`nix develop`). It provides the `wasm32-wasip2` target, `wasm-tools`, and `wit-bindgen`. Run cargo from `backend/`.

2. **Create a crate** under `backend/plugins/` so `cargo xtask` discovers it. Use the same `wit-bindgen` version and features as the first-party plugins:

   ```toml
   # backend/plugins/my-plugin/Cargo.toml
   [package]
   name = "my-plugin"
   version = "0.1.0"
   edition = "2024"
   license = "MIT"   # any license; see PLUGIN_LICENSING.md
   publish = false

   [lib]
   crate-type = ["cdylib"]

   [dependencies]
   noflex-plugin-sdk = { path = "../../crates/noflex-plugin-sdk" }
   wit-bindgen = { version = "…", default-features = false, features = ["macros", "realloc", "bitflags", "async"] }

   [package.metadata.noflex-plugin]
   plugin_id = "my-plugin"
   display_name = "My Plugin"
   hooks = ["metadata"]
   capabilities = ["http.egress"]
   http_egress_hosts = ["api.example.com"]

   [package.metadata.noflex-plugin.priority]
   metadata = 50
   ```

3. **Implement the hooks:**

   ```rust
   // backend/plugins/my-plugin/src/lib.rs
   #![cfg(target_arch = "wasm32")]

   wit_bindgen::generate!({
       path: "../../crates/noflex-plugin-wit/wit",
       world: "metadata-plugin",
       generate_all,
   });

   use exports::noflex::plugin::{background, metadata};

   struct MyPlugin;

   impl metadata::Guest for MyPlugin {
       fn attribution() -> Option<metadata::ProviderAttribution> {
           None
       }

       fn supports(kind: metadata::WorkKind) -> bool {
           matches!(kind, metadata::WorkKind::Movie)
       }

       async fn search(
           query: metadata::SearchQuery,
       ) -> Result<Vec<metadata::MetadataMatch>, metadata::PluginError> {
           // Call api.example.com through `host-net.request`.
           Ok(vec![])
       }

       async fn fetch(
           id: metadata::ExternalId,
       ) -> Result<metadata::MetadataPayload, metadata::PluginError> {
           Err(metadata::PluginError::NotFound)
       }

       async fn fetch_children(
           _parent: metadata::ExternalId,
           _parent_work: metadata::WorkId,
       ) -> Result<Vec<metadata::MetadataPayload>, metadata::PluginError> {
           Ok(vec![])
       }
   }

   impl background::Guest for MyPlugin {
       async fn run_task(
           task: String,
           _cursor: Option<String>,
           _payload: Option<String>,
       ) -> Result<background::TaskOutcome, background::PluginError> {
           Err(background::PluginError::BadInput(format!("unknown task `{task}`")))
       }
   }

   export!(MyPlugin);
   ```

4. **Build and hot-reload:**

   ```sh
   cargo xtask plugin-build my-plugin   # compiles and embeds the manifest
   cargo xtask plugin-dev my-plugin     # symlinks into $XDG_CONFIG_HOME/noflex/plugins/
   ```

   A running `noflex-server` picks the change up within about half a second.

`noflex-metadata-musicbrainz` is the best end-to-end reference: HTTP through `host-net`, request pacing, and parse tests against recorded fixtures.

## Install locations

NoFlex searches three directories at startup and watches all three. Later tiers shadow earlier ones on `plugin_id` collision.

| Tier   | Path                               | Typical source                 |
| ------ | ---------------------------------- | ------------------------------ |
| System | `/etc/noflex/plugins/`             | NixOS module, distro package   |
| State  | `/var/lib/noflex/plugins/`         | Downloads                      |
| User   | `$XDG_CONFIG_HOME/noflex/plugins/` | Dev installs, home-manager     |

Secrets follow the same tiers under `secrets/`, one flat `<plugin-id>.toml` per plugin. A value an administrator sets from the admin UI wins over every tier; see [Settings](#settings).

`[plugin.overrides.<id>] disabled = true` in the server config keeps a plugin from loading at all, whichever tier it's in: no hook, task, event or account link reaches it (see [`CONFIGURATION.md`](CONFIGURATION.md#pluginoverridesid)).

## Manifest

The manifest is TOML embedded in the `.wasm` as a custom section named `noflex:manifest`. `cargo xtask plugin-build` derives it from `[package.metadata.noflex-plugin]`.

| Field                                      | Required | Notes                                                                                                                                                                                     |
| ------------------------------------------ | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `plugin_id`                                | yes      | Unique, kebab-case, stable across versions                                                                                                                                                |
| `display_name`                             | yes      | Shown in the admin UI                                                                                                                                                                     |
| `version`                                  | auto     | Stamped from the crate version                                                                                                                                                            |
| `noflex_api_version`                       | auto     | Stamped to the host's plugin API; pre-1.0 hosts require an exact match                                                                                                                    |
| `hooks`                                    | yes      | Interfaces you export: `["metadata", "image", ...]`                                                                                                                                       |
| `capabilities`                             | yes      | Any of `http.egress`, `http.private-network`, `secrets.read`, `user-secrets.read`, `user-secrets.write`, `assets.read`, `background`. Unknown or duplicate entries fail the load; using an undeclared import answers `capability-denied`. |
| `http_egress_hosts`                        | optional | Allowlist for `host-net.request`: exact hosts (IP literals without brackets), `*.suffix`, or `"*"`. Requires `http.egress`                                                                |
| `secrets_read`                             | optional | Keys read via `host-secrets.read`. Requires `secrets.read`                                                                                                                                |
| `user_secrets_read` / `user_secrets_write` | optional | Per-user keys for `host-secrets.read-for-user` / `write-for-user`. Require `user-secrets.read` / `user-secrets.write`                                                                     |
| `settings`                                 | optional | `[[settings]]` entries an administrator edits from the admin UI; see [Settings](#settings)                                                                                                |
| `tasks`                                    | optional | `[[tasks]]` entries (`name`, `interval_ms`, optional `payload`) for `background`. Require `background`                                                                                    |
| `events`                                   | optional | `notifier` only: event names to receive (`work-added`, `playback-started`, ...). Unset receives every kind                                                                               |
| `priority.<hook>`                          | optional | Per-hook default priority; server config can override                                                                                                                                     |
| `deadlines_secs.<hook>`                    | optional | Per-hook call deadline in seconds (default 30)                                                                                                                                            |
| `memory_limit_mb`                          | optional | Default 64                                                                                                                                                                                |
| `license`, `author`, `homepage`            | optional | Display only                                                                                                                                                                              |

## Settings

A manifest's `[[settings]]` say what an administrator can configure, and the admin UI's Plugins tab draws a form from them. Each is a top-level key of the plugin's config stanza, or of its secrets:

```toml
secrets_read = ["auth_token"]

[[settings]]
key = "url"
label = "URL"
description = "Every event is posted here as JSON."
kind = "url"
required = true

[[settings]]
key = "auth_token"
label = "Bearer token"
kind = "secret"
```

| Field         | Required | Notes                                                                                                   |
| ------------- | -------- | ------------------------------------------------------------------------------------------------------- |
| `key`         | yes      | Unique within the manifest                                                                              |
| `label`       | yes      | Shown beside the field, in sentence case. Refusals lead with it: "Retries must be a whole number"       |
| `description` | optional | One short line under the field                                                                          |
| `kind`        | yes      | `text`, `url` (absolute `http` or `https`), `secret`, `boolean`, `integer`, or `choice`                 |
| `choices`     | `choice` | The values a `choice` allows; any other kind with `choices` fails the load                              |
| `required`    | optional | Default `false`. The plugin still loads without one, but an administrator can't clear the last source   |
| `default`     | optional | Must fit `kind` (`default = 3` for an integer). Not allowed for a `secret`                              |

- **Values reach the guest unchanged in shape.** A non-secret setting is a key of the document `host-config.read` returns, a TOML boolean or integer for those kinds and a string otherwise. A `secret` is read with `host-secrets.read(key)`, never through config, and its key must be in `secrets_read`, which stays the one list of secrets a plugin reads.
- **Precedence:** a value set from the admin UI, then `config.toml` (or for a secret, the secret files), then `default`. Clearing a value in the admin UI falls back.
- **A change reloads the plugin**, secrets included, so reading config and secrets once at start is fine.
- A secret's value never leaves the server: the admin UI shows only whether one is set, and where from.

## Host imports

Plugins have no sockets, files, or database access of their own. They call back into the host through narrow, capability-gated imports, and every capability use is audit-logged.

- `host-log.log(level, namespace, message, fields)` — structured logs into the server's `tracing` output. Use `Level::Audit` for security-relevant events.
- `host-secrets.read(key)` — an API key declared in `secrets_read`: an administrator's value from the admin UI, else the secret files.
- `host-secrets.read-for-user(user, key)` / `write-for-user(user, key, value)` — per-user tokens, bound to the call they run in: `user` must be the user that call acts for (`user-link` calls act for their user, `scrobbler` calls for the event's user, a `background` run for whoever's call enqueued it, through its `more(...)` chain). Anything else is `capability-denied`, or `unavailable` in a call that acts for no one.
- `host-config.read()` — a TOML document: the plugin's `[plugin.per_plugin_config.<id>]` stanza, with the [settings](#settings) an administrator set over it and manifest defaults for the rest.
- `host-library.lookup-work-by-id(id)` / `lookup-edition-by-id(id)` — read-only library lookups.
- `host-tasks.enqueue(task, payload)` — queue a run of one of your `[[tasks]]`. Requires `background`.
- `host-assets.read-file(name)` — a host-managed asset such as model weights. Requires `assets.read`.
- `host-inference.embed-tokens(model, tokens)` — run an ONNX embedding model from the assets directory natively, on whichever device the operator chose. Requires `inference.run`.
- `host-net.request(req)` — HTTP egress performed by the host. Requires `http.egress`, and the URL's host must match `http_egress_hosts`. Destinations are vetted after DNS, at connect time: loopback, private (RFC 1918 / ULA), link-local, CGNAT, and other special-purpose addresses — including IPv6 forms embedding them (NAT64, 6to4, IPv4-mapped) — are refused unless you also declare `http.private-network` (LAN webhooks, self-hosted IdPs). A name resolving to a mix of public and forbidden addresses is refused outright; proxy environment variables are ignored; redirects are never followed. Requests are audit-logged by origin and a digest of the path and query, never the full URL.
- `host-metrics.histogram(name, value)` / `host-metrics.counter(name, delta)` — metrics.

## Priority and fallback

When several plugins implement the same hook, the host:

1. Calls `search()` on every candidate concurrently and ranks by plugin priority, then match confidence.
2. Calls `fetch()` on **exactly one** provider — the top match's source — and uses its payload as-is. Fields are never merged across providers.
3. Falls back to the next candidate only when `fetch()` fails (unavailable, not found, rate limited, internal error, or timeout). A full call queue (`Overloaded`) isn't a failure — the fetch never ran — so the host retries the same provider briefly and otherwise leaves the work for later.

`image` has its own priority axis, so one plugin can lead for metadata while another leads for artwork. A library's provider order (set per library in the admin UI) and `[plugin.overrides]` in the server config both reorder candidates.

## Artwork and people

- **`image.fetch(id, kind, language)`** answers with the best `kind` image of the record `id` names. The host calls it after applying a payload, for each kind the payload's `images` named (or the work kind's defaults), trying the winning provider first and then the library's other image providers.
- **`list-candidates(id, kind)`** lists every image a provider has, best first, for an administrator picking artwork by hand. Each `ImageCandidate` carries a `handle` the host hands back unchanged to **`fetch-candidate(handle, preview)`**: `preview` asks for a picker-sized rendition, otherwise the original. The handle is the plugin's own; clients only ever see an id the host made for it, valid for half an hour.
- A picked image is locked on the work (`*_IMAGE` fields), so refreshes and backfills leave it alone until it's unlocked.
- **People.** Each `metadata-person` carries `external-ids`, the provider's own ids for them, in the same path style as its work ids (TMDb `person/<id>`, TheTVDB `people/<id>`). After applying a payload, the host asks the plugin that supplied it for the photos of the first 20 people billed who don't have one yet: `image.fetch` with one of those ids and kind `primary`. It runs in the background and never fails the payload; answer `not-found` for someone with no photo. The host keeps every provider's ids on the person's tag.

## Account linking

- **`link-start(user)`** returns a `LinkChallenge`: the `auth-url` the user opens, the `state` the host echoes back, and for a device flow the `user-code` they enter there, which clients show beside the link.
- **`link-complete(user, state, callback-query)`** finishes a redirect flow with the query string the provider sent the host's callback route. A device flow returns an empty list here and polls from a `background` task instead (`host-tasks.enqueue` from `link-start`), handing tokens over with `host-secrets.write-for-user`.
- **`unlink(user)`** abandons a link in progress and, best effort, revokes the grant upstream. The host first stops the user's background chains and waits for any run in flight; their secrets are still readable during the call and deleted once it returns, whatever it returns.
- Anyone signed in can link a plugin that exports `user-link` once its operator secrets are all set.

## Scrobbling

The host turns the domain bus into `scrobble-event`s and enriches each from storage, so a scrobbler needs no library lookups. Users who never linked are skipped.

- `action` is `started` when playback starts, and `stopped` with `position-ms` for the progress reported when it stops; the session's own end isn't scrobbled apart from that. A `stopped` without a position is a title marked played.
- `external-ids` are the work's provider ids, without the scanner's own keys. Match on these, never on titles.
- For an episode, `series-external-ids`, `season-number` and `episode-number` name it by its show, since an episode's provider ids are paths through its series.
- `runtime-ms` is the edition's runtime when known; the plugin derives its own progress percent and watched threshold.

## Attribution

A provider's terms often require crediting it wherever its data shows, so `metadata` plugins answer `attribution()`, or `None` to ask for no credit:

```rust
fn attribution() -> Option<metadata::ProviderAttribution> {
    Some(metadata::ProviderAttribution {
        text: "Metadata provided by Example.".into(), // at most 300 characters
        url: Some("https://example.com".into()),      // http or https
        logo: Some(include_bytes!("../assets/logo.svg").to_vec()),
    })
}
```

- `logo` is an SVG document, or PNG, JPEG, WebP or GIF bytes, at most 256 KiB. The host tells them apart by content. Clients draw it unchanged at small sizes on a dark surface, so ship the variant made for dark backgrounds. A logo or URL the host rejects is dropped with a warning and the text stands alone.
- The host reads it once per load and stores it, so titles stay credited after the plugin is removed.
- A title is credited to the plugin that supplied its fields, then to any plugin whose artwork it is showing. Clients read `Work.attributions`; `providerAttributions` lists every credit by plugin id, for anything naming a provider.
- Logos are served from `/attributions/<plugin-id>/logo` without a credential, sandboxed by a content security policy.
- Check the provider's terms before shipping its logo, and keep a `NOTICE.md` beside it saying where it came from and under what license: it isn't covered by your plugin's license.

## Concurrency, deadlines, and backpressure

- One instance serves every call. Up to 64 calls run at once and 64 more may queue before callers see `Overloaded`.
- Each call has a wall-clock deadline (30s, or the hook's `deadlines_secs`) from when the instance starts it. Time awaiting `host-net` counts, so size deadlines and request `timeout-ms` for slow upstreams.
- A call past its deadline is cut off and the instance is recycled — the component model can't cancel a single task, so in-flight siblings fail too. CPU-heavy work isn't otherwise penalized; the host yields between epoch ticks.
- To sleep without blocking the instance (pacing a rate-limited API), import `wasi:clocks/monotonic-clock@0.3.0` and await `wait-until` / `wait-for`.
- When the host calls `metadata::fetch` and `image::fetch` for the same id back to back, both land on the same instance. `noflex-plugin-sdk` provides `TtlCache` (bounded LRU with expiry) and `SingleFlight` (coalesces concurrent requests for one key) so the upstream is hit once.

## Background tasks

The component model gives a guest no way to run unprompted, so the host drives periodic work — re-indexing, sweeping an upstream, finishing a device-flow link — through the `background` hook.

```toml
[[tasks]]
name = "sweep"
interval_ms = 60_000
# payload = "..."   # optional static payload
```

- **Registration is declarative.** The schedule is read from the manifest before the plugin ever runs, so it's auditable from the artifact and a malformed entry is rejected without starting a guest. There is no `register()` import.
- **Long work chunks.** Return `more(cursor)` and the host reschedules promptly with that cursor rather than waiting for the interval. Background calls get no deadline exception, so chunk anything long.
- **State survives.** Cursors and `host-tasks.enqueue` payloads are persisted host-side across instance recycling and restarts. The enqueue queue is FIFO per task and drains before the scheduled payload.
- **User binding.** A payload enqueued from a user-bound call (`user-link`, `scrobbler`, or a run that is itself bound) runs bound to that user, as does its `more(...)` chain. Each user gets a chain of their own beside the task's unbound one, which its schedule and unbound enqueues drive: a user's payload only continues or replaces their own chain, and every chain that's due runs together. Chains are persisted with the user they're bound to, so a user's chain resumes at once, still bound, after a restart.
- **Supervision.** A trap recycles the instance with exponential backoff; five consecutive traps open the circuit breaker until the plugin is reloaded. Health is on the GraphQL `plugins` query (`instanceRestarts`, `circuitOpen`, and per-task `lastRun`, `consecutiveFailures`, `totalRuns`).

## Trust

Any plugin in a search-path directory loads immediately, on the assumption that an admin put it there, and every capability use is audit-logged. **Don't let untrusted users write to your plugin directories.**

## ABI stability

Plugins build for `wasm32-wasip2` on the component-model async ABI, which is still moving upstream. Until Component Model 1.0 / WASI 1.0 freeze it, the host and plugins are **co-versioned**: a plugin's `noflex_api_version` must match the host exactly, and upstream breaks are absorbed by clean re-bumps of the WIT, host, SDK, and every first-party plugin together — no compatibility shims. Rebuild your plugin against the matching `noflex-plugin-wit` when you upgrade NoFlex.

## Licensing

Plugins are independent works across the WIT boundary and may use any license. See [`PLUGIN_LICENSING.md`](PLUGIN_LICENSING.md).
