// SPDX-License-Identifier: AGPL-3.0-only
//
// Trakt scrobbler — first-party WASM plugin.
//
// Two surfaces over `https://api.trakt.tv`, all egress performed by the
// host through the capability-gated `host-net` import (guests have no
// socket access):
//
//   * `user-link` implements Trakt's OAuth device-authorization flow.
//     `link-start` requests a device code, stashes it in instance state,
//     enqueues a `poll-link` background run, and returns the activation
//     URL. `poll-link` polls `/oauth/device/token` until the user
//     authorizes, then hands the token pair to the host via
//     `host-secrets.write-for-user` — plugins never persist per-user
//     tokens themselves.
//   * `scrobbler` maps each host-enriched `scrobble-event` onto Trakt's
//     `/scrobble/start|pause|stop`, refreshing the access token once on
//     a 401. Delivery is lossy like the webhook notifier: failures are
//     logged, never propagated, so a dead upstream or expired grant
//     can't error the host's dispatch loop.
//
// Known warts (deliberate, documented rather than redesigned):
//   * The WIT `link-challenge` carries only a URL plus opaque state —
//     there is no field for the user code Trakt shows at
//     https://trakt.tv/activate. The code is therefore surfaced via
//     `host-log` at link-start time; the URL alone is returned.
//   * Matching is external-ids only (tmdb/tvdb/imdb). No title
//     matching, no show-level fallback lookups — a work without usable
//     ids is skipped.
//
// The Trakt request/response mapping and the poll-cursor codec live in
// `api` — pure Rust with no WIT bindings, so `cargo test` exercises
// them on the host target (the bindings themselves only compile for
// wasm32).

// `pub` so the host build doesn't see the module as dead code — on
// wasm it is consumed by `guest`, on the host only by its unit tests.
pub mod api;

#[cfg(target_arch = "wasm32")]
mod guest;
