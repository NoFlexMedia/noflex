// SPDX-License-Identifier: AGPL-3.0-only
//
// Chapter-title heuristic intro/credits detector — first-party WASM
// plugin.
//
// The host sweeps editions that have never been through detection and
// calls `intro-detector.detect`; the guest reads the edition's chapter
// list, runtime, and already-persisted markers through
// `host-library.lookup-edition-by-id` and guesses intro/credits bounds
// from chapter titles ("Intro", "Opening", "OP", "Prologue",
// "Cold Open", "Credits", "Ending", ...). An edition that already
// carries any marker is left untouched — the detector never refines,
// so a wrong guess is only ever corrected by hand, not by re-running.
//
// The heuristic itself lives in `detect` — pure Rust with no WIT
// bindings, so `cargo test` exercises it on the host target (the
// bindings themselves only compile for wasm32).

// `pub` so the host build doesn't see the module as dead code — on
// wasm it is consumed by `guest`, on the host only by its unit tests.
pub mod detect;

#[cfg(target_arch = "wasm32")]
mod guest;
