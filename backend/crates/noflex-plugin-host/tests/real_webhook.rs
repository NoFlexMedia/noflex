// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end test for the real first-party webhook component: the
// domain-event bus fans events out through the dispatcher, the plugin
// maps them to JSON and POSTs via `host-net`, and a local axum sink on
// 127.0.0.1 captures what arrives. The second test provisions the
// `webhook.auth` secret through the XDG secrets tier and asserts the
// Bearer header rides along.
//
// Gated on the plugin artifact existing on disk, matching
// real_plugin.rs conventions: build first with
// `cargo xtask plugin-build webhook`.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use axum::{Router, http::HeaderMap, routing::post};
use noflex_core::{
    LibraryId, UserId, WorkId,
    events::{DomainEvent, DomainEventBus},
};
use noflex_plugin_host::{EngineConfig, HookKind, PluginRegistry, build_engine};
use tokio::sync::mpsc;

const TIMEOUT: Duration = Duration::from_secs(15);

fn plugin_wasm_path() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent()?.parent()?;
    let p = workspace_root
        .join("plugins/noflex-webhook/target/wasm32-wasip2/release/noflex_webhook.wasm");
    p.is_file().then_some(p)
}

struct Captured {
    content_type: Option<String>,
    authorization: Option<String>,
    body: serde_json::Value,
}

/// Stand-in for the operator's webhook endpoint. Captures each POST as
/// a header pair + parsed JSON body (a malformed body fails the test
/// right at the sink, not in an assertion later).
async fn spawn_sink() -> (String, mpsc::Receiver<Captured>) {
    let (tx, rx) = mpsc::channel(16);
    let app = Router::new().route(
        "/hook",
        post(move |headers: HeaderMap, body: String| {
            let tx = tx.clone();
            async move {
                let captured = Captured {
                    content_type: headers
                        .get(axum::http::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok().map(str::to_string)),
                    authorization: headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok().map(str::to_string)),
                    body: serde_json::from_str(&body).expect("webhook body is valid JSON"),
                };
                let _ = tx.send(captured).await;
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (format!("http://127.0.0.1:{port}"), rx)
}

async fn recv(rx: &mut mpsc::Receiver<Captured>, what: &str) -> Captured {
    tokio::time::timeout(TIMEOUT, rx.recv())
        .await
        .unwrap_or_else(|_| panic!("timed out after {TIMEOUT:?} waiting for {what}"))
        .expect("sink channel stays open")
}

fn registry_with_plugin(wasm: &Path, config_toml: String) -> (DomainEventBus, PluginRegistry) {
    let bus = DomainEventBus::default();
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_event_bus(bus.clone());
    let info = registry
        .load_from_with_config(wasm, config_toml)
        .unwrap_or_else(|e| panic!("load webhook plugin: {e}"));
    assert_eq!(info.id, "webhook", "plugin_id from embedded manifest");
    assert!(
        info.hooks.contains(&HookKind::Notifier),
        "exports notifier hook"
    );
    (bus, registry)
}

#[tokio::test]
async fn posts_domain_events_as_json() {
    let Some(wasm) = plugin_wasm_path() else {
        eprintln!("skipping: build the plugin first with `cargo xtask plugin-build webhook`");
        return;
    };
    let (base, mut rx) = spawn_sink().await;
    let (bus, _registry) = registry_with_plugin(&wasm, format!("url = \"{base}/hook\"\n"));

    let user = UserId::new();
    let work = WorkId::new();
    bus.emit(DomainEvent::WorkPlayed {
        user,
        work,
        title: "The Matrix".into(),
    });
    bus.emit(DomainEvent::ScanCompleted {
        library: LibraryId::new(),
        library_name: "Movies".into(),
        added: 3,
        updated: 1,
        removed: 0,
        added_works: vec![],
        updated_works: vec![],
        removed_works: vec![],
    });

    let first = recv(&mut rx, "work-played POST").await;
    assert_eq!(first.content_type.as_deref(), Some("application/json"));
    assert_eq!(first.authorization, None, "no auth_secret_key configured");
    assert_eq!(first.body["event"], "work-played");
    assert_eq!(
        first.body["data"],
        serde_json::json!({
            "user": user.to_string(),
            "work": work.to_string(),
            "title": "The Matrix",
        })
    );

    let second = recv(&mut rx, "scan-completed POST").await;
    assert_eq!(second.body["event"], "scan-completed");
    assert_eq!(
        second.body["data"],
        serde_json::json!({
            "library_name": "Movies",
            "added": 3,
            "updated": 1,
            "removed": 0,
        })
    );
}

#[tokio::test]
async fn sends_bearer_header_when_secret_is_provisioned() {
    let Some(wasm) = plugin_wasm_path() else {
        eprintln!("skipping: build the plugin first with `cargo xtask plugin-build webhook`");
        return;
    };
    // Plugin-global secrets come from on-disk tiers; the XDG tier is the
    // only one a test can point anywhere, so it carries the provisioned
    // `webhook.auth` value.
    let tier = tempfile::tempdir().expect("tempdir");
    let secrets_dir = tier.path().join("noflex").join("secrets");
    std::fs::create_dir_all(&secrets_dir).expect("secrets dir");
    // Dotted keys nest in TOML; the flat `webhook.auth` secret key
    // must be quoted.
    std::fs::write(
        secrets_dir.join("webhook.toml"),
        "\"webhook.auth\" = \"test-token-123\"\n",
    )
    .expect("write secret");
    // SAFETY: this is the only test in the binary that mutates the env,
    // and secret reads (the only getenv on this path) happen on this
    // test's own dispatch. The previous value is restored afterwards.
    let prior_xdg = std::env::var_os("XDG_CONFIG_HOME");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tier.path());
    }

    let (base, mut rx) = spawn_sink().await;
    let (bus, _registry) = registry_with_plugin(
        &wasm,
        format!("url = \"{base}/hook\"\nauth_secret_key = \"webhook.auth\"\n"),
    );
    bus.emit(DomainEvent::ScanStarted {
        library: LibraryId::new(),
        library_name: "Movies".into(),
    });

    let captured = recv(&mut rx, "scan-started POST").await;
    assert_eq!(captured.body["event"], "scan-started");
    assert_eq!(captured.body["data"]["library_name"], "Movies");
    assert_eq!(
        captured.authorization.as_deref(),
        Some("Bearer test-token-123")
    );

    // Restore before the tempdir drops so no later getenv sees a
    // dangling path.
    unsafe {
        match prior_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}
