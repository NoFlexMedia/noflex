// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end smoke test: load the actual first-party TMDb `.wasm` component
// (built by `cargo build --release --target wasm32-wasip2` in
// `plugins/noflex-metadata-tmdb/`) and confirm the host can:
//
//   1. Find the embedded `noflex:manifest` custom section,
//   2. Parse it into [`PluginManifest`],
//   3. Instantiate the [`wasmtime::Component`] without errors,
//   4. Surface the right hooks in the registry snapshot.
//
// The test is gated on the plugin artifact existing on disk so it doesn't
// fail in environments that haven't built the plugin yet (e.g. cold CI
// without the `wasm32-wasip2` target preinstalled). A separate CI job
// builds the plugins first, then runs this test.

use std::path::PathBuf;

use noflex_plugin_host::{
    EmbedDocument, EmbedInput, EngineConfig, HookKind, PluginId, PluginRegistry, build_engine,
    plugin_types,
};

fn plugin_wasm_path(crate_name: &str, plugin_id: &str) -> Option<PathBuf> {
    // tests/ runs with CARGO_MANIFEST_DIR = crates/noflex-plugin-host/.
    // Walk up to the workspace root, then down into plugins/.
    let _ = plugin_id; // kept for the file naming convention
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent()?.parent()?;
    let p = workspace_root.join(format!(
        "plugins/{crate_name}/target/wasm32-wasip2/release/{}.wasm",
        crate_name.replace('-', "_")
    ));
    p.is_file().then_some(p)
}

#[tokio::test]
async fn loads_real_tmdb_plugin() {
    let Some(wasm) = plugin_wasm_path("noflex-metadata-tmdb", "tmdb") else {
        eprintln!(
            "skipping: build the plugin first with `nix develop --command cargo xtask plugin-build`"
        );
        return;
    };

    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let info = registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load tmdb plugin: {e}"));

    assert_eq!(info.id, "tmdb", "plugin_id from embedded manifest");
    assert_eq!(info.display_name, "The Movie Database");
    assert_eq!(info.api_version, "0.2");
    assert!(
        info.hooks.contains(&HookKind::Metadata),
        "exports metadata hook"
    );
    assert!(info.hooks.contains(&HookKind::Image), "exports image hook");
    assert!(!info.binary_sha256.is_empty(), "binary sha256 was computed");

    // Registry snapshot returns the plugin.
    let snap = registry.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].id, "tmdb");
}

