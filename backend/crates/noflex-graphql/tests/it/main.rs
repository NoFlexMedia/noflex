// SPDX-License-Identifier: AGPL-3.0-only

// One integration-test binary per crate: a target per file would link
// its own copy of the dependency graph, Wasmtime included.

// The merged GraphQL Query root's resolver future is laid out past
// rustc's default query depth.
#![recursion_limit = "256"]

mod access_watch;
mod active_streams;
mod admin_status;
mod auth;
mod canonical_identity;
mod client_contract;
mod collections;
mod deletion;
mod device_download;
mod export_jobs;
mod external_auth;
mod library_crud;
mod live_tv;
mod metadata_manage;
mod metadata_search;
mod multi_server;
mod play_stats;
mod playback_events;
mod plugin_lifecycle;
mod plugin_links;
mod preferences;
mod request_budget;
mod scan_jobs;
mod schema_snapshot;
mod search;
mod sessions;
mod setup_wizard;
mod taxonomy;
mod ui_api;
mod users;
