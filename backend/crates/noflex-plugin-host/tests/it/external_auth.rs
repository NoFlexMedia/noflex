// SPDX-License-Identifier: AGPL-3.0-only

use noflex_plugin_host::{EngineConfig, HookKind, PluginId, PluginRegistry, build_engine};

const PLUGIN_ID: &str = "fixture-external-auth";

#[tokio::test]
async fn real_external_auth_component_round_trips_and_validates_manifest() {
    let Some(wasm) = crate::fixtures::wasm("external-auth") else {
        eprintln!("skipping: could not build the external-auth fixture");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = PluginRegistry::new(engine).expect("registry");
    let info = registry.load_from(&wasm).expect("load fixture");
    assert_eq!(info.id, PLUGIN_ID);
    assert_eq!(info.manifest.hooks, ["external-auth"]);
    assert!(info.hooks.contains(&HookKind::Auth));

    let plugin = registry
        .get_plugin(&PluginId(PLUGIN_ID.into()))
        .expect("fixture registered");
    let callback = "https://media.example/auth/fixture-external-auth/callback";
    let start = plugin
        .call_external_auth_start(
            callback.into(),
            "fixture-state".into(),
            "fixture-pkce".into(),
        )
        .await
        .expect("start dispatch")
        .expect("start result");
    assert!(start.contains("state=fixture-state"));
    assert!(start.contains(callback));

    let identity = plugin
        .call_external_auth_complete(
            callback.into(),
            "fixture-code".into(),
            "fixture-verifier".into(),
        )
        .await
        .expect("complete dispatch")
        .expect("complete result");
    assert_eq!(identity.issuer, "https://id.example");
    assert_eq!(identity.subject, format!("subject:{callback}"));
    assert_eq!(identity.preferred_username.as_deref(), Some("fixture-user"));
}
