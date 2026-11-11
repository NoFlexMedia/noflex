// SPDX-License-Identifier: AGPL-3.0-only

// One integration-test binary per crate: a target per file would link
// its own copy of the dependency graph, Wasmtime included. real_webhook
// stays a separate target because it mutates the process environment.

mod background_tasks;
mod concurrent_instance;
mod dispatcher;
mod embedder;
mod embedding_worker;
mod enqueue_task;
mod external_auth;
mod fixtures;
mod host_library;
mod intro_detector;
mod metadata_service;
mod net;
mod plugin_settings;
mod post_scan;
mod real_plugin;
mod remote_embedder;
mod scrobbler;
mod user_link;
