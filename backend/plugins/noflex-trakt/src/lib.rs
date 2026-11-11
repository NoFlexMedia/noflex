// SPDX-License-Identifier: AGPL-3.0-only
//
// Trakt scrobbler — first-party WASM plugin.
//
// Two surfaces over `https://api.trakt.tv`, all egress performed by the
// host through the capability-gated `host-net` import (guests have no
// socket access):
//
//   * `user-link` implements Trakt's OAuth device-authorization flow.
//     `link-start` requests a device code, enqueues a `poll-link`
//     background run, and returns the activation URL and the code to
//     enter there. `poll-link` polls `/oauth/device/token` at Trakt's
//     interval until the user authorizes or the code expires, then hands
//     the token pair to the host via `host-secrets.write-for-user` —
//     plugins never persist per-user tokens themselves. `unlink` stops a
//     link in progress and revokes the user's token at Trakt.
//   * `scrobbler` maps each host-enriched `scrobble-event` onto Trakt's
//     `/scrobble/start|pause|stop`, refreshing the access token once on
//     a 401. Delivery is lossy like the webhook notifier: failures are
//     logged, never propagated, so a dead upstream or expired grant
//     can't error the host's dispatch loop.
//
// Matching is external ids only (tmdb/tvdb/imdb), with an episode named
// by its series' ids and its numbers; no title matching, and a work
// without usable ids is skipped. Nothing is read back from Trakt: this is
// scrobbling, not history sync.
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
