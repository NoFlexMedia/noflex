// SPDX-License-Identifier: AGPL-3.0-only
//
// Library surface for the `noflex-server` crate. The `main.rs`
// binary is a thin wrapper around this — the heavy lifting
// (config, metrics, download worker) lives here so the
// integration tests in `tests/` can reach it. The
// `ExportEventBroadcaster` itself lives in `noflex-graphql`
// (where the GQL subscription resolver lives) and is
// re-exported here as `subscriptions` for the worker's
// convenience.

// The merged GraphQL Query root's resolver future, embedded in the
// server's handlers, is laid out past rustc's default query depth.
#![recursion_limit = "256"]

pub mod attributions;
pub mod channel_logos;
pub mod config;
pub mod download;
pub mod download_serve;
pub mod episode_thumbs;
pub mod erasure;
pub mod external_auth;
pub mod file_serve;
pub mod graphql_ws;
pub mod hwaccel;
pub mod images;
pub mod library_watchers;
pub mod live_tv;
pub mod live_tv_runtime;
pub mod metrics;
pub mod movie_art;
pub mod optimize_worker;
pub mod optimized_serve;
pub mod plugin_links;
pub mod restricted_routes;
pub mod scan_scheduler;
pub mod stream;
pub mod subprocess;
pub mod system_status;
pub mod trickplay;
pub mod trickplay_serve;
pub mod web;

/// Re-export the broadcaster from `noflex-graphql` so
/// `download::DownloadWorker` and the rest of the server
/// don't have to import a `noflex-graphql::` path.
pub use noflex_graphql::ExportEventBroadcaster as ExportEvents;
