// SPDX-License-Identifier: AGPL-3.0-only

// One integration-test binary per crate: a target per file would link
// its own copy of the dependency graph, Wasmtime included.

// The merged GraphQL Query root's resolver future is laid out past
// rustc's default query depth.
#![recursion_limit = "256"]

mod channel_logos;
mod download;
mod episode_thumbs;
mod external_auth;
mod fixtures;
mod graphql_ws;
mod images;
mod library_watchers;
mod live_tv_dvr;
mod optimize;
mod plugin_lifecycle;
mod plugin_links;
mod restrictions;
mod stream;
mod trickplay;
