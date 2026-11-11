// SPDX-License-Identifier: AGPL-3.0-only
//
// Admin-set plugin settings: they reach the guest's config over
// config.toml, reload the plugin when they change (secrets included), and
// survive the watcher's next sync.

use std::{collections::HashMap, sync::Arc};

use noflex_plugin_host::{
    EngineConfig, LifecycleBus, PluginId, PluginRegistry, SearchPath, SettingSource, SettingsError,
    build_engine, watcher,
};
use noflex_storage::Storage;
use noflex_storage_turso::TursoStore;

fn token(config_toml: &str) -> (Option<String>, Option<i64>) {
    let config: toml::Table = toml::from_str(config_toml).expect("a TOML document");
    (
        config
            .get("token")
            .and_then(|v| v.as_str())
            .map(String::from),
        config.get("retries").and_then(toml::Value::as_integer),
    )
}

fn set(key: &str, value: Option<&str>) -> (String, Option<String>) {
    (key.to_string(), value.map(String::from))
}

#[tokio::test]
async fn settings_reload_the_plugin_and_outlive_a_sync() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("plugins");
    std::fs::create_dir(&plugins).unwrap();
    std::fs::copy(wasm, plugins.join("fixture.wasm")).unwrap();
    let store = TursoStore::open(dir.path().join("settings.db"))
        .await
        .expect("open store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_storage(storage.clone())
        .with_plugin_config(HashMap::from([(
            "concurrency-fixture".to_string(),
            toml::toml! { token = "from-config" },
        )]));
    let search_path = SearchPath::from_explicit([&plugins]);
    let bus = LifecycleBus::default();
    let id = PluginId("concurrency-fixture".into());

    watcher::sync_once(&registry, &bus, &search_path).await;
    let first = registry.get_plugin(&id).expect("loaded");
    assert_eq!(
        token(&first.config_toml),
        (Some("from-config".into()), Some(3)),
        "config.toml, then the manifest default"
    );

    let updated = registry
        .update_settings(&id, vec![set("token", Some("from-admin"))], "admin")
        .await
        .expect("update");
    let reloaded = registry.get_plugin(&id).expect("loaded");
    assert!(
        !Arc::ptr_eq(&first, &reloaded),
        "a change reloads the plugin"
    );
    assert_eq!(
        token(&reloaded.config_toml).0.as_deref(),
        Some("from-admin")
    );
    let settings = registry.settings(&updated.manifest).await.expect("view");
    let source = |key: &str| settings.iter().find(|s| s.key == key).unwrap().source;
    assert_eq!(source("token"), SettingSource::Admin);
    assert_eq!(source("retries"), SettingSource::Default);
    assert_eq!(source("password"), SettingSource::Unset);

    watcher::sync_once(&registry, &bus, &search_path).await;
    let after_sync = registry.get_plugin(&id).expect("loaded");
    assert!(
        Arc::ptr_eq(&reloaded, &after_sync),
        "a sync with nothing changed keeps the admin's config"
    );

    registry
        .update_settings(&id, vec![set("password", Some("hunter2"))], "admin")
        .await
        .expect("set a secret");
    let after_secret = registry.get_plugin(&id).expect("loaded");
    assert!(
        !Arc::ptr_eq(&after_sync, &after_secret),
        "a secret reloads the plugin too, which may have cached it"
    );
    assert!(!after_secret.config_toml.contains("hunter2"));
    let settings = registry
        .settings(&after_secret.manifest)
        .await
        .expect("view");
    let password = settings.iter().find(|s| s.key == "password").unwrap();
    assert_eq!(
        (password.source, password.value.as_deref()),
        (SettingSource::Admin, None)
    );

    let err = registry
        .update_settings(
            &id,
            vec![set("token", None), set("retries", Some("lots"))],
            "admin",
        )
        .await
        .expect_err("a bad value refuses the whole change");
    assert!(matches!(err, SettingsError::Invalid { .. }), "{err}");
    assert_eq!(
        token(&registry.get_plugin(&id).unwrap().config_toml)
            .0
            .as_deref(),
        Some("from-admin")
    );

    registry
        .update_settings(&id, vec![set("token", None)], "admin")
        .await
        .expect("clear");
    assert_eq!(
        token(&registry.get_plugin(&id).unwrap().config_toml)
            .0
            .as_deref(),
        Some("from-config"),
        "clearing falls back to config.toml"
    );
}
