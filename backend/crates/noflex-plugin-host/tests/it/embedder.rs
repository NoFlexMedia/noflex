// SPDX-License-Identifier: AGPL-3.0-only
//
// Integration tests for the embedder hook (WIT 0.6.0). The fixture guest
// (`tests/fixtures/embedder/`) produces deterministic, text-sensitive
// dim-8 vectors and reads `fixture-model.txt` through `host-assets`
// when given the sentinel text "asset-check". The suite proves: claim
// reporting, batch round-trips with shape validation, the no-embedder
// None path, capability-gated asset reads (with and without a
// configured assets dir), and concurrent calls on the one instance.
//
// Both fixtures are built on demand (cargo, wasm32-wasip2) when their
// artifacts are missing; the tests self-skip if the toolchain can't
// produce them (e.g. a host without the target installed).

use noflex_plugin_host::{
    EmbedInput, EngineConfig, HostError, PluginRegistry, build_engine, embed_inputs,
};

fn registry() -> PluginRegistry {
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    PluginRegistry::new(engine).expect("registry")
}

#[tokio::test]
async fn embedder_claim_returns_manifest_claim() {
    let Some(wasm) = crate::fixtures::wasm("embedder") else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    let registry = registry();
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    let claim = registry.embedder_claim().expect("embedder is loaded");
    assert_eq!(claim.namespace, "fixture-embed");
    assert_eq!(claim.model_id, "fixture/embed-v1");
    assert_eq!(claim.dimensions, 8);
}

#[tokio::test]
async fn embed_texts_round_trip_is_deterministic() {
    let Some(wasm) = crate::fixtures::wasm("embedder") else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    let registry = registry();
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    let texts = vec![
        "the wire".to_string(),
        "a completely different title".to_string(),
        "the wire".to_string(),
    ];
    let first = embed_texts(&registry, texts.clone())
        .await
        .expect("dispatch succeeds")
        .expect("embedder is loaded");
    assert_eq!(first.namespace, "fixture-embed");
    assert_eq!(first.model_id, "fixture/embed-v1");
    assert_eq!(first.vectors.len(), 3, "one vector per input text");
    for v in &first.vectors {
        assert_eq!(v.len(), 8, "vectors have the claimed dimensionality");
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
    }
    // Same input in, same vector out — and repeated texts in one batch
    // agree with each other.
    assert_eq!(first.vectors[0], first.vectors[2]);
    assert_ne!(first.vectors[0], first.vectors[1]);

    let second = embed_texts(&registry, texts)
        .await
        .expect("dispatch succeeds")
        .expect("embedder is loaded");
    assert_eq!(first.vectors, second.vectors, "deterministic across calls");
}

#[tokio::test]
async fn embed_texts_without_embedder_returns_none() {
    let Some(wasm) = crate::fixtures::wasm("concurrency") else {
        eprintln!("skipping: could not build the concurrency fixture plugin");
        return;
    };
    let registry = registry();
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    assert!(registry.embedder_claim().is_none());
    let outcome = embed_texts(&registry, vec!["anything".to_string()])
        .await
        .expect("no embedder is not an error");
    assert!(outcome.is_none());

    // Empty input short-circuits the same way even with no plugins.
    let outcome = embed_texts(&registry, Vec::new())
        .await
        .expect("empty batch is not an error");
    assert!(outcome.is_none());
}

#[tokio::test]
async fn embed_texts_rejects_oversized_batch() {
    let registry = registry();
    let texts = (0..65).map(|i| format!("text-{i}")).collect();
    let err = embed_texts(&registry, texts)
        .await
        .expect_err("65 inputs exceeds the 64-input cap");
    assert!(err.to_string().contains("64-input cap"), "msg: {err}");
}

#[tokio::test]
async fn asset_read_round_trips_through_host_assets() {
    let Some(wasm) = crate::fixtures::wasm("embedder") else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let model = b"fixture model weights (not really)";
    std::fs::write(dir.path().join("fixture-model.txt"), model).expect("write asset");

    let registry = registry().with_assets_dir(Some(dir.path().to_path_buf()));
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    let outcome = embed_texts(&registry, vec!["asset-check".to_string()])
        .await
        .expect("dispatch succeeds")
        .expect("embedder is loaded");
    assert_eq!(
        outcome.vectors[0][0],
        model.len() as f32,
        "vector[0] carries the asset's byte length"
    );
}

#[tokio::test]
async fn asset_read_without_assets_dir_surfaces_error() {
    let Some(wasm) = crate::fixtures::wasm("embedder") else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    // No assets dir configured: the guest's read-file fails and the
    // plugin error propagates out of embed_texts.
    let registry = registry();
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    let err = embed_texts(&registry, vec!["asset-check".to_string()])
        .await
        .expect_err("read-file without an assets dir must fail");
    assert!(
        err.to_string().contains("asset store not configured"),
        "msg: {err}"
    );
}

#[tokio::test]
async fn concurrent_embed_calls_complete_on_one_instance() {
    let Some(wasm) = crate::fixtures::wasm("embedder") else {
        eprintln!("skipping: could not build the fixture-embedder plugin");
        return;
    };
    let registry = registry();
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture plugin: {e}"));

    let calls: Vec<_> = (0..4)
        .map(|i| {
            let texts = vec![format!("concurrent-{i}")];
            embed_texts(&registry, texts)
        })
        .collect();
    let results = futures::future::join_all(calls).await;
    let mut seen = Vec::new();
    for outcome in results {
        let outcome = outcome
            .expect("dispatch succeeds")
            .expect("embedder is loaded");
        assert_eq!(outcome.vectors.len(), 1);
        seen.push(outcome.vectors[0].clone());
    }
    for w in seen.windows(2) {
        assert_ne!(w[0], w[1], "each caller got its own text's vector");
    }
}

async fn embed_texts(
    registry: &PluginRegistry,
    texts: Vec<String>,
) -> Result<Option<noflex_plugin_host::EmbedOutcome>, HostError> {
    embed_inputs(registry, texts.into_iter().map(EmbedInput::Query).collect()).await
}
