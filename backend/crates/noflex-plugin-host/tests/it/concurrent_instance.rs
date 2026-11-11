// SPDX-License-Identifier: AGPL-3.0-only
//
// Proves the one-instance dispatch invariant: N concurrent hook calls on a
// single loaded plugin genuinely overlap inside the guest, and each caller
// gets its own response. The fixture guest (`tests/fixtures/concurrency/`) records the
// in-flight high-water mark in instance-local state and reports it back,
// so a host that secretly re-instantiated per call would be caught (the
// mark could never exceed 1).
//
// Also exercises `image.fetch`'s `stream<u8>` ingestion: the fixture
// streams a fixed payload and the host must deliver it as one contiguous
// buffer.
//
// The fixture component is built on demand (cargo, wasm32-wasip2) when the
// artifact is missing; the test self-skips if the toolchain can't produce
// it (e.g. a host without the target installed).

use std::{collections::HashMap, sync::Arc};

use noflex_plugin_host::{
    EngineConfig, LifecycleBus, PluginId, PluginRegistry, SearchPath, build_engine,
    metadata_and_image, metadata_fetch, plugin_types, watcher,
};

const CONCURRENCY: usize = 8;

fn query(title: &str) -> plugin_types::SearchQuery {
    plugin_types::SearchQuery {
        title: title.to_string(),
        year: None,
        kind: plugin_types::WorkKind::Movie,
        hints: vec![],
    }
}

#[tokio::test]
async fn hot_reload_reuses_unchanged_plugin_and_retains_config() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let installed = dir.path().join("fixture.wasm");
    std::fs::copy(wasm, &installed).unwrap();
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let search_path = SearchPath::from_explicit([dir.path()]);
    let bus = LifecycleBus::default();
    let configs = HashMap::from([(
        "concurrency-fixture".to_string(),
        "token = \"real-config\"".to_string(),
    )]);

    watcher::sync_once_with_config(&registry, &bus, &search_path, &configs);
    let id = PluginId("concurrency-fixture".into());
    let first = registry.get_plugin(&id).expect("fixture loaded");
    assert_eq!(first.config_toml, "token = \"real-config\"");

    watcher::sync_once_with_config(&registry, &bus, &search_path, &configs);
    let second = registry.get_plugin(&id).expect("fixture remains loaded");
    assert!(Arc::ptr_eq(&first, &second));

    std::fs::write(&installed, b"not wasm").unwrap();
    watcher::sync_once_with_config(&registry, &bus, &search_path, &configs);
    let after_failed_change = registry.get_plugin(&id).expect("old plugin retained");
    assert!(Arc::ptr_eq(&first, &after_failed_change));
}

#[tokio::test]
async fn a_disabled_plugin_never_loads() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(&wasm, dir.path().join("fixture.wasm")).unwrap();
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_disabled(["concurrency-fixture".to_string()]);
    let bus = LifecycleBus::default();
    let mut events = bus.subscribe();

    let search_path = SearchPath::from_explicit([dir.path()]);
    watcher::sync_once_with_config(&registry, &bus, &search_path, &HashMap::new());
    assert!(registry.snapshot().is_empty());
    assert!(
        events.try_recv().is_err(),
        "a disabled plugin is no load failure"
    );
    assert!(matches!(
        registry.load_from(&wasm),
        Err(noflex_plugin_host::HostError::Disabled { .. })
    ));
}

#[tokio::test]
async fn concurrent_search_calls_overlap_on_one_instance() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let plugin = registry
        .get_plugin(&PluginId("concurrency-fixture".to_string()))
        .expect("fixture registered");

    let mut handles = Vec::new();
    for i in 0..CONCURRENCY {
        let plugin = Arc::clone(&plugin);
        handles.push(tokio::spawn(async move {
            let title = format!("probe-{i}");
            let result = plugin
                .call_search(query(&title))
                .await
                .expect("dispatch succeeds")
                .expect("fixture search succeeds");
            (title, result)
        }));
    }

    let mut max_in_flight = 0u32;
    for h in handles {
        let (title, matches) = h.await.expect("call task joins");
        assert_eq!(matches.len(), 1, "fixture returns exactly one match");
        assert_eq!(
            matches[0].title, title,
            "each caller received its own response"
        );
        let marker = matches[0].external_id.value.clone();
        let n: u32 = marker
            .strip_prefix("max-in-flight:")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("fixture marker malformed: {marker}"));
        max_in_flight = max_in_flight.max(n);
    }

    assert!(
        max_in_flight >= 2,
        "expected calls to overlap on the single instance, high-water mark was {max_in_flight}"
    );
}

