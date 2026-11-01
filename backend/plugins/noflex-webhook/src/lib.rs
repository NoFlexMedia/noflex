// SPDX-License-Identifier: AGPL-3.0-only
//
// Generic outbound webhook notifier — first-party WASM plugin.
//
// Bridges NoFlex domain events to any HTTP endpoint that accepts a JSON
// POST (Gotify, ntfy, Discord, Slack, ...). Each `notify-event` becomes
// one POST to the operator-configured URL, performed by the host
// through the capability-gated `host-net` import. Delivery is lossy by
// design: a dead endpoint or a denied capability is logged, and the
// notify call still returns Ok so one bad target never errors the
// host's dispatch loop.
//
// The event→JSON mapping and config parsing live in `payload` — pure
// Rust with no WIT bindings, so `cargo test` exercises them on the
// host target (the bindings themselves only compile for wasm32).

// `pub` so the host build doesn't see the module as dead code — on
// wasm it is consumed by `guest`, on the host only by its unit tests.
pub mod payload;

#[cfg(target_arch = "wasm32")]
mod guest;
