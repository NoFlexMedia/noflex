# noflex-webhook

Generic outbound webhook notifier — first-party WASM plugin. Bridges
NoFlex domain events ("added to the library", "now playing", favorites,
...) to any HTTP endpoint that accepts a JSON POST: Gotify, ntfy,
Discord, Slack, or your own receiver. Each event becomes one POST, performed by
the host through the capability-gated `host-net` import.

## Configuration

Set both from the admin UI (the Settings button beside the plugin on the
Plugins tab), or in files:

- `url` — the endpoint every event is POSTed to. Missing or empty, the
  plugin logs an error once and every notify is a no-op (operator
  misconfig never traps the instance).
- `auth_token` — a secret. When set, every POST carries
  `Authorization: Bearer <token>`. If it can't be read, the plugin warns
  and sends **without** the header — a missing secret never fails the
  notification.

In files, `url` goes in the server's `config.toml` and `auth_token` in
`webhook.toml` in a secret tier (`/etc/noflex/secrets/`,
`/var/lib/noflex/secrets/`, `$XDG_CONFIG_HOME/noflex/secrets/`). A value
set from the admin UI wins over both.

```toml
# config.toml
[plugin.per_plugin_config.webhook]
url = "https://ntfy.example.com/noflex"
```

```toml
# $XDG_CONFIG_HOME/noflex/secrets/webhook.toml
auth_token = "my-token"
```

Config is read once at instantiation; the host reloads the plugin when a
setting changes.

## Payload shape

Every POST carries `content-type: application/json` and a flat body:

```json
{"event": "<kind>", "data": { ...variant fields... }}
```

`<kind>` is the `DomainEvent` machine name (also what a manifest
`events` filter subscribes to); `data` carries the event's fields
verbatim:

| `event` | `data` |
|---|---|
| `work-added` | `{"work", "title"}` |
| `work-played` | `{"user", "work", "title"}` |
| `favorite` | `{"user", "work", "title", "favorite"}` |
| `progress-at-stop` | `{"user", "work", "title", "position_ms"}` |
| `playback-started` | `{"user", "work", "title", "media_file"}` |
| `playback-stopped` | `{"user", "work", "title", "media_file"}` |

IDs (`user`, `work`, `media_file`) are hyphenated UUID strings.

## Delivery contract

Webhooks are lossy by design. The plugin logs and swallows every
delivery failure — endpoint answering ≥ 400, connection refused,
timeout (10s per request), or a `host-net` plugin-error such as
capability-denied — and `notify` still returns Ok, so one dead endpoint
never errors the host's dispatch loop. Redirects are never followed
(the host returns 3xx as-is).

## Why `http_egress_hosts = ["*"]`

Unrestricted egress is deliberate for this plugin: the destination is
operator-configured, so it cannot be enumerated at build time (LAN
receivers included). The operator's configured URL **is** the egress
grant. The host warns at load about the unrestricted grant and
audit-logs every request — the audit log is the record of where events
actually went.
