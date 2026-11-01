# noflex-webhook

Generic outbound webhook notifier — first-party WASM plugin. Bridges
NoFlex domain events ("scan finished", "now playing", favorites, ...)
to any HTTP endpoint that accepts a JSON POST: Gotify, ntfy, Discord,
Slack, or your own receiver. Each event becomes one POST, performed by
the host through the capability-gated `host-net` import.

## Configuration

In the server's `config.toml`:

```toml
[plugins.per_plugin_config.webhook]
url = "https://ntfy.example.com/noflex"   # required
auth_secret_key = "webhook.auth"          # optional: name of a secrets-read key
```

- `url` — the endpoint every event is POSTed to. Missing or empty, the
  plugin logs an error once and every notify is a no-op (operator
  misconfig never traps the instance).
- `auth_secret_key` — when set, the plugin reads the secret of that name
  via `host-secrets` and sends it as `Authorization: Bearer <value>`.
  The key must be listed in the manifest's `secrets_read` (`webhook.auth`
  ships declared; use that name unless you re-manifest). If the read
  fails, the plugin warns and sends **without** the header — a missing
  secret never fails the notification.

Secrets live in the tiered store (`/etc/noflex/secrets/`,
`/var/lib/noflex/secrets/`, `$XDG_CONFIG_HOME/noflex/secrets/`) as
`<plugin_id>.toml`. Dotted keys nest in TOML, so quote them:

```toml
# $XDG_CONFIG_HOME/noflex/secrets/webhook.toml
"webhook.auth" = "my-token"
```

Config is read once at instantiation; the host re-instantiates the
plugin when the server config changes.

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
| `scan-started` | `{"library_name"}` |
| `scan-completed` | `{"library_name", "added", "updated", "removed"}` |
| `favorite` | `{"user", "work", "title", "favorite"}` |
| `progress-at-stop` | `{"user", "work", "title", "position_ms"}` |
| `playback-started` | `{"user", "work", "title", "media_file"}` |
| `playback-stopped` | `{"user", "work", "title", "media_file"}` |
| `custom` | `{"event", "payload_json"}` |

IDs (`user`, `work`, `media_file`) are hyphenated UUID strings.
`payload_json` stays a **string containing JSON** — the plugin never
double-decodes it.

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
