# Deployment

Every release has the server for Linux, on x86_64 and arm64, and for Windows on
x86_64, each with its own ffmpeg, plugins and search model. Pick the install
that suits the machine; the sections after them apply to all. Server settings
are documented in [`CONFIGURATION.md`](CONFIGURATION.md), and what each hardware
accelerator needs from the host in [`PACKAGING.md`](PACKAGING.md#what-the-host-still-provides).

| Install | For | Config | Data |
| --- | --- | --- | --- |
| [Container](#container) | Docker, Podman, a NAS's app store, Unraid, TrueNAS | `/config/noflex/config.toml` | `/data`, `/cache` |
| [.deb or .rpm](#debian-ubuntu-fedora) | Debian, Ubuntu, Fedora, RHEL and their relatives | `/etc/noflex/config.toml` | `/var/lib/noflex`, `/var/cache/noflex` |
| [Bundle](#any-linux-from-the-bundle) | any Linux with glibc 2.28 or newer | `/etc/noflex/config.toml` | as you set it |
| [Installer](#windows) | Windows 10 and 11 | `%ProgramData%\NoFlex\config.toml` | `%ProgramData%\NoFlex` |
| [NixOS module](#nixos) | NixOS | the module's options | `/var/lib/noflex`, `/var/cache/noflex` |

Each listens on port 28793. Open `http://<the server>:28793` once it's running:
the first visit creates the administrator's account, then a library.

## Container

`ghcr.io/noflexmedia/noflex`, for amd64 and arm64, tagged with each release's
version, its `major.minor`, and `latest` for the newest that isn't a pre-release. [`packaging/container/compose.yaml`](../packaging/container/compose.yaml)
is a starting point: copy it, point the media volume at your library, then:

```sh
docker compose up -d
```

| Volume | Holds |
| --- | --- |
| `/config` | `noflex/config.toml`, `noflex/plugins/`, `noflex/secrets/` |
| `/data` | the database, optimized versions, recordings |
| `/cache` | transcodes, images and embeddings, all made again as needed |
| `/media` | your library, mounted read-only |

The image runs as root unless given `user:`, which should own the volumes. For
hardware transcoding, uncomment the GPU section for your card in `compose.yaml`:
Intel and AMD pass `/dev/dri` and the group that owns it, and Nvidia needs the
[NVIDIA Container Toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/)
on the host.

## Debian, Ubuntu, Fedora

Download the package for your architecture from the
[latest release](https://github.com/NoFlexMedia/noflex/releases), then:

```sh
sudo apt install ./noflex-server_<version>_amd64.deb     # or arm64
sudo dnf install ./noflex-server-<version>-1.x86_64.rpm  # or aarch64
sudo systemctl enable --now noflex
```

The server goes in `/opt/noflex` and runs as the `noflex` account, which the
package creates, with a hardened unit: it reads media anywhere, home directories
included, and writes only to `/var/lib/noflex` and `/var/cache/noflex`. Settings
go in `/etc/noflex/config.toml`, which upgrades leave alone. A directory the
server should also write to, such as a backup folder, goes in an override:

```sh
sudo systemctl edit noflex
# [Service]
# ReadWritePaths=/srv/backups
```

Upgrading is installing the next package, which restarts the server if it was
running. Removing it stops the server and leaves its data.

## Any Linux, from the bundle

`noflex-<version>-server-<target>.tar.zst` unpacks into a directory that runs as
it is, from anywhere: `./noflex-server`. To run it as a service the way the
packages do, unpack it into `/opt/noflex` and install the files in its `linux/`:

```sh
sudo mkdir -p /opt/noflex
sudo tar --zstd -xf noflex-<version>-server-x86_64-unknown-linux-gnu.tar.zst -C /opt/noflex --strip-components=1
cd /opt/noflex/linux
sudo install -Dm644 noflex.service /etc/systemd/system/noflex.service
sudo install -Dm644 noflex.sysusers /etc/sysusers.d/noflex.conf
sudo install -Dm644 config.toml /etc/noflex/config.toml
sudo systemd-sysusers && sudo systemctl daemon-reload && sudo systemctl enable --now noflex
```

## Windows

`noflex-<version>-server-windows-x64-setup.exe` installs the server under
Program Files and runs it as the `NoFlex` service, which starts with Windows and
restarts after a failure. It opens port 28793 to the server in the firewall, and
adds NoFlex to the Start menu. The installer isn't signed yet, so SmartScreen
asks before running it.

- Config is `%ProgramData%\NoFlex\config.toml`, which upgrades leave alone, and
  the log is `%ProgramData%\NoFlex\logs\noflex.log`. Restart the service after
  editing the config: `sc stop NoFlex`, then `sc start NoFlex`.
- `%ProgramData%\NoFlex` is open only to SYSTEM and Administrators, as the
  database holds keys to the servers people link.
- The service runs as LocalSystem, which reads local drives but not network
  shares, nor drive letters someone mapped. For a library on a share, use its
  `\\server\share` path and set the service to log on as an account that can
  reach it, in Services.
- Uninstalling removes the service and the program, and leaves `%ProgramData%\NoFlex`.

The `.zip` is the same bundle without an installer: `noflex-server.exe` runs in
the foreground, and `noflex-server.exe service install` registers it as the
service from wherever it is, as an administrator.

## NixOS

The flake's module (`nix/module.nix`) covers the common production knobs; for
every option, read the module source.

```nix
{
  services.noflex = {
    enable = true;
    dataDir = "/var/lib/noflex";
    cacheDir = "/var/cache/noflex";
    bind = "0.0.0.0:28793";
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

The Nix package doesn't carry the first-party plugins the other installs do;
add them as below.

## Adding a plugin

A plugin is a `.wasm` file in one of the server's plugin directories, which it
loads as soon as it appears. Later tiers shadow earlier ones:

| Tier | Linux | Windows | Container |
| --- | --- | --- | --- |
| Bundled | `plugins/` beside the server | the same | the same |
| System | `/etc/noflex/plugins` | `%ProgramData%\NoFlex\plugins` | |
| State | `/var/lib/noflex/plugins` | | |
| User | `$XDG_CONFIG_HOME/noflex/plugins` | | `/config/noflex/plugins` |

`NOFLEX_PLUGIN_SEARCH_PATH`, or the NixOS module's `pluginPath`, replaces the
list. Plugin API keys go in `<plugin-id>.toml` in a `secrets/` folder beside the
system, state or user tier's `plugins/`. To build a first-party plugin from source:

```sh
nix develop --command sh -c 'cd backend && cargo xtask plugin-build tmdb'
install -Dm644 backend/plugins/noflex-metadata-tmdb/target/wasm32-wasip2/release/noflex_metadata_tmdb.wasm \
  /var/lib/noflex/plugins/tmdb.wasm
```

See [`PLUGINS.md`](PLUGINS.md).

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
    proxy_pass http://127.0.0.1:28793;
  }

  # GraphQL subscriptions (watch rooms, live admin gauges) need the
  # WebSocket upgrade forwarded.
  location /graphql/ws {
    proxy_pass http://127.0.0.1:28793;
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

- **Container:** `docker compose pull && docker compose up -d`.
- **.deb or .rpm:** install the new release's package.
- **Windows:** run the new release's installer over the old one.
- **Bundle:** stop the server, unpack the new bundle over the old, start it.
- **NixOS:** `nix flake update --update-input noflex`, then `nixos-rebuild switch`.

Backups made before an upgrade are forward-compatible across
patch versions; major upgrades may need a clean restore.

## Day-to-day

- `/health` — cheap storage-only check.
- `/healthz/deep` — storage + plugin host + ffmpeg presence;
  503 if anything's wrong.
- `/graphql` — the canonical API (GraphiQL in dev).
- Logs: `journalctl -u noflex` under systemd, `docker compose logs` in a
  container, `%ProgramData%\NoFlex\logs\noflex.log` on Windows, and the Logs
  tab in the web client's administration everywhere.
