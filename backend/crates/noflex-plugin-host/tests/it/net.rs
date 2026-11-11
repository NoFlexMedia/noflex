// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end tests for the `host-net` import: a fixture guest drives
// host-performed HTTP requests from inside `notifier.notify` (custom
// event payload `METHOD URL [body-len]`) and records the outcome; the
// host test reads the log back through the fixture's `report` task.
// A local axum server on an ephemeral 127.0.0.1 port stands in for the
// upstream. Capability / allowlist / size-cap rejections all happen
// host-side before any network I/O, so those cases need no server.
//
// The fixture components are built on demand (cargo, wasm32-wasip2)
// when the artifacts are missing; the tests self-skip if the toolchain
// can't produce them.

use std::sync::Arc;

use noflex_plugin_host::{
    EngineConfig, LoadedPlugin, PluginId, PluginRegistry, TaskOutcome, build_engine,
    notifier::noflex::plugin::types::{NotifyEvent, WorkAddedEvent},
};

const NET_ID: &str = "fixture-net";
const DENIED_ID: &str = "fixture-net-denied";

/// Spin up the upstream stand-in: `/hello` answers a fixed body,
/// `/redirect` answers a bare 302 (never followed per the host-net
/// contract), `/echo` returns the request body. Returns the base URL.
async fn spawn_server() -> String {
    use axum::{Router, routing};

    let app = Router::new()
        .route("/hello", routing::get(|| async { "hello-egress" }))
        .route(
            "/redirect",
            routing::get(|| async {
                (
                    axum::http::StatusCode::FOUND,
                    [(axum::http::header::LOCATION, "http://example.invalid/")],
                )
            }),
        )
        .route(
            "/echo",
            routing::post(|body: axum::body::Bytes| async move { body }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    format!("http://127.0.0.1:{port}")
}

fn registry_with(fixture: &str, plugin_id: &str) -> Option<(PluginRegistry, Arc<LoadedPlugin>)> {
    let wasm = crate::fixtures::wasm(fixture)?;
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load {fixture} fixture: {e}"));
    let plugin = registry
        .get_plugin(&PluginId(plugin_id.to_string()))
        .expect("fixture registered");
    Some((registry, plugin))
}

fn load_net() -> Option<(PluginRegistry, Arc<LoadedPlugin>)> {
    registry_with("net", NET_ID)
}

fn load_denied() -> Option<(PluginRegistry, Arc<LoadedPlugin>)> {
    registry_with("net-denied", DENIED_ID)
}

/// Drive one guest-side egress attempt and return its outcome line.
/// `call_notify` awaits the guest, so the line is in the log by the
/// time the call returns.
async fn drive(plugin: &Arc<LoadedPlugin>, payload: &str) -> String {
    plugin
        .call_notify(NotifyEvent::WorkAdded(WorkAddedEvent {
            work: "net".into(),
            title: payload.into(),
        }))
        .await
        .expect("notify dispatch succeeds")
        .expect("notify succeeds");
    let outcome = plugin
        .call_run_task("report".to_string(), None, None)
        .await
        .expect("report dispatch succeeds")
        .expect("report succeeds");
    let TaskOutcome::More(log) = outcome else {
        panic!("report always answers `more`");
    };
    log.lines().last().unwrap_or_default().trim().to_string()
}

#[tokio::test]
async fn egress_to_allowlisted_host_succeeds() {
    let Some((_registry, plugin)) = load_net() else {
        eprintln!("skipping: could not build the fixture-net plugin");
        return;
    };
    let base = spawn_server().await;
    let line = drive(&plugin, &format!("GET {base}/hello")).await;
    assert_eq!(line, "ok:200:hello-egress");

    // A request body under the cap is delivered and echoed back.
    let line = drive(&plugin, &format!("POST {base}/echo 1024")).await;
    assert_eq!(line.len(), "ok:200:".len() + 1024, "echoed body: {line:?}");
}

#[tokio::test]
async fn egress_to_unlisted_host_is_denied() {
    let Some((_registry, plugin)) = load_net() else {
        eprintln!("skipping: could not build the fixture-net plugin");
        return;
    };
    // Nothing listens here; the point is the deny happens before any
    // connection attempt.
    let line = drive(&plugin, "GET http://127.0.0.2:9/").await;
    assert!(
        line.starts_with("err:capability-denied:") && line.contains("127.0.0.2"),
        "unlisted host denied, naming the host: {line:?}"
    );
}

#[tokio::test]
async fn egress_without_capability_is_denied() {
    let Some((_registry, plugin)) = load_denied() else {
        eprintln!("skipping: could not build the fixture-net-denied plugin");
        return;
    };
    let base = spawn_server().await;
    let line = drive(&plugin, &format!("GET {base}/hello")).await;
    assert!(
        line.starts_with("err:capability-denied:") && line.contains("http.egress"),
        "missing capability denied: {line:?}"
    );
}

#[tokio::test]
async fn wildcard_matches_subdomain_but_not_the_apex() {
    let Some((_registry, plugin)) = load_net() else {
        eprintln!("skipping: could not build the fixture-net plugin");
        return;
    };
    // The manifest allowlists `*.wild.example.com`: the apex is not a
    // subdomain of itself, so it is denied before any DNS lookup.
    let line = drive(&plugin, "GET http://wild.example.com/").await;
    assert!(
        line.starts_with("err:capability-denied:"),
        "apex not matched by the wildcard: {line:?}"
    );
    // The subdomain passes the allowlist; the request then fails at
    // DNS (NXDOMAIN), proving the gate let it through.
    let line = drive(&plugin, "GET http://sub.wild.example.com/").await;
    assert!(
        !line.starts_with("err:capability-denied:"),
        "subdomain must pass the allowlist (fails later at DNS): {line:?}"
    );
}

#[tokio::test]
async fn redirect_is_returned_not_followed() {
    let Some((_registry, plugin)) = load_net() else {
        eprintln!("skipping: could not build the fixture-net plugin");
        return;
    };
    let base = spawn_server().await;
    let line = drive(&plugin, &format!("GET {base}/redirect")).await;
    assert_eq!(line, "ok:302:", "the 3xx reaches the guest as-is");
}

#[tokio::test]
async fn oversized_request_body_is_rejected() {
    let Some((_registry, plugin)) = load_net() else {
        eprintln!("skipping: could not build the fixture-net plugin");
        return;
    };
    let base = spawn_server().await;
    // 4 MiB + 1 byte, generated guest-side.
    let line = drive(
        &plugin,
        &format!("POST {base}/echo {}", 4 * 1024 * 1024 + 1),
    )
    .await;
    assert!(
        line.starts_with("err:bad-input:") && line.contains("cap"),
        "oversized body rejected: {line:?}"
    );
}

#[tokio::test]
async fn unsupported_scheme_and_method_are_bad_input() {
    let Some((_registry, plugin)) = load_net() else {
        eprintln!("skipping: could not build the fixture-net plugin");
        return;
    };
    let line = drive(&plugin, "GET ftp://127.0.0.1/file").await;
    assert!(line.starts_with("err:bad-input:"), "scheme gate: {line:?}");
    let line = drive(&plugin, "OPTIONS http://127.0.0.1/").await;
    assert!(line.starts_with("err:bad-input:"), "method gate: {line:?}");
}