#[tokio::test]
async fn image_fetch_stream_is_buffered_to_completion() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let plugin = registry
        .get_plugin(&PluginId("concurrency-fixture".to_string()))
        .expect("fixture registered");

    let result = plugin
        .call_fetch_image(
            metadata_and_image::noflex::plugin::types::ExternalId {
                kind: metadata_and_image::noflex::plugin::types::ExternalIdKind::Custom,
                value: "fixture:1".to_string(),
            },
            metadata_and_image::noflex::plugin::types::ImageKind::Primary,
            None,
        )
        .await
        .expect("dispatch succeeds")
        .expect("fixture image.fetch succeeds");

    let (meta, bytes) = result;
    assert_eq!(meta.mime, "application/octet-stream");
    assert_eq!(bytes, b"noflex-fixture-image-bytes");
}

#[tokio::test]
async fn metadata_fetch_returns_payload_from_one_instance() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let plugin = registry
        .get_plugin(&PluginId("concurrency-fixture".to_string()))
        .expect("fixture registered");

    let payload = plugin
        .call_fetch(plugin_types::ExternalId {
            kind: plugin_types::ExternalIdKind::Custom,
            value: "fixture:42".to_string(),
        })
        .await
        .expect("dispatch succeeds")
        .expect("fixture metadata.fetch succeeds");

    assert_eq!(payload.title, "Fixture Title");
    assert_eq!(payload.genres, vec!["Drama", "Fixture"]);
    assert_eq!(payload.year, Some(2001));
    assert_eq!(payload.runtime_ms, Some(5_420_000));
    assert_eq!(payload.external_ids.len(), 1);
    assert_eq!(payload.external_ids[0].value, "fixture:42");
    assert!(matches!(
        payload.external_ids[0].kind,
        plugin_types::ExternalIdKind::Custom
    ));
}

#[tokio::test]
async fn registry_metadata_fetch_walks_candidates() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    let external_ids = vec![noflex_core::model::ExternalId {
        kind: noflex_core::model::ExternalIdKind::Custom,
        value: "fixture:7".to_string(),
    }];

    // A candidate that isn't loaded must be skipped, not fail the walk.
    let candidates = vec![
        PluginId("not-loaded".to_string()),
        PluginId("concurrency-fixture".to_string()),
    ];
    let (id, payload) = metadata_fetch(&registry, &candidates, &external_ids)
        .await
        .expect("fixture answers");
    assert_eq!(id.0, "concurrency-fixture");
    assert_eq!(payload.title, "Fixture Title");
    assert_eq!(payload.genres, vec!["Drama", "Fixture"]);
    assert_eq!(payload.external_ids.len(), 1);
    assert_eq!(payload.external_ids[0].value, "fixture:7");
    assert_eq!(
        payload.external_ids[0].kind,
        noflex_core::model::ExternalIdKind::Custom
    );
    assert_eq!(payload.images.len(), 1);
    assert_eq!(
        payload.images[0].kind,
        noflex_core::model::ImageKind::Primary
    );

    assert!(
        metadata_fetch(&registry, &[], &external_ids)
            .await
            .is_none(),
        "no candidates means no payload"
    );
}

/// A deadline tight enough that the old store-lifetime epoch deadline
/// (armed once at instantiation) would expire between two calls.
fn tight_deadline_config() -> EngineConfig {
    EngineConfig {
        default_epoch_deadline_secs: 1,
        epoch_ticker_period_ms: 1,
        ..EngineConfig::default()
    }
}

#[tokio::test]
async fn calls_still_succeed_after_the_deadline_window_has_elapsed() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let config = tight_deadline_config();
    let engine = build_engine(&config).expect("engine builds");
    let registry = PluginRegistry::new_with_config(Arc::clone(&engine), config).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));
    let plugin = registry
        .get_plugin(&PluginId("concurrency-fixture".to_string()))
        .expect("fixture registered");

    plugin
        .call_search(query("before"))
        .await
        .expect("dispatch succeeds")
        .expect("fixture search succeeds");
    // Ten deadline windows' worth of ticks, without sleeping through them.
    for _ in 0..10_000 {
        engine.increment_epoch();
    }
    let matches = plugin
        .call_search(query("after"))
        .await
        .expect("a call after the window must not trap the long-lived instance")
        .expect("fixture search succeeds");
    assert_eq!(matches[0].title, "after");
    assert_eq!(plugin.health.read().unwrap().instance_restarts, 0);
}
