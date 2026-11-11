// SPDX-License-Identifier: AGPL-3.0-only
//
// T28: W7 pluginLifecycle GQL subscription bridge.
// The subscription reads from the same `LifecycleBus` the
// `PluginWatcher` writes into, so the test exercises the
// real end-to-end path (bus → stream → GQL Response) the
// production server uses.

use async_graphql::Request;
use futures::StreamExt;
use noflex_graphql::{PluginLifecycleEvent, SchemaContext, build_schema};
use noflex_plugin_host::{
    HookKind, LifecycleBus, PluginId, PluginLifecycleEvent as CanonicalLifecycleEvent, UnloadReason,
};

fn sample_manifest() -> noflex_plugin_host::PluginManifest {
    let toml = r#"
        manifest_version = 1
        plugin_id = "tmdb"
        display_name = "TMDb"
        version = "0.1.0"
        noflex_api_version = "0.2"
        hooks = ["metadata", "image"]
    "#;
    noflex_plugin_host::PluginManifest::from_bytes(toml.as_bytes()).expect("manifest parses")
}

async fn first_payload(
    schema: &async_graphql::Schema<
        noflex_graphql::Query,
        noflex_graphql::Mutation,
        noflex_graphql::Subscription,
    >,
    query: &str,
    emit: impl FnOnce() + Send + 'static,
) -> serde_json::Value {
    let mut stream = schema.execute_stream(Request::new(query).data(noflex_storage::RequestUser {
        user_id: noflex_core::UserId::new(),
        is_admin: true,
    }));
    tokio::spawn(async move {
        // Give the subscription a tick to register before we
        // emit, so the receiver is in the bus's recv list when
        // the broadcast lands. tokio::sync::broadcast only
        // delivers to receivers that called subscribe() before
        // the send, so this small sleep removes the race.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        emit();
    });
    let resp = stream
        .next()
        .await
        .expect("at least one response frame")
        .into_result()
        .expect("subscription stream did not error");
    resp.data.into_json().expect("valid json data")
}