#[tokio::test]
async fn loads_real_tvdb_plugin() {
    let Some(wasm) = plugin_wasm_path("noflex-metadata-tvdb", "tvdb") else {
        eprintln!("skipping: build the plugin first with `cargo xtask plugin-build`");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let info = registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load tvdb plugin: {e}"));
    assert_eq!(info.id, "tvdb");
    assert!(info.hooks.contains(&HookKind::Metadata));
}

/// Trakt imports `host-user-state` beside its world, so this is the check
/// that the linker serves the write imports to a real guest.
#[tokio::test]
async fn loads_real_trakt_plugin() {
    let Some(wasm) = plugin_wasm_path("noflex-trakt", "trakt") else {
        eprintln!("skipping: build the plugin first with `cargo xtask plugin-build`");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let info = registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load trakt plugin: {e}"));
    assert_eq!(info.id, "trakt");
    assert!(info.hooks.contains(&HookKind::Scrobbler));
    let plugin = registry
        .get_plugin(&PluginId("trakt".into()))
        .expect("registered");
    assert!(
        plugin
            .manifest
            .tasks
            .iter()
            .any(|task| task.name == "sync" && task.per_user)
    );
    // An unbound run acts for nobody, so it does nothing rather than fail.
    let outcome = plugin
        .call_run_task("sync".into(), None, Some(uuid::Uuid::now_v7().to_string()))
        .await
        .expect("dispatch");
    assert!(
        matches!(outcome, Ok(noflex_plugin_host::TaskOutcome::Done)),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn opensubtitles_without_a_key_says_how_to_set_one() {
    let Some(wasm) = plugin_wasm_path("noflex-subtitles-opensubtitles", "opensubtitles") else {
        eprintln!("skipping: build the plugin first with `cargo xtask plugin-build opensubtitles`");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let info = registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load opensubtitles plugin: {e}"));
    assert_eq!(info.id, "opensubtitles");
    assert!(info.hooks.contains(&HookKind::Subtitle));

    let search = registry
        .search_subtitles(&noflex_core::providers::SubtitleQuery {
            moviehash: None,
            file_size: 0,
            file_name: "Alien.mkv".into(),
            title: "Alien".into(),
            year: Some(1979),
            kind: noflex_core::model::WorkKind::Movie,
            external_ids: vec![],
            series_external_ids: vec![],
            season: None,
            episode: None,
            languages: vec!["eng".into()],
        })
        .await;
    assert!(search.matches.is_empty());
    assert!(
        matches!(
            &search.failures[..],
            [failure] if matches!(
                &failure.error,
                noflex_plugin_host::ProviderError::Unavailable(why) if why.contains("API key")
            )
        ),
        "{:?}",
        search.failures
    );
}

#[tokio::test]
async fn loads_real_musicbrainz_plugin() {
    let Some(wasm) = plugin_wasm_path("noflex-metadata-musicbrainz", "musicbrainz") else {
        eprintln!("skipping: build the plugin first with `cargo xtask plugin-build`");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let info = registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load musicbrainz plugin: {e}"));
    assert_eq!(info.id, "musicbrainz");
    assert!(info.hooks.contains(&HookKind::Metadata));

    // The plugin imports `wasi:clocks@0.3.0` for request pacing; a call that
    // fails fast (unsupported kind, no network) proves the host links it.
    let plugin = registry
        .get_plugin(&PluginId("musicbrainz".into()))
        .expect("registered");
    let outcome = plugin
        .call_search(plugin_types::SearchQuery {
            title: "Heat".into(),
            year: None,
            kind: plugin_types::WorkKind::Movie,
            hints: vec![],
        })
        .await
        .expect("instance instantiates with WASI 0.3 clocks linked");
    assert!(
        matches!(outcome, Err(plugin_types::PluginError::BadInput(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn loads_real_omdb_plugin() {
    use noflex_plugin_host::ratings::noflex::plugin::types as wit;

    let Some(wasm) = plugin_wasm_path("noflex-ratings-omdb", "omdb") else {
        eprintln!("skipping: build the plugin first with `cargo xtask plugin-build`");
        return;
    };
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let info = registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load omdb plugin: {e}"));
    assert_eq!(info.id, "omdb");
    assert_eq!(info.hooks, vec![HookKind::Ratings]);
    let plugin = registry
        .get_plugin(&PluginId("omdb".into()))
        .expect("registered");
    let attribution = plugin
        .call_attribution()
        .await
        .expect("answers through its ratings export")
        .expect("asks for credit");
    assert!(
        attribution.text.contains("CC BY-NC 4.0"),
        "{}",
        attribution.text
    );

    let imdb = |value: &str| wit::ExternalId {
        kind: wit::ExternalIdKind::Imdb,
        value: value.into(),
    };
    // Nothing to look up asks for nothing, key or no key.
    let tmdb_only = vec![wit::ExternalId {
        kind: wit::ExternalIdKind::Tmdb,
        value: "movie/603".into(),
    }];
    let outcome = plugin
        .call_ratings(tmdb_only, wit::WorkKind::Movie)
        .await
        .expect("dispatch");
    assert!(matches!(outcome, Ok(ref r) if r.is_empty()), "{outcome:?}");
    let outcome = plugin
        .call_ratings(vec![imdb("tt0944947")], wit::WorkKind::Episode)
        .await
        .expect("dispatch");
    assert!(
        matches!(outcome, Ok(ref r) if r.is_empty()),
        "episodes only with the setting: {outcome:?}"
    );
    let outcome = plugin
        .call_ratings(vec![imdb("tt0133093")], wit::WorkKind::Movie)
        .await
        .expect("dispatch");
    assert!(
        matches!(outcome, Err(wit::PluginError::Unavailable(ref m)) if m.contains("no API key")),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn loads_all_three_first_party_plugins_into_a_single_registry() {
    let tmdb = plugin_wasm_path("noflex-metadata-tmdb", "tmdb");
    let tvdb = plugin_wasm_path("noflex-metadata-tvdb", "tvdb");
    let mb = plugin_wasm_path("noflex-metadata-musicbrainz", "musicbrainz");
    if tmdb.is_none() || tvdb.is_none() || mb.is_none() {
        eprintln!("skipping: one or more plugin artifacts missing");
        return;
    }
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry.load_from(&tmdb.unwrap()).expect("tmdb");
    registry.load_from(&tvdb.unwrap()).expect("tvdb");
    registry.load_from(&mb.unwrap()).expect("musicbrainz");
    let snap = registry.snapshot();
    assert_eq!(snap.len(), 3, "all three plugins registered");
    let mut ids: Vec<&str> = snap.iter().map(|p| p.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["musicbrainz", "tmdb", "tvdb"]);
    // exporters_of(metadata) should now return all three ids.
    let mut meta = registry.exporters_of(HookKind::Metadata);
    meta.sort_by(|a, b| a.0.cmp(&b.0));
    let meta_ids: Vec<&str> = meta.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(meta_ids, vec!["musicbrainz", "tmdb", "tvdb"]);
}

/// End-to-end dispatch from the host into the real plugin. Exercises:
///   1. Host-side `wasmtime::component::bindgen!` lift of `SearchQuery`,
///   2. cabi_realloc on the guest to allocate argument memory,
///   3. The plugin's `metadata::search` Guest impl,
///   4. Lower of the returned `Vec<MetadataMatch>` back to the host.
///
/// WASI 0.3 async-native stack: the export is an `async func` dispatched
/// through the plugin's single instance inside `Store::run_concurrent`,
/// every `wit-bindgen` crate in the dep tree unified to a single rlib
/// (the silent cause of the earlier cabi_realloc trap).
#[tokio::test]
async fn dispatches_search_into_real_plugin() {
    let Some(wasm) = plugin_wasm_path("noflex-metadata-tmdb", "tmdb") else {
        eprintln!(
            "skipping: build the plugin first with `nix develop --command cargo xtask plugin-build`"
        );
        return;
    };

    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load tmdb plugin: {e}"));

    let plugin = registry
        .get_plugin(&PluginId("tmdb".to_string()))
        .expect("plugin registered");

    // A season search resolves from the host's hints alone (no secret,
    // no network), so the full lift/lower round trip is observable.
    let season = plugin
        .call_search(plugin_types::SearchQuery {
            title: "Season 2".to_string(),
            year: None,
            kind: plugin_types::WorkKind::Season,
            hints: vec![
                "parent-external-id:tmdb:tv/1399".to_string(),
                "season:2".to_string(),
            ],
        })
        .await
        .expect("dispatch succeeds")
        .expect("hint-derived match");
    assert_eq!(season.len(), 1);
    assert_eq!(
        season[0].external_id.kind,
        plugin_types::ExternalIdKind::Tmdb
    );
    assert_eq!(season[0].external_id.value, "tv/1399/season/2");
    assert_eq!(season[0].confidence, 1.0);
}

/// Same smoke coverage for the local-embedding plugin: artifact loads,
/// the `[embedding]` manifest claim parses, and the embedder hook is
/// exported. The live-embed assertion is gated on the real model files
/// being provisioned (they never are in CI — no test downloads the
/// ~300MB weights); point `NOFLEX_EMBEDDINGS_TEST_ASSETS` at a directory
/// containing `embeddinggemma-2/{model_text.onnx,model.onnx_data,tokenizer.json}`
/// to run it locally.
#[tokio::test]
async fn loads_real_embeddings_plugin() {
    let Some(wasm) = plugin_wasm_path("noflex-embeddings", "embeddings") else {
        eprintln!(
            "skipping: build the plugin first with `cargo xtask plugin-build noflex-embeddings`"
        );
        return;
    };

    let assets = std::env::var_os("NOFLEX_EMBEDDINGS_TEST_ASSETS").map(PathBuf::from);
    let provisioned = assets.as_ref().is_some_and(|dir| {
        ["model_text.onnx", "model.onnx_data", "tokenizer.json"]
            .iter()
            .all(|f| dir.join("embeddinggemma-2").join(f).is_file())
    });

    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine)
        .expect("registry")
        .with_assets_dir(assets.clone());
    let info = registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load embeddings plugin: {e}"));

    assert_eq!(info.id, "embeddings", "plugin_id from embedded manifest");
    assert_eq!(info.api_version, "0.2");
    assert!(
        info.hooks.contains(&HookKind::Embedder),
        "exports embedder hook"
    );

    // The claim parses (load_from rejects an [embedding] table that
    // doesn't pair with the embedder hook) and matches the model card.
    let claim = info
        .manifest
        .embedding
        .as_ref()
        .expect("[embedding] table parses");
    assert_eq!(claim.namespace, "embeddinggemma-2-512");
    assert_eq!(claim.model_id, "google/embeddinggemma-2");
    assert_eq!(claim.dimensions, 512);

    if !provisioned {
        eprintln!(
            "skipping live-embed: model files not provisioned \
             (set NOFLEX_EMBEDDINGS_TEST_ASSETS to the plugin assets dir)"
        );
        return;
    }

    let plugin = registry
        .get_plugin(&PluginId("embeddings".to_string()))
        .expect("plugin registered");
    let document = |title: &str, text: &str| {
        EmbedInput::Document(EmbedDocument {
            title: title.to_string(),
            text: text.to_string(),
        })
    };
    let vectors = plugin
        .call_embed(vec![
            EmbedInput::Query("an astronaut stranded on another planet".to_string()),
            document(
                "The Martian (2015)",
                "A botanist is left behind on Mars and must survive.",
            ),
            document(
                "When Harry Met Sally (1989)",
                "Two friends wonder if men and women can just be friends.",
            ),
            EmbedInput::Query("The Martian (2015)".to_string()),
        ])
        .await
        .expect("dispatch succeeds")
        .expect("embed returns Ok with provisioned model");
    assert_eq!(vectors.len(), 4, "one vector per input");
    let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f32>();
    for vector in &vectors {
        assert_eq!(vector.len(), 512, "manifest-claimed dimensionality");
        assert!((dot(vector, vector) - 1.0).abs() < 1e-3, "unit length");
    }
    assert!(
        dot(&vectors[0], &vectors[1]) > dot(&vectors[0], &vectors[2]) + 0.05,
        "the query lands nearer the film about it"
    );
    assert!(
        dot(&vectors[1], &vectors[3]) < 0.99,
        "documents and queries are prompted differently"
    );
}
