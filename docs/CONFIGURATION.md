# Configuration

NoFlex loads its configuration via [figment](https://docs.rs/figment) from
three layers, in order of decreasing precedence:

1. **`NOFLEX_*` environment variables** — highest precedence. Useful for
   container deployments and `systemd` units.
2. **User-tier TOML** — `$XDG_CONFIG_HOME/noflex/config.toml`, falling
   back to `~/.config/noflex/config.toml`. This is where a typical
   installation puts its config.
3. **System-tier TOML** — `/etc/noflex/config.toml`. Read-only; the NixOS
   module writes it.

Env vars override the TOML files. Each scalar key has exactly one
variable, listed under **Env** in the reference below: `NOFLEX_` followed by
the key path upper-cased, with `.` replaced by `_`. The mapping is an explicit
table (because key names themselves contain `_`), so unlisted `NOFLEX_*`
variables are ignored. Arrays and maps (`transcoder.ladder`,
`plugin.search_path`, `plugin.overrides`, `plugin.per_plugin_config`) can only
be set in TOML.

```sh
NOFLEX_BIND=0.0.0.0:9999                    # -> bind
NOFLEX_LOG_LEVEL=debug                     # -> log.level
NOFLEX_TRANSCODER_HWACCEL=vaapi            # -> transcoder.hwaccel
NOFLEX_LIVE_TV_RECORDING_CONCURRENCY=4     # -> live_tv.recording_concurrency
NOFLEX_EXTERNAL_AUTH_AUTO_PROVISION=true   # -> external_auth.auto_provision
```

## Full key reference

### `bind`
- **Type:** `string` (host:port)
- **Default:** `127.0.0.1:8096`
- **Env:** `NOFLEX_BIND`

The address the HTTP server binds to. Use `0.0.0.0` to listen on all
interfaces (do this in containers / behind a reverse proxy).

### `public_url`
- **Type:** absolute HTTP(S) URL (optional)
- **Default:** unset
- **Env:** `NOFLEX_PUBLIC_URL`

The externally reachable base URL used to construct external-auth callback
URIs, for example `https://media.example.com`. External login and identity
linking are unavailable until this is set. NoFlex does not infer it from
`Host` or forwarded headers.

The URL must not contain userinfo, a path, a query, or a fragment: NoFlex
(including its web app) must be served at the root of its origin, so
`https://media.example.com/noflex` is rejected. HTTPS is required except for
`localhost`, `127.0.0.1`, and `[::1]`, where HTTP is allowed for local
development. A trailing slash is ignored. Provider callback URIs have the form
`${public_url}/auth/<plugin-id>/callback`.

### `external_auth.auto_provision`
- **Type:** boolean
- **Default:** `false`
- **Env:** `NOFLEX_EXTERNAL_AUTH_AUTO_PROVISION`

Whether an external login whose verified identity is not yet linked to any
NoFlex account creates a new (non-admin) account. When `false`, only
identities that an existing user has linked from their account settings can
log in; anyone else gets "no NoFlex account is linked to this identity; ask an
admin". Enable this only when every identity your provider will verify should
get access, since any of them can then create an account.

### `cross_origin`
- **Type:** boolean
- **Default:** `true`
- **Env:** `NOFLEX_CROSS_ORIGIN`

Whether to answer cross-origin requests, which lets web clients signed in to another NoFlex server connect here as a linked server. Requests still need this server's own credentials. Stays off until the first account exists, whatever this says, so a website can't reach the setup wizard through a visitor's browser. Native clients don't need it.

### `data_dir`
- **Type:** path
- **Default:** XDG data dir (`~/.local/share/noflex` on Linux)
- **Env:** `NOFLEX_DATA_DIR`

The directory the SQLite database (`noflex.db`) lives in. The plugin
secrets store and the scan event log also live here.

### `cache_dir`
- **Type:** path
- **Default:** XDG cache dir (`~/.cache/noflex` on Linux)
- **Env:** `NOFLEX_CACHE_DIR`

Transcode cache, image cache, and the embed model cache. Sized for
heavy IO; not backed up.

### `cache.transcode_dir`, `cache.image_dir`, `cache.embed_dir`
- **Type:** path
- **Default:** `transcode/`, `image/`, `embed/` under `cache_dir`
- **Env:** `NOFLEX_CACHE_TRANSCODE_DIR`, `NOFLEX_CACHE_IMAGE_DIR`, `NOFLEX_CACHE_EMBED_DIR`

Move one cache onto a different volume, e.g. transcodes onto fast scratch
storage. The image cache holds downloaded artwork and generated thumbnails.

### `backup.dir`
- **Type:** path
- **Default:** empty (backups disabled)
- **Env:** `NOFLEX_BACKUP_DIR`

Directory for timestamped database backups (`backup-<RFC3339>.sql`, a
complete SQLite dump). Put it on a separate volume.

### `backup.interval_hours`
- **Type:** integer
- **Default:** `0`
- **Env:** `NOFLEX_BACKUP_INTERVAL_HOURS`

Hours between scheduled backups. A backup also runs at startup; `0` runs only
that one.

### `backup.keep`
- **Type:** integer
- **Default:** `7`
- **Env:** `NOFLEX_BACKUP_KEEP`

How many backups to keep. Older ones are pruned at the start of each run.

### `log.level`
- **Type:** `string` (RUST_LOG filter syntax)
- **Default:** `info`
- **Env:** `NOFLEX_LOG_LEVEL`

Standard `tracing` filter. `info,noflex=debug` keeps the third-party
crate noise down while debugging NoFlex itself.

### `log.file`
- **Type:** path (optional)
- **Default:** stderr
- **Env:** `NOFLEX_LOG_FILE`

When set, log lines go to this file instead of stderr. The file is not
rotated; use logrotate or journald for that. The parent directory must exist.

### `transcoder.hwaccel`
- **Type:** enum: `none | auto | vaapi | videotoolbox | mf | nvenc | qsv`
- **Default:** `auto`
- **Env:** `NOFLEX_TRANSCODER_HWACCEL`

Hardware acceleration preference. `auto` probes the host at startup and
picks the best available.

### `transcoder.fallback`
- **Type:** enum: `auto | software`
- **Default:** `auto`
- **Env:** `NOFLEX_TRANSCODER_FALLBACK`

What to do when the chosen `hwaccel` is unavailable. `auto` falls back
to software encoding; `software` is a hard error.

### `transcoder.ladder`
- **Type:** array of `{max_pixels, video_codec, video_profile, audio_codec, container, bitrate_kbps}`
- **Default:** AV1 Main and HEVC Main at ~4K, ~1440p and ~1080p; H.264 High at those plus ~720p and ~480p
- **Env:** not supported (use TOML)

What a re-encode can produce. A rung is a video profile and the largest picture it carries, in luma samples (`8294400` is 3840×2160, and also holds a wider 4K picture that's less tall), at a bitrate. The smallest rung of the encode's codec that holds the picture is used, and its bitrate scales down with the picture's area. `video_profile` is a ceiling (`h264_high`, `h264_main`, `h264_baseline`, `hevc_main`, `av1_main`), since encodes are 8-bit: clients that decode less get their own best.

Live transcodes prefer codecs your hardware encoder has proven it can do at startup, in the order the ladder lists them, then H.264. Software HEVC and AV1 can't reliably keep up with playback, so they're only used for clients that can't decode H.264.

### `transcoder.tonemap_curve`
- **Type:** `hable` | `mobius` | `reinhard` | `spline` | `bt2390`
- **Default:** `hable`
- **Env:** `NOFLEX_TRANSCODER_TONEMAP_CURVE`

The curve HDR is mapped to SDR with when a client can't present the HDR itself. Administrators can override it under Administration → Transcoding. Tone-mapping runs on the GPU when ffmpeg has libplacebo and a working Vulkan driver, except with VAAPI and Quick Sync, and on the CPU otherwise. `spline` and `bt2390` only exist on the GPU, so the CPU uses `hable` for them.

### `transcoder.optimize`
- **Type:** table
- **Env:** `NOFLEX_TRANSCODER_OPTIMIZE_<KEY>`, except `encode_video` (use TOML)

Smart transcoding: preparing versions in the background for titles users are likely to watch next, for devices that opt in from their account settings. Viewers can also mark a title, for those devices or just the one they're on; marks go ahead of every guess, first come first served, and are never evicted while they stand. `dir`, `max_storage_gb` and `max_version_gb` are defaults: administrators can override them under Administration → Transcoding.

| Key | Default | |
|---|---|---|
| `enabled` | `true` | Nothing is prepared until a device opts in. |
| `dir` | `optimized/` under `data_dir` | Where finished versions live. They take real disk, so don't point this at a cache volume. Changing it leaves existing versions where they are. |
| `max_storage_gb` | `200` | Versions nobody is likely to watch are evicted past this, least recently used first, and no new job starts while over it. A marked title waiting for space evicts guesses too. |
| `max_version_gb` | unset | Titles whose version is estimated larger than this aren't prepared. |
| `max_cpu_percent` | `40` | Jobs start only below this CPU use, marked titles included. Jobs also wait for playback to end, and running jobs pause while anyone is watching. |
| `plan_interval_secs` | `900` | How often to work out what's likely to be watched next. |
| `lookahead_episodes` | `3` | Episodes to prepare beyond each next episode. |
| `recommendations` | `5` | Top recommendations to prepare. |
| `encode_video` | `["av1", "h265", "h264"]` | Codecs to encode to, preferred first. Each device gets the first it decodes. Ones ffmpeg or the GPU can't encode are skipped. |

### `plugin.search_path`
- **Type:** array of paths
- **Default:** `["/etc/noflex/plugins", "/var/lib/noflex/plugins", "<XDG_CONFIG_HOME>/noflex/plugins"]`
- **Env:** not supported (use TOML)

Directories searched in order for `.wasm` plugin components. Later
tiers shadow earlier ones.

### `plugin.assets_dir`
- **Type:** path (optional)
- **Default:** unset
- **Env:** `NOFLEX_PLUGIN_ASSETS_DIR`

Root directory served to plugins through the `host-assets.read-file` import
(embedding model weights, tokenizers, ...). When unset, the import reports
`unavailable`.

### `plugin.overrides.<id>`
- **Type:** table `{ disabled: bool, priorities: map of hook_name -> work_kind_name -> priority }`
- **Default:** empty
- **Env:** not supported (use TOML)

Per-plugin overrides. `disabled = true` keeps the plugin from loading.
`priorities` overrides the priorities declared in the plugin's manifest; this
layer wins. A priority of `0` excludes the plugin for that work kind. Both
fields are optional.

```toml
[plugin.overrides.tmdb]
disabled = false
priorities.metadata.movie = 200
```

### `plugin.per_plugin_config.<id>`
- **Type:** `toml::Value` (free-form)
- **Default:** empty
- **Env:** not supported (use TOML)

Per-plugin TOML blob passed to the plugin via `host_config::read()`. Use
this for plugin-specific settings (e.g. TMDb API base URL).

For the first-party OIDC plugin, register
`${public_url}/auth/oidc/callback` with the identity provider and configure:

```toml
[plugin.per_plugin_config.oidc]
issuer = "https://identity.example.com"
client_id = "noflex"
scopes = "openid profile email" # optional; this is the default; must include `openid`
# optional: "client_secret_basic" or "client_secret_post". Unset picks from the
# provider's advertised methods, preferring basic (the OIDC default).
token_endpoint_auth_method = "client_secret_basic"
```

Identity providers on a LAN or loopback address (Authentik, Keycloak,
Authelia) work out of the box: the plugin declares `http.private-network`.
Logins only reach existing accounts unless `external_auth.auto_provision` is
enabled.

Store the client secret in a flat `oidc.toml` file at one of the plugin
secret tiers (later tiers override earlier ones): `/etc/noflex/secrets/`,
`/var/lib/noflex/secrets/`, or `$XDG_CONFIG_HOME/noflex/secrets/`.

```toml
client_secret = "replace-me"
```

The issuer and discovered endpoints must use HTTPS, except for loopback HTTP
in development. NoFlex matches accounts only by the verified issuer and
subject; it never links an external identity by email address or username.

The first-party TMDb plugin works out of the box with NoFlex's shared API key,
which TMDb's terms allow for non-commercial use only. Commercial use needs your
own key and a license from TMDb. To use your own, take the v4 read access token
or the v3 key from <https://www.themoviedb.org/settings/api> and store it the
same way, as `tmdb.toml` in a secret tier:

```toml
api_key = "replace-me"
```

The first-party TheTVDB plugin also works out of the box with a shared key. To
use your own from <https://thetvdb.com/api-information>, store it as
`tvdb.toml`. `pin` is your subscriber PIN, only for a user-supported key:

```toml
api_key = "replace-me"
# pin = "replace-me"
```

### Live TV source URLs

Live TV M3U and XMLTV URLs are administrator-managed trusted destinations. They
must use HTTP or HTTPS, but may resolve to LAN tuners, private addresses, or
other internal hosts. NoFlex uses normal reqwest DNS resolution and does not
block private address ranges. Redirects are disabled, response bodies are
capped, and requests have finite connect and overall timeouts. Only configure
sources you trust the server to contact.

### `search.device`
- **Type:** enum: `cpu | gpu | npu | auto`
- **Default:** `cpu`
- **Env:** `NOFLEX_SEARCH_DEVICE`

Where the local embedding model runs. `gpu` tries CUDA, DirectML and CoreML; `npu`
tries OpenVINO's NPU and Qualcomm's QNN; `auto` tries NPU, then GPU, then CPU. Each
needs an ONNX Runtime built for it (see `search.onnxruntime_library`). Without one,
or without the hardware, the model runs on the CPU, logs a warning, and the
Indexing tab shows where it really is.

### `search.onnxruntime_library`
- **Type:** path (optional)
- **Default:** unset
- **Env:** `NOFLEX_SEARCH_ONNXRUNTIME_LIBRARY`

The ONNX Runtime shared library to load, such as a CUDA or OpenVINO build. Unset
uses `ORT_DYLIB_PATH`, then a release bundle's, then the system's.

### `search.threads`
- **Type:** integer
- **Default:** `0`
- **Env:** `NOFLEX_SEARCH_THREADS`

CPU threads per inference run. `0` uses half the physical cores.

### `search.batch_size`
- **Type:** integer
- **Default:** `8`
- **Env:** `NOFLEX_SEARCH_BATCH_SIZE`

Works embedded at a time, up to 64. A batch can't be interrupted, so this also
bounds how long a search or transcode waits for the CPU back.

### `search.busy_percent`
- **Type:** number
- **Default:** `30`
- **Env:** `NOFLEX_SEARCH_BUSY_PERCENT`

Indexing starts a batch only while everything else, ffmpeg included, uses less than
this share of the host's CPU. A host that stays busy still gets one batch every ten
minutes. An embeddings API (set from the Indexing tab) doesn't wait, since it isn't
using this host's CPU.

### `live_tv.recording_concurrency`
- **Type:** integer
- **Default:** `2`
- **Env:** `NOFLEX_LIVE_TV_RECORDING_CONCURRENCY`

Maximum number of recording ffmpeg child processes that may run at once.
Additional due recordings wait for a slot and use the remaining effective
recording window when they start.

### `scanner.debounce_ms`
- **Type:** integer
- **Default:** `500`
- **Env:** `NOFLEX_SCANNER_DEBOUNCE_MS`

How long the filesystem watcher waits after the last notify event
before triggering a rescan. Lower = snappier, higher = fewer rescans.

### `scanner.concurrency`
- **Type:** integer
- **Default:** `4`
- **Env:** `NOFLEX_SCANNER_CONCURRENCY`

How many `walk_videos` passes run in parallel during a full library
scan. Set to `1` on slow disks.
