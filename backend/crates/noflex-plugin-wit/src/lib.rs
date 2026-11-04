// SPDX-License-Identifier: AGPL-3.0-only
//
// noflex-plugin-wit is the single source of truth for the NoFlex plugin WIT
// world. The host (noflex-plugin-host) and the plugin SDK (noflex-plugin-sdk)
// both `wasmtime::component::bindgen!` against the same `wit/` directory.
//
// This crate intentionally contains almost no Rust — just stable constants
// for the package name, version, and on-disk path that downstream crates use
// to locate the WIT files at build time.

/// The WIT package name plugins target.
pub const PACKAGE: &str = "noflex:plugin";

/// The WIT package version this build of NoFlex understands.
///
/// Plugins declare `noflex_api_version = "0.9"` in their manifest. While the
/// major is 0 the host requires an exact `MAJOR.MINOR` match and rejects
/// anything else at load — every 0.x bump so far has been a breaking ABI
/// change (0.2: async hooks + `stream<u8>` payloads; 0.3:
/// `background` hook + `host-tasks` import; 0.4: `user-link` hook,
/// `host-secrets.read-for-user`, new `notify-event` actions; 0.5:
/// `embedder` hook + `host-assets` import; 0.6: `host-net` import on every
/// world + `post-scan-plugin` convenience world; 0.7: `scrobbler` +
/// `intro-detector` hooks, `host-library.lookup-edition-by-id`,
/// `host-secrets.write-for-user`; 0.8: dedicated `external-auth` hook; 0.9: `original-language`,
/// `origin-countries`, `keywords` and `budget-usd` on `metadata-payload`).
pub const VERSION: &str = "0.9.0";

/// Major component of [`VERSION`] used for the manifest compatibility check.
pub const API_MAJOR: u32 = 0;

/// Minor component of [`VERSION`].
pub const API_MINOR: u32 = 9;

/// Absolute path (resolved at compile time) of this crate's `wit/` directory.
///
/// Downstream code generators wire this in like:
///
/// ```ignore
/// wasmtime::component::bindgen!({
///     path: noflex_plugin_wit::WIT_DIR,
///     world: "plugin",
/// });
/// ```
pub const WIT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");
