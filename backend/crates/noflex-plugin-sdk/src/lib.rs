// SPDX-License-Identifier: AGPL-3.0-only
//
// noflex-plugin-sdk is the surface third-party plugin authors target.
// Goals:
//   1. Re-export the WIT types and the bindings-generator macro so plugins
//      only need to depend on this one crate.
//   2. Provide caching + request-coalescing helpers so providers like TMDb
//      and Fanart don't issue duplicate upstream requests when the host
//      calls `metadata::fetch` and `image::fetch` for the same external id
//      back-to-back.
//   3. Provide a thin `#[noflex_plugin]` proc-macro that lets authors
//      `impl MetadataProvider for MyPlugin` and have the
//      WIT export plumbing generated automatically.

pub mod cache;
pub mod singleflight;

pub use noflex_plugin_wit::{API_MAJOR, API_MINOR, PACKAGE, VERSION, WIT_DIR};

// Plugins call `wit_bindgen::generate!` directly with the relative WIT
// path. The SDK intentionally does NOT re-export `wit-bindgen` — doing
// so would link two copies of `libwit_bindgen` into the final component
// (one from the plugin's direct dep, one from the SDK's), and the
// duplicate `cabi_realloc_wit_bindgen_*` symbols collide at link time
// (or, worse, silently misbehave at runtime). See
// `plugins/noflex-metadata-tmdb/src/lib.rs` for the canonical pattern.
