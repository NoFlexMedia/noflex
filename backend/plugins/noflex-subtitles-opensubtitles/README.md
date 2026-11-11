# noflex-subtitles-opensubtitles

Online subtitle search from [OpenSubtitles.com](https://www.opensubtitles.com)
— first-party WASM plugin for the `subtitle` hook. A viewer searches from
the player, picks one, and NoFlex keeps it in its own subtitle cache as one
of the file's tracks; nothing is written into the media folder.

## Setup

NoFlex has no shared OpenSubtitles key, so this plugin does nothing until an
administrator gives it one. Without a key every search reports the plugin as
unavailable and says so.

1. Create an account on opensubtitles.com, and an API consumer under your
   profile's API consumers page. Its API key is what this plugin needs.
2. Set it from the admin UI (the Settings button beside the plugin on the
   Plugins tab), or in `opensubtitles.toml` in a secret tier
   (`/etc/noflex/secrets/`, `/var/lib/noflex/secrets/`,
   `$XDG_CONFIG_HOME/noflex/secrets/`):

   ```toml
   api_key = "..."
   # Optional: signing in raises the daily download quota to the account's.
   username = "..."
   password = "..."
   ```

| Setting    | Kind   | What it's for                                                 |
| ---------- | ------ | ------------------------------------------------------------- |
| `api_key`  | secret | Every request. Required.                                      |
| `username` | secret | Signing in, with `password`, for the account's download quota |
| `password` | secret | Signing in                                                    |

## What it sends

A search sends the file's OpenSubtitles moviehash and size, its IMDb or TMDb
ids (an episode's series ids with its season and episode numbers), the
wanted languages and, when nothing matches those, its title and year.
Requests carry the API key, a `User-Agent` naming NoFlex and its version,
and once signed in the session token. The plugin reaches
`api.opensubtitles.com`, `vip-api.opensubtitles.com` (a VIP account's API
host) and `www.opensubtitles.com` (where download links point), and nothing
else.

## Limits

- **Requests** are paced at 5 a second, OpenSubtitles' limit; a `429` backs
  the plugin off for its `Retry-After`.
- **Downloads** count against a daily quota: the API key's own without an
  account, the account's when signed in. A used-up quota answers
  `rate-limited` with the seconds until it resets, which the player shows.
- Every download asks the API for SubRip, which it converts to from
  whatever was uploaded. Uploads split over several files (one per disc)
  are left out of results, as they can't be laid over a single file.
- AI-translated subtitles are left out of results.

## Tests

`cargo test` here checks request building and parsing against the
responses in `fixtures/`, which are written by hand after the documented
shapes; see `fixtures/README.md`.
