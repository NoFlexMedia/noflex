# Deployment

This guide assumes NixOS: the flake's module (`nix/module.nix`) covers
the common production knobs. Below is the quick path; for every option,
read the module source. Server settings are documented in
[`CONFIGURATION.md`](CONFIGURATION.md).

## Minimal NixOS host

```nix
{
  services.noflex = {
    enable = true;
    dataDir = "/var/lib/noflex";
    cacheDir = "/var/cache/noflex";
    bind = "0.0.0.0:8096";
    openFirewall = true;
    mediaPaths = [ "/media/movies" "/media/shows" ];
    configFile = "/etc/noflex/config.toml";
  };

  # Let the host's media group read the media.
  users.users.noflex.extraGroups = [ "media" ];
}
```

The systemd unit is hardened: `ProtectSystem=strict`, `PrivateTmp`,
`NoNewPrivileges`, `ReadWritePaths` limited to `dataDir` and
`cacheDir`, and `ReadOnlyPaths` covering `mediaPaths` (so the
process can read but not modify your media library).

## Adding a plugin

```sh
# Build the first-party TMDb plugin
nix develop --command sh -c 'cd backend && cargo xtask plugin-build tmdb'
# Copy it into the state tier
install -Dm644 backend/plugins/noflex-metadata-tmdb/target/wasm32-wasip2/release/noflex_metadata_tmdb.wasm \
  /var/lib/noflex/plugins/tmdb.wasm
```

The plugin host scans three tiers, higher-shadowing-lower:
`/etc/noflex/plugins` (system), `/var/lib/noflex/plugins` (state),
`$XDG_CONFIG_HOME/noflex/plugins` (user). Each tier is overridable
in the NixOS module via `pluginPath` (a `:`-separated env var). Plugin
API keys go in `/var/lib/noflex/secrets/<plugin-id>.toml`. See
[`PLUGINS.md`](PLUGINS.md).

## Backups

Set `backupDir` to a path on a separate volume. The scheduler
runs once at startup and then every `backupIntervalHours`, keeping
`backupKeep` old files. Files are named `backup-<RFC3339>.sql` and
are a complete SQLite dump — restore by loading it into an empty
Turso database.

Backups include each user's keys for the other servers they've linked, and those keys aren't encrypted in the database, so store backups as you would passwords. Optimized versions in `data_dir/optimized/` can be left out: the server prepares them again as needed.

## Deleted accounts

Anyone can delete their own account, from Account in the web client or You in the apps, and an administrator can delete anyone else's.

- The account goes at once with everything that's theirs: sessions and API keys, watch history, devices, playlists, invites, linked services and servers, picture. Their entries in the activity log, their offline copies and their recordings are cleared in the background, at the next start if the server stops first.
- Backups and log files made before keep what they held until they're pruned or rotated. Nothing rewrites them.
- **The last administrator deleting their account erases the server.** The apps and the web warn twice before it happens. Everyone is signed out, and the server restarts itself to empty its database, the transcode, image and embed caches, `data_dir/downloads/` and `data_dir/recordings/`, and the optimized versions it made, then waits at first-run setup. Media files, the configuration file and backups stay. An erasure interrupted by a stop is finished at the next start, before anything else runs. Point the cache directories only at folders that hold nothing else: they're emptied whole.

## Hardware transcoding

The `hwaccel` option takes `none` (software only), `auto`
(recommended; picks the first available backend), `vaapi`,
`videotoolbox`, `nvenc`, `qsv`, `amf`, or `mf`. The `auto` probe runs at
startup and is logged. AMD cards use `amf` on Windows and `vaapi` on Linux.
A release bundle carries its own ffmpeg; otherwise
it must be on the host's PATH, or named by `FFMPEG_BIN` and `FFPROBE_BIN`.
See [`PACKAGING.md`](PACKAGING.md) for what each accelerator needs on the host.

## Remote access

There is no relay — NoFlex is self-hosted end to end, and media
bytes never transit a third-party service. Pick
one of the supported paths:

### Mesh VPN (recommended for private access)

Tailscale or WireGuard puts every client on the same private
network as the server; nothing is exposed publicly and TLS is
the VPN's problem. Bind `bind` to the tailnet interface address
(or firewall the port to it) and you're done — TV clients with a
Tailscale app work unchanged.

### Reverse proxy + TLS (public exposure)

NoFlex doesn't terminate TLS itself; a standard nginx/Caddy/HAProxy
sits in front:

```nginx
server {
  listen 443 ssl;
  server_name media.example.com;
  ssl_certificate /var/lib/acme/cert.pem;
  ssl_certificate_key /var/lib/acme/key.pem;

  # Streaming means large, long-lived responses — don't buffer
  # or cap them.
  client_max_body_size 0;
  proxy_buffering off;
  proxy_read_timeout 1h;

  location / {
    proxy_pass http://127.0.0.1:8096;
  }

  # GraphQL subscriptions (watch rooms, live admin gauges) need the
  # WebSocket upgrade forwarded.
  location /graphql/ws {
    proxy_pass http://127.0.0.1:8096;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
  }
}
```

Caddy's `reverse_proxy` forwards WebSocket upgrades on its own.

Set the canonical callback base in `/etc/noflex/config.toml`; forwarded host
headers are deliberately not used for authentication redirects:

```toml
public_url = "https://media.example.com"
```

External login and identity linking remain unavailable if `public_url` is
missing or invalid. Register
`https://media.example.com/auth/<plugin-id>/callback` with the identity
provider. Use HTTPS in production; plain HTTP is accepted only for loopback
hosts during local development.

Notes:

- Everything rides one port: `/graphql` for the API, `/graphql/ws`
  for subscriptions (`graphql-transport-ws`), `/stream/*`, `/images/*`,
  `/trickplay/*` and `/live-tv/channels/*/logo` for media bytes.
- Media players that can't set headers authenticate byte requests
  with a `?token=` / `?api_key=` query parameter, so the proxy must
  forward query strings (the default) — and will see credentials in
  its access logs unless you filter them.
- If you expose the port publicly, rate-limit `/graphql` at the
  proxy; the `login` mutation is the password-guessing surface.

### Raw port forward

Works, but only behind TLS you terminate elsewhere — never expose
the plain-HTTP port to the internet.

## Updating

```sh
nix flake update --update-input noflex
nixos-rebuild switch
```

Backups made before an upgrade are forward-compatible across
patch versions; major upgrades may need a clean restore.

## Day-to-day

- `/health` — cheap storage-only check.
- `/healthz/deep` — storage + plugin host + ffmpeg presence;
  503 if anything's wrong.
- `/graphql` — the canonical API (GraphiQL in dev).
- `journalctl -u noflex` — logs.