#[tokio::test]
async fn loaded_event_reaches_the_subscriber() {
    let bus = LifecycleBus::new(8);
    let schema = build_schema(SchemaContext {
        lifecycle_bus: Some(bus.clone()),
        ..Default::default()
    });

    let query = r#"subscription { pluginLifecycle { kind pluginId sha256 hooks } }"#;
    let bus_for_emit = bus.clone();
    let emit = move || {
        bus_for_emit.emit(CanonicalLifecycleEvent::Loaded {
            plugin_id: PluginId("tmdb".into()),
            binary_sha256: "deadbeef".into(),
            hooks: vec![HookKind::Metadata, HookKind::Image],
        });
    };
    let data = first_payload(&schema, query, emit).await;
    let ev = &data["pluginLifecycle"];
    assert_eq!(ev["kind"], "LOADED");
    assert_eq!(ev["pluginId"], "tmdb");
    assert_eq!(ev["sha256"], "deadbeef");
    let hooks: Vec<&str> = ev["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(hooks, vec!["METADATA", "IMAGE"]);
}

#[tokio::test]
async fn load_failed_event_carries_path_and_reason() {
    let bus = LifecycleBus::new(8);
    let schema = build_schema(SchemaContext {
        lifecycle_bus: Some(bus.clone()),
        ..Default::default()
    });

    let query = r#"subscription { pluginLifecycle { kind path reason pluginId sha256 } }"#;
    let bus_for_emit = bus.clone();
    let emit = move || {
        bus_for_emit.emit(CanonicalLifecycleEvent::LoadFailed {
            path: std::path::PathBuf::from("/opt/noflex/plugins/broken.wasm"),
            reason: "manifest rejected: api version mismatch".into(),
        });
    };
    let data = first_payload(&schema, query, emit).await;
    let ev = &data["pluginLifecycle"];
    assert_eq!(ev["kind"], "LOAD_FAILED");
    assert_eq!(ev["path"], "/opt/noflex/plugins/broken.wasm");
    assert_eq!(ev["reason"], "manifest rejected: api version mismatch");
    assert!(ev["pluginId"].is_null());
    assert!(ev["sha256"].is_null());
}

#[tokio::test]
async fn unloaded_event_includes_reason_string() {
    let bus = LifecycleBus::new(8);
    let schema = build_schema(SchemaContext {
        lifecycle_bus: Some(bus.clone()),
        ..Default::default()
    });

    let query = r#"subscription { pluginLifecycle { kind pluginId reason } }"#;
    let bus_for_emit = bus.clone();
    let emit = move || {
        bus_for_emit.emit(CanonicalLifecycleEvent::Unloaded {
            plugin_id: PluginId("tvdb".into()),
            reason: UnloadReason::ReplacedByHotReload,
        });
    };
    let data = first_payload(&schema, query, emit).await;
    let ev = &data["pluginLifecycle"];
    assert_eq!(ev["kind"], "UNLOADED");
    assert_eq!(ev["pluginId"], "tvdb");
    assert_eq!(ev["reason"], "ReplacedByHotReload");
}

#[tokio::test]
async fn trapped_event_carries_hook_and_trap_message() {
    let bus = LifecycleBus::new(8);
    let schema = build_schema(SchemaContext {
        lifecycle_bus: Some(bus.clone()),
        ..Default::default()
    });

    let query = r#"subscription { pluginLifecycle { kind pluginId hooks trap } }"#;
    let bus_for_emit = bus.clone();
    let emit = move || {
        bus_for_emit.emit(CanonicalLifecycleEvent::Trapped {
            plugin_id: PluginId("musicbrainz".into()),
            hook: HookKind::Metadata,
            trap: "uncaught unreachable in fetch()".into(),
        });
    };
    let data = first_payload(&schema, query, emit).await;
    let ev = &data["pluginLifecycle"];
    assert_eq!(ev["kind"], "TRAPPED");
    assert_eq!(ev["pluginId"], "musicbrainz");
    assert_eq!(ev["trap"], "uncaught unreachable in fetch()");
    let hooks: Vec<&str> = ev["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(hooks, vec!["METADATA"]);
}

#[tokio::test]
async fn discovered_event_parses_manifest_hooks() {
    let bus = LifecycleBus::new(8);
    let schema = build_schema(SchemaContext {
        lifecycle_bus: Some(bus.clone()),
        ..Default::default()
    });

    let query = r#"subscription { pluginLifecycle { kind pluginId path sha256 hooks } }"#;
    let manifest = sample_manifest();
    let path = std::path::PathBuf::from("/opt/noflex/plugins/tmdb.wasm");
    let bus_for_emit = bus.clone();
    let emit = move || {
        bus_for_emit.emit(CanonicalLifecycleEvent::Discovered(Box::new(
            noflex_plugin_host::DiscoveredEvent {
                path,
                manifest,
                binary_sha256: "abc123".into(),
            },
        )));
    };
    let data = first_payload(&schema, query, emit).await;
    let ev = &data["pluginLifecycle"];
    assert_eq!(ev["kind"], "DISCOVERED");
    assert_eq!(ev["pluginId"], "tmdb");
    assert_eq!(ev["path"], "/opt/noflex/plugins/tmdb.wasm");
    assert_eq!(ev["sha256"], "abc123");
    let hooks: Vec<&str> = ev["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(hooks, vec!["METADATA", "IMAGE"]);
}

#[test]
fn from_conversion_populates_correct_fields_per_variant() {
    use noflex_plugin_host::{DiscoveredEvent, PluginManifest};

    // Loaded: only the success fields, no path/reason.
    let ev = CanonicalLifecycleEvent::Loaded {
        plugin_id: PluginId("x".into()),
        binary_sha256: "f".into(),
        hooks: vec![HookKind::Notifier],
    };
    let g: PluginLifecycleEvent = ev.into();
    assert_eq!(g.kind, noflex_graphql::PluginLifecycleKind::Loaded);
    assert_eq!(g.plugin_id.as_deref(), Some("x"));
    assert_eq!(g.sha256.as_deref(), Some("f"));
    assert_eq!(g.hooks, vec![HookKind::Notifier]);
    assert!(g.path.is_none());
    assert!(g.reason.is_none());
    assert!(g.trap.is_none());

    // Discovered: a manifest string hook that doesn't parse is
    // dropped from the `hooks` list (the kind field stays None
    // for the unknown tag).
    let m = PluginManifest::from_bytes(
        br#"
        manifest_version = 1
        plugin_id = "p"
        display_name = "P"
        version = "0.1.0"
        noflex_api_version = "0.2"
        hooks = ["notifier", "made-up-hook"]
        "#,
    )
    .unwrap();
    let ev = CanonicalLifecycleEvent::Discovered(Box::new(DiscoveredEvent {
        path: std::path::PathBuf::from("/p.wasm"),
        manifest: m,
        binary_sha256: "h".into(),
    }));
    let g: PluginLifecycleEvent = ev.into();
    assert_eq!(g.kind, noflex_graphql::PluginLifecycleKind::Discovered);
    assert_eq!(g.hooks, vec![HookKind::Notifier]);
}
