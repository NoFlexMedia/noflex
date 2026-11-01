// SPDX-License-Identifier: AGPL-3.0-only
//
// T28 (server-side): the `LifecycleBus` is constructed in
// `main.rs`, handed to the `PluginWatcher` (which writes),
// to the schema context (which `pluginLifecycle` subscribes
// from). This test exercises the bus + the GQL subscription
// end-to-end at the server crate's integration level, mirroring
// what the production binary does at startup.

use async_graphql::Request;
use futures::StreamExt;
use noflex_graphql::{SchemaContext, build_schema};
use noflex_plugin_host::{HookKind, LifecycleBus, PluginId, PluginLifecycleEvent};
use noflex_storage::RequestUser;

/// `pluginLifecycle` is admin-only.
fn admin() -> RequestUser {
    RequestUser {
        user_id: noflex_core::UserId::new(),
        is_admin: true,
    }
}

#[tokio::test]
async fn gql_subscription_receives_plugin_lifecycle_events_from_bus() {
    let bus = LifecycleBus::new(8);
    let schema = build_schema(SchemaContext {
        lifecycle_bus: Some(bus.clone()),
        ..Default::default()
    });

    let query = r#"subscription { pluginLifecycle { kind pluginId sha256 hooks } }"#;
    let mut stream = schema.execute_stream(Request::new(query).data(admin()));

    let bus_for_emit = bus.clone();
    let emit = tokio::spawn(async move {
        // Race-free: the GQL subscription registers the bus
        // subscriber inside the resolver, which is the next
        // thing the runtime polls. A tiny yield gives the
        // subscriber time to register before we send.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        bus_for_emit.emit(PluginLifecycleEvent::Loaded {
            plugin_id: PluginId("tmdb".into()),
            binary_sha256: "caffe".into(),
            hooks: vec![HookKind::Metadata, HookKind::Image],
        });
    });

    let resp = stream
        .next()
        .await
        .expect("at least one response frame")
        .into_result()
        .expect("subscription stream did not error");
    emit.await.expect("emit task");

    let data = resp.data.into_json().expect("valid json data");
    let ev = &data["pluginLifecycle"];
    assert_eq!(ev["kind"], "LOADED");
    assert_eq!(ev["pluginId"], "tmdb");
    assert_eq!(ev["sha256"], "caffe");
    let hooks: Vec<&str> = ev["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(hooks, vec!["METADATA", "IMAGE"]);
}

#[tokio::test]
async fn multiple_lifecycle_events_flow_through_in_order() {
    let bus = LifecycleBus::new(8);
    let schema = build_schema(SchemaContext {
        lifecycle_bus: Some(bus.clone()),
        ..Default::default()
    });

    let query = r#"subscription { pluginLifecycle { kind pluginId } }"#;
    let mut stream = schema.execute_stream(Request::new(query).data(admin()));

    let bus_for_emit = bus.clone();
    let emit = tokio::spawn(async move {
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        bus_for_emit.emit(PluginLifecycleEvent::Loaded {
            plugin_id: PluginId("tmdb".into()),
            binary_sha256: "a".into(),
            hooks: vec![HookKind::Metadata],
        });
        bus_for_emit.emit(PluginLifecycleEvent::Unloaded {
            plugin_id: PluginId("tmdb".into()),
            reason: noflex_plugin_host::UnloadReason::Shutdown,
        });
    });

    let mut kinds: Vec<String> = Vec::new();
    for _ in 0..2 {
        let resp = stream
            .next()
            .await
            .expect("at least one response frame")
            .into_result()
            .expect("subscription stream did not error");
        let data = resp.data.into_json().expect("valid json data");
        let kind = data["pluginLifecycle"]["kind"]
            .as_str()
            .unwrap()
            .to_string();
        kinds.push(kind);
    }
    emit.await.expect("emit task");

    assert_eq!(kinds, vec!["LOADED", "UNLOADED"]);
}
