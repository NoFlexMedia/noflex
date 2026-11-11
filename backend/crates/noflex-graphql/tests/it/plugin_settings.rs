// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::PathBuf, sync::Arc};

use async_graphql::Request;
use noflex_core::UserId;
use noflex_graphql::{SchemaContext, build_schema};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
use noflex_storage::{RequestUser, Storage};
use noflex_storage_turso::TursoStore;
use serde_json::json;

/// The plugin host's concurrency fixture, whose manifest declares a text, an
/// integer and a secret setting. Built by `noflex-plugin-host`'s tests.
fn fixture() -> Option<PathBuf> {
    let wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../noflex-plugin-host/tests/fixtures/target/wasm32-wasip2/release/\
         noflex_fixture_concurrency.wasm",
    );
    wasm.is_file().then_some(wasm)
}

fn as_user(is_admin: bool) -> RequestUser {
    RequestUser {
        user_id: UserId::new(),
        is_admin,
    }
}

const SET: &str = r#"
    mutation Set($values: [PluginSettingInput!]!) {
        setPluginSettings(pluginId: "concurrency-fixture", values: $values) {
            id
            settings { key kind value isSet source }
        }
    }
"#;

#[tokio::test]
async fn administrators_set_settings_and_never_read_a_secret_back() {
    let Some(wasm) = fixture() else {
        eprintln!("skipping: build the plugin host's fixtures first");
        return;
    };
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = store.clone();
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(
        PluginRegistry::new(engine)
            .expect("registry")
            .with_storage(storage),
    );
    registry.load_from(&wasm).expect("load fixture");
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        plugins: Some(registry),
        ..Default::default()
    });
    let set = |values: serde_json::Value, admin: bool| {
        Request::new(SET)
            .variables(async_graphql::Variables::from_json(
                json!({ "values": values }),
            ))
            .data(as_user(admin))
    };

    let refused = schema
        .execute(set(json!([{ "key": "token", "value": "x" }]), false))
        .await;
    assert_eq!(refused.errors[0].message, "administrator required");

    let resp = schema
        .execute(set(
            json!([
                { "key": "token", "value": "from-admin" },
                { "key": "password", "value": "hunter2" },
            ]),
            true,
        ))
        .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert!(!data.to_string().contains("hunter2"));
    assert_eq!(
        data["setPluginSettings"]["settings"],
        json!([
            { "key": "token", "kind": "TEXT", "value": "from-admin", "isSet": true, "source": "ADMIN" },
            { "key": "retries", "kind": "INTEGER", "value": "3", "isSet": true, "source": "DEFAULT" },
            { "key": "password", "kind": "SECRET", "value": null, "isSet": true, "source": "ADMIN" },
        ])
    );

    let bad = schema
        .execute(set(json!([{ "key": "retries", "value": "lots" }]), true))
        .await;
    let code = bad.errors[0]
        .extensions
        .as_ref()
        .and_then(|ext| ext.get("code"))
        .cloned();
    assert_eq!(
        code,
        Some(async_graphql::Value::from("INVALID_PLUGIN_SETTING"))
    );
}
