// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex server binary wires up:
//   - tracing
//   - default Turso storage backend
//   - the canonical GraphQL endpoint (`/graphql`)
//
// Feature work lands incrementally on top of this scaffold.

// The merged GraphQL Query root's resolver future is laid out past
// rustc's default query depth in release builds of this crate.
#![recursion_limit = "256"]

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Context;
use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    Router,
    extract::State,
    middleware,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use noflex_auth::{JwtConfig, auth_middleware};
use noflex_graphql::{
    ExportEventBroadcaster, MetadataSearch, NoflexSchema, SchemaContext, build_schema,
};
use noflex_plugin_host::{
    EngineConfig, LifecycleBus, PluginRegistry, SearchPath, build_engine, spawn_epoch_ticker,
    watcher as plugin_watcher,
};
use noflex_scanner::{FilesystemScanner, ScanRegistry};
use noflex_server::{
    config::{Config, PluginConfig},
    download::DownloadWorker,
    download_serve::serve_download_job,
    file_serve::RootCapability,
    stream,
};
use noflex_storage::Storage;
use noflex_storage_turso::TursoStore;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Clone)]
struct AppState {
    storage: Arc<dyn Storage>,
    schema: NoflexSchema,
    jwt: Arc<JwtConfig>,
    plugin_registry: Arc<PluginRegistry>,
    metrics: Arc<noflex_server::metrics::Metrics>,
    /// Broadcast channel of export-job status events consumed by the GQL
    /// subscription.
    #[allow(dead_code)]
    export_events: Arc<ExportEventBroadcaster>,
    /// Background download worker. Held for the lifetime of the server.
    #[allow(dead_code)]
    download_worker: Arc<DownloadWorker>,
    download_root: RootCapability,
    /// Per-user plugin link flow; the callback route calls its
    /// concrete `complete_link`.
    plugin_links: Arc<noflex_server::plugin_links::ServerPluginLinks>,
    external_auth: Arc<noflex_server::external_auth::ServerExternalAuth>,
}

/// Worker threads' stack. Executing a GraphQL request polls the merged schema
/// root's future, which grows with every resolver and sits close to tokio's
/// 2 MiB default in debug builds. Untouched pages cost nothing.
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

/// How long a blocking task can hold up exit once asked to stop.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(WORKER_STACK_BYTES)
        .build()
        .context("start runtime")?;
    let result = runtime.block_on(run());
    // Drops every task, and with them the ffmpeg each one is watching.
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    result
}

/// Resolves when the process is asked to stop. Without a handler those signals
/// end it on the spot, leaving its ffmpeg children running.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = terminate.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(error) => {
                tracing::warn!(%error, "can't listen for SIGTERM");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

async fn run() -> anyhow::Result<()> {
    let cfg = Config::load().context("load config")?;
    let log_buffer = init_tracing(&cfg.log);

    std::fs::create_dir_all(&cfg.data_dir)
        .with_context(|| format!("create data dir {}", cfg.data_dir.display()))?;
    // Cache directories. An empty `dir` field falls back to
    // `<cache_dir>/<name>`. Three separate dirs by purpose so a
    // deployer can put the transcode cache on a fast scratch
    // disk without putting the image cache next to it.
    for (name, configured) in [
        ("transcode", &cfg.cache.transcode_dir),
        ("image", &cfg.cache.image_dir),
        ("embed", &cfg.cache.embed_dir),
    ] {
        let path = if configured.is_empty() {
            cfg.cache_dir.join(name)
        } else {
            std::path::PathBuf::from(configured)
        };
        if let Err(e) = std::fs::create_dir_all(&path) {
            tracing::warn!(error = %e, path = %path.display(), "cache dir create failed");
        }
    }
    let db_path = cfg.data_dir.join("noflex.db");

    let store = TursoStore::open(&db_path)
        .await
        .context("open turso store")?;
    store.migrate().await.context("run migrations")?;
    let storage: Arc<dyn Storage> = Arc::new(store);
    let jwt: Arc<JwtConfig> =
        Arc::new(JwtConfig::for_server(&cfg.data_dir).context("load or create JWT signing key")?);

    // Bring up the plugin runtime. Loads any `.wasm` plugins from the
    // tiered search path; missing directories are tolerated. The epoch
    // ticker enforces per-call deadlines during dispatch.
    let plugin_engine_cfg = EngineConfig {
        inference: Arc::new(noflex_plugin_host::Inference::new(
            noflex_plugin_host::InferenceConfig {
                device: cfg.search.device,
                threads: cfg.search.threads,
                library: cfg.search.onnxruntime_library.clone(),
            },
        )),
        ..EngineConfig::default()
    };
    let engine =
        build_engine(&plugin_engine_cfg).map_err(|e| anyhow::anyhow!("build wasm engine: {e}"))?;
    let _epoch_handle =
        spawn_epoch_ticker(engine.clone(), plugin_engine_cfg.epoch_ticker_period_ms);
    let lifecycle_bus = LifecycleBus::default();
    let event_bus = noflex_core::DomainEventBus::default();
    // Subscribes before the plugin watcher and library watchers start,
    // so boot-time lifecycle and scan events reach the admin view.
    let metrics = noflex_server::metrics::Metrics::new();
    let status_recorder = noflex_server::system_status::Recorder::spawn(
        &lifecycle_bus,
        &event_bus,
        storage.clone(),
        log_buffer.clone(),
        metrics.clone(),
    );
    let plugin_registry = Arc::new(
        PluginRegistry::new_with_config(engine.clone(), plugin_engine_cfg.clone())
            .map_err(|e| anyhow::anyhow!("init plugin registry: {e}"))?
            // Storage for `host-library` lookups and background-task state.
            .with_storage(storage.clone())
            // Assets root for `host-assets.read-file` (model weights).
            .with_assets_dir(cfg.plugin.assets_dir.clone())
            // Fan domain events out to notifier plugins.
            .with_event_bus(event_bus.clone()),
    );
    let plugin_links = Arc::new(noflex_server::plugin_links::ServerPluginLinks::new(
        plugin_registry.clone(),
        storage.clone(),
    ));
    let external_auth = Arc::new(noflex_server::external_auth::ServerExternalAuth::new(
        plugin_registry.clone(),
        storage.clone(),
        cfg.public_url.clone(),
        cfg.external_auth.auto_provision,
    ));
    let search_path = if cfg.plugin.search_path.is_empty() {
        SearchPath::default_tiers()
    } else {
        SearchPath::from_explicit(cfg.plugin.search_path.iter().cloned())
    };
    let _plugin_watcher = plugin_watcher::start(
        plugin_registry.clone(),
        lifecycle_bus.clone(),
        search_path.clone(),
        per_plugin_config_toml(&cfg.plugin.per_plugin_config),
    )
    .context("start plugin watcher")?;

    // An administrator's embeddings API wins over any plugin, from the first sweep.
    match storage.embedding_endpoint().await {
        Ok(Some(endpoint)) => match noflex_plugin_host::RemoteEmbedder::new(endpoint) {
            Ok(remote) => plugin_registry.set_remote_embedder(Some(Arc::new(remote))),
            Err(e) => tracing::warn!(error = %e, "embeddings API unusable; using embedder plugins"),
        },
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "reading the embeddings API setting failed"),
    }

    // Host-driven embedding worker: backfills the vector index on boot, on scan
    // completion, when the embedder changes, and hourly, while the host is idle.
    let _embedding_worker = noflex_plugin_host::spawn_embedding_worker(
        plugin_registry.clone(),
        storage.clone(),
        &event_bus,
        noflex_plugin_host::embeddings::EmbeddingWorkerConfig {
            batch_size: cfg.search.batch_size,
            busy_percent: cfg.search.busy_percent,
            ..Default::default()
        },
    );
    match plugin_registry.embedder_claim() {
        Some(claim) => info!(
            namespace = %claim.namespace,
            model = %claim.model_id,
            "embedder plugin active; embedding worker started"
        ),
        None => info!("no embedder plugin loaded; embedding worker idles until one appears"),
    }

    // Scrobble fan-out (playback events -> scrobbler plugins, per-user
    // linked accounts only) and the intro-detection sweep (startup,
    // scan-completed, hourly). Both re-check for their plugins on every
    // trigger, so hot-reload installs pick up work without a restart.
    let _scrobble_dispatcher = noflex_plugin_host::spawn_scrobble_dispatcher(
        plugin_registry.clone(),
        storage.clone(),
        &event_bus,
    );
    let _intro_detector_worker = noflex_plugin_host::spawn_intro_detector_worker(
        plugin_registry.clone(),
        storage.clone(),
        &event_bus,
    );

    let scanner = Arc::new(FilesystemScanner::new(storage.clone()).with_events(event_bus.clone()));
    let scan_registry = Arc::new(ScanRegistry::new(scanner.clone()));
    // Auto-scan on filesystem change: one watcher per watch-enabled
    // library, kept in step with library mutations through the schema.
    let library_watchers = Arc::new(noflex_server::library_watchers::LibraryWatchers::new(
        storage.clone(),
        scanner.clone(),
        status_recorder.clone(),
    ));
    library_watchers.start_all().await;
    let _scan_scheduler = noflex_server::scan_scheduler::spawn(
        storage.clone(),
        scan_registry.clone(),
        status_recorder.clone(),
    );
    let priority_table = Arc::new(tokio::sync::RwLock::new({
        let plugins: Vec<(
            noflex_plugin_host::PluginId,
            noflex_plugin_host::PluginManifest,
        )> = plugin_registry
            .snapshot()
            .into_iter()
            .map(|p| (noflex_plugin_host::PluginId(p.id), p.manifest))
            .collect();
        let plugin_refs: Vec<(
            &noflex_plugin_host::PluginId,
            &noflex_plugin_host::PluginManifest,
        )> = plugins.iter().map(|(id, m)| (id, m)).collect();
        noflex_plugin_host::PriorityTable::build(plugin_refs, &build_host_overrides(&cfg.plugin))
    }));
    // Admin overrides from `setPluginOverride` win over manifest and
    // config priorities, and must survive restarts.
    match storage.list_plugin_overrides().await {
        Ok(rows) => {
            let mut table = priority_table.write().await;
            for row in rows {
                if let Err(error) =
                    table.set(&row.plugin_id, &row.hook, &row.work_kind, row.priority)
                {
                    tracing::warn!(plugin = %row.plugin_id, %error, "skipping stored plugin override");
                }
            }
        }
        Err(error) => tracing::warn!(%error, "loading stored plugin overrides failed"),
    }
    // Hot-loaded and removed plugins change who can be dispatched to;
    // without a rebuild a new plugin's manifest priorities never apply.
    {
        let table = priority_table.clone();
        let registry = plugin_registry.clone();
        let host_overrides = build_host_overrides(&cfg.plugin);
        let mut events = lifecycle_bus.subscribe();
        tokio::spawn(async move {
            use noflex_plugin_host::PluginLifecycleEvent as Event;
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match events.recv().await {
                    Ok(Event::Loaded { .. } | Event::Unloaded { .. })
                    | Err(RecvError::Lagged(_)) => {
                        let plugins: Vec<_> = registry
                            .snapshot()
                            .into_iter()
                            .map(|p| (noflex_plugin_host::PluginId(p.id), p.manifest))
                            .collect();
                        table
                            .write()
                            .await
                            .rebuild(plugins.iter().map(|(id, m)| (id, m)), &host_overrides);
                    }
                    Ok(_) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }
    let metadata = Arc::new(MetadataSearch::new(plugin_registry.clone()));
    let export_events = Arc::new(ExportEventBroadcaster::default());
    let ffmpeg_path = noflex_core::bundle::ffmpeg_tool("ffmpeg", "FFMPEG_BIN");
    // Transcode jobs write into the resolved transcode cache
    // dir (same resolution as the cache-dir bootstrap above).
    let transcode_dir = if cfg.cache.transcode_dir.is_empty() {
        cfg.cache_dir.join("transcode")
    } else {
        std::path::PathBuf::from(&cfg.cache.transcode_dir)
    };
    let transcoder = Arc::new(noflex_transcoder::FfmpegJobManager::new(
        ffmpeg_path.clone(),
        transcode_dir.clone(),
    ));
    // Image resize cache uses the same resolution rules as the transcode dir.
    let image_cache_dir = if cfg.cache.image_dir.is_empty() {
        cfg.cache_dir.join("image")
    } else {
        std::path::PathBuf::from(&cfg.cache.image_dir)
    };
    std::fs::create_dir_all(&image_cache_dir).ok();
    let trickplay_worker = Arc::new(noflex_server::trickplay::TrickplayWorker::new(
        storage.clone(),
        ffmpeg_path.clone(),
        image_cache_dir.clone(),
    ));
    let _trickplay_worker = trickplay_worker.spawn(&event_bus);
    let _episode_thumb_worker = Arc::new(noflex_server::episode_thumbs::EpisodeThumbWorker::new(
        storage.clone(),
        ffmpeg_path.clone(),
        image_cache_dir.clone(),
    ))
    .spawn(&event_bus);
    let _movie_art_worker = Arc::new(noflex_server::movie_art::MovieArtWorker::new(
        storage.clone(),
        ffmpeg_path.clone(),
        image_cache_dir.clone(),
    ))
    .spawn(&event_bus);
    // Provider metadata + artwork: refresh/identify through GraphQL, and
    // a worker that auto-identifies new works after scans and backfills
    // provider art for identified ones.
    let metadata_service = Arc::new(
        noflex_plugin_host::MetadataService::new(plugin_registry.clone(), storage.clone())
            .with_priority_table(priority_table.clone())
            .with_image_dir(image_cache_dir.clone()),
    );
    let _metadata_worker =
        noflex_plugin_host::spawn_metadata_worker(metadata_service.clone(), &event_bus);
    // Probe the local ffmpeg once at startup and resolve a single
    // hardware backend. Detection never fails startup; a missing or
    // limited ffmpeg just means we transcode in software.
    let hw_detect = noflex_transcoder::hwdetect::detect(&ffmpeg_path).await;
    let resolved_hwaccel = noflex_server::hwaccel::resolve_backend(
        cfg.transcoder.hwaccel,
        cfg.transcoder.fallback,
        &hw_detect,
    );
    let configured = noflex_server::hwaccel::configured_backend(cfg.transcoder.hwaccel);
    if configured.is_some() && configured != Some(resolved_hwaccel) {
        tracing::warn!(
            hwaccel = ?cfg.transcoder.hwaccel,
            fallback = ?cfg.transcoder.fallback,
            resolved = ?resolved_hwaccel,
            "configured hardware backend unavailable at runtime; fell back"
        );
    }
    tracing::info!(
        hwaccel = ?cfg.transcoder.hwaccel,
        fallback = ?cfg.transcoder.fallback,
        detected_hwaccels = ?hw_detect.hwaccels,
        detected_encoders = ?hw_detect.encoders,
        resolved = ?resolved_hwaccel,
        "transcoder hardware acceleration resolved"
    );
    // The session control plane lives behind GraphQL; it mints the
    // scoped media tokens the byte routes accept.
    // In ladder order, then anything smart transcoding wants on top.
    let mut wanted_codecs: Vec<noflex_core::VideoCodec> = Vec::new();
    let ladder_codecs = cfg.transcoder.ladder.iter().map(|rung| rung.video_codec);
    for codec in ladder_codecs.chain(cfg.transcoder.optimize.encode_video.iter().copied()) {
        if !wanted_codecs.contains(&codec) {
            wanted_codecs.push(codec);
        }
    }
    let usable_codecs = noflex_server::hwaccel::usable_codecs(
        &ffmpeg_path,
        resolved_hwaccel,
        &hw_detect,
        &wanted_codecs,
    )
    .await;
    tracing::info!(codecs = ?usable_codecs, "usable transcode codecs");
    let live_codecs = usable_codecs
        .iter()
        .copied()
        .filter(|codec| {
            cfg.transcoder
                .ladder
                .iter()
                .any(|rung| rung.video_codec == *codec)
        })
        .collect::<Vec<_>>();
    let playback_sessions = Arc::new(
        stream::ServerPlaybackSessions::new(
            storage.clone(),
            transcoder.clone(),
            Arc::new(cfg.transcoder.clone()),
            resolved_hwaccel,
            hw_detect.vaapi_device.clone(),
            jwt.clone(),
        )
        .with_encode_video(noflex_server::hwaccel::live_codec_order(
            resolved_hwaccel,
            &live_codecs,
        ))
        .with_gpu_tonemap(hw_detect.gpu_tonemap)
        .with_events(event_bus.clone()),
    );
    let live_tv = Arc::new(noflex_server::live_tv_runtime::ServerLiveTv::new(
        storage.clone(),
        playback_sessions.clone(),
        ffmpeg_path.clone(),
        cfg.data_dir.clone(),
        cfg.live_tv.recording_concurrency,
    )?);
    let _recording_worker = live_tv.clone().spawn();
    let _guide_refresh_worker = live_tv.clone().spawn_guide_refresh();
    let admin_status = noflex_server::system_status::SystemStatusService::start(
        noflex_server::system_status::SystemStatusDeps {
            config: cfg.clone(),
            storage: storage.clone(),
            plugins: plugin_registry.clone(),
            logs: log_buffer,
            recorder: status_recorder,
            metrics: metrics.clone(),
            hw_detect: hw_detect.clone(),
            resolved_backend: resolved_hwaccel,
            ffmpeg_path: ffmpeg_path.clone(),
            db_path: Some(db_path.clone()),
        },
    )
    .await;
    let optimized_dir = cfg
        .transcoder
        .optimize
        .dir
        .clone()
        .unwrap_or_else(|| cfg.data_dir.join("optimized"));
    std::fs::create_dir_all(&optimized_dir)
        .with_context(|| format!("create optimized dir {}", optimized_dir.display()))?;
    let schema = build_schema(SchemaContext {
        storage: Some(storage.clone()),
        scanner: Some(scan_registry.clone()),
        library_watch: Some(library_watchers),
        metadata: Some(metadata),
        priority_table: Some(priority_table),
        jwt: Some(jwt.clone()),
        export_events: Some(export_events.clone()),
        lifecycle_bus: Some(lifecycle_bus.clone()),
        event_bus: Some(event_bus.clone()),
        playback_sessions: Some(playback_sessions.clone()),
        live_tv: Some(live_tv),
        plugins: Some(plugin_registry.clone()),
        plugin_links: Some(plugin_links.clone()),
        external_auth: Some(external_auth.clone()),
        media_url_signer: Some(Arc::new(noflex_server::stream::JwtMediaUrlSigner::new(
            jwt.clone(),
        ))),
        watch_room_bus: Some(Arc::new(noflex_graphql::WatchRoomBus::default())),
        admin_status: Some(admin_status),
        metadata_service: Some(metadata_service),
        tonemap_curve: Some(cfg.transcoder.tonemap_curve),
        smart_transcoding: Some(noflex_graphql::SmartTranscodingDefaults {
            enabled: cfg.transcoder.optimize.enabled,
            directory: optimized_dir.display().to_string(),
            max_storage_gb: cfg.transcoder.optimize.max_storage_gb,
            max_version_gb: cfg.transcoder.optimize.max_version_gb,
        }),
    });
    let download_output_dir = if cfg.cache.transcode_dir.is_empty() {
        cfg.data_dir.join("downloads")
    } else {
        std::path::PathBuf::from(&cfg.cache.transcode_dir)
    };
    std::fs::create_dir_all(&download_output_dir)
        .with_context(|| format!("create download dir {}", download_output_dir.display()))?;
    let download_root = RootCapability::new(&download_output_dir)
        .await
        .context("open download root")?;
    let download_worker = Arc::new(
        DownloadWorker::new(storage.clone(), ffmpeg_path.clone(), download_output_dir)
            .with_events(export_events.clone()),
    );
    let _download_handle = download_worker.spawn();
    let mut optimize_config = cfg.transcoder.optimize.clone();
    optimize_config
        .encode_video
        .retain(|codec| usable_codecs.contains(codec));
    let _optimize_worker = Arc::new(noflex_server::optimize_worker::OptimizeWorker {
        storage: storage.clone(),
        playback: playback_sessions.clone(),
        plugins: Some(plugin_registry.clone()),
        config: optimize_config,
        ladder: cfg.transcoder.ladder.clone(),
        ffmpeg_path: ffmpeg_path.clone(),
        backend: resolved_hwaccel,
        vaapi_device: hw_detect.vaapi_device.clone(),
        gpu_tonemap: hw_detect.gpu_tonemap,
        tonemap_curve: cfg.transcoder.tonemap_curve,
        dir: optimized_dir,
    })
    .spawn();
    let recordings_dir = cfg.data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir)
        .with_context(|| format!("create recordings dir {}", recordings_dir.display()))?;
    let restricted_routes = noflex_server::restricted_routes::routes(
        noflex_server::restricted_routes::RestrictedRouteState {
            storage: storage.clone(),
            jwt: jwt.clone(),
            transcoder: transcoder.clone(),
            playback_job_authorizer: playback_sessions,
            transcode_cache_dir: transcode_dir,
            image_cache_dir,
            recordings_dir,
        },
    );
    let state = AppState {
        storage: storage.clone(),
        schema,
        jwt: jwt.clone(),
        plugin_registry: plugin_registry.clone(),
        metrics,
        export_events,
        download_worker,
        download_root,
        plugin_links,
        external_auth,
    };

    // When `cfg.backup.dir` is non-empty, run a backup once at startup and
    // then every `interval_hours`. Old
    // backups beyond `keep` are pruned. A disabled scheduler
    // (empty `dir`) is the default.
    if !cfg.backup.dir.is_empty() {
        let backup_storage = storage.clone();
        let backup_cfg = cfg.backup.clone();
        tokio::spawn(async move {
            run_backup_scheduler(backup_storage, backup_cfg).await;
        });
    }

    // Sessions untouched longer than the refresh-token TTL hold an
    // expired refresh token and can never be used again, so they're
    // pruned on a fixed hourly cadence (once immediately at startup),
    // matching the deliberately non-configurable TTLs.
    let prune_storage = storage.clone();
    let refresh_ttl_secs = jwt.refresh_token_ttl_secs;
    tokio::spawn(async move {
        run_session_pruner(prune_storage, refresh_ttl_secs).await;
    });

    let app = Router::new()
        .merge(noflex_server::web::routes())
        .route("/health", get(health))
        .route("/healthz/deep", get(healthz_deep))
        .route("/graphql", post(graphql_handler).get(graphiql))
        .route("/graphql/ws", get(graphql_ws))
        .route(
            "/metrics",
            get(
                |axum::extract::State(s): axum::extract::State<AppState>| async move {
                    noflex_server::metrics::render_response(s.metrics).await
                },
            ),
        )
        .route("/download/{job_id}/file", get(download_file))
        .route(
            "/plugins/{plugin_id}/link/callback",
            get(plugin_link_callback),
        )
        .route("/auth/{plugin_id}/callback", get(external_auth_callback))
        .with_state(state.clone())
        .layer(middleware::from_fn(auth_middleware))
        .layer(axum::Extension(state.jwt.as_ref().clone()))
        // Published for the auth middleware's API-key lookups.
        .layer(axum::Extension(storage.clone()))
        .merge(restricted_routes);
    let app = if cfg.cross_origin {
        app.layer(noflex_server::web::cross_origin_layer(storage.clone()))
    } else {
        app
    };

    info!(bind = %cfg.bind, "noflex server listening");
    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .context("bind tcp")?;
    // Peer addresses feed the login throttle (`ClientAddr`).
    let serve = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );
    // Not a graceful shutdown: a stream can stay open for hours, and a client
    // whose connection drops just reconnects.
    tokio::select! {
        result = async { serve.await } => result.context("serve")?,
        () = shutdown_signal() => info!("shutting down"),
    }
    Ok(())
}

/// Serve a completed export job to its owner (or an admin). The
/// actual work lives in `noflex_server::download_serve::serve_download_job`;
/// this handler just parses the path id and forwards the Range
/// header and the caller's principal.
async fn download_file(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<noflex_auth::Principal>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let job_id = match uuid::Uuid::parse_str(&job_id) {
        Ok(u) => noflex_core::ExportJobId::from_uuid(u),
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                format!("invalid job id: {e}"),
            )
                .into_response();
        }
    };
    let range_header = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    serve_download_job(
        &state.storage,
        &principal,
        job_id,
        &state.download_root,
        range_header,
    )
    .await
}

/// The provider's redirect target for the per-user plugin link flow.
/// Deliberately unauthenticated: the browser arrives from the
/// provider's redirect without our session headers, so the pending
/// state — single-use, short-TTL, bound to the linking user — is the
/// credential. All failures return one generic 400.
async fn plugin_link_callback(
    State(state): State<AppState>,
    axum::extract::Path(plugin_id): axum::extract::Path<String>,
    axum::extract::RawQuery(raw): axum::extract::RawQuery,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    noflex_server::plugin_links::callback_response(
        &state.plugin_links,
        &plugin_id,
        raw.as_deref(),
        params.get("state").map(String::as_str),
    )
    .await
}

async fn external_auth_callback(
    State(state): State<AppState>,
    axum::extract::Path(plugin_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    noflex_server::external_auth::callback_response(
        &state.external_auth,
        &plugin_id,
        params.get("state").map(String::as_str),
        params.get("code").map(String::as_str),
    )
    .await
}

fn per_plugin_config_toml(
    map: &std::collections::HashMap<String, toml::Value>,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for (k, v) in map {
        // `Value::to_string` renders a table as a bare inline table
        // (`{ url = "..." }`), which is not a parseable TOML document —
        // and host-config::read promises the guest a stanza it can feed
        // straight to a TOML parser.
        let rendered = match v {
            toml::Value::Table(_) => toml::to_string(v).unwrap_or_else(|_| v.to_string()),
            _ => v.to_string(),
        };
        out.insert(k.clone(), rendered);
    }
    out
}

/// Install the global subscriber; returns the ring buffer the admin
/// `recentLogs` query reads.
fn init_tracing(
    log_cfg: &noflex_server::config::LogConfig,
) -> Arc<noflex_server::system_status::LogBuffer> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let filter = EnvFilter::try_from_env("NOFLEX_LOG")
        .or_else(|_| EnvFilter::try_new("info,noflex=debug"))
        .expect("static filter parses");
    let fmt_layer = fmt::layer().with_target(true).with_thread_ids(false);
    // When `log.file` is set, create its parent directory and touch the file
    // so deployers can validate that the configured path is honored. Events
    // continue to go to stdout until a rotating file writer is configured.
    if let Some(path) = log_cfg.file.as_ref() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path);
    }
    let log_buffer = Arc::new(noflex_server::system_status::LogBuffer::default());
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(log_buffer.layer())
        .init();
    log_buffer
}

fn build_host_overrides(cfg: &PluginConfig) -> noflex_plugin_host::Overrides {
    let mut disabled: Vec<String> = Vec::new();
    let mut by_plugin: std::collections::HashMap<String, noflex_plugin_host::PluginOverride> =
        std::collections::HashMap::new();
    for (plugin_id, override_) in &cfg.overrides {
        if override_.disabled {
            disabled.push(plugin_id.clone());
        }
        if !override_.priorities.is_empty() {
            by_plugin.insert(
                plugin_id.clone(),
                noflex_plugin_host::PluginOverride {
                    priorities: override_.priorities.clone(),
                },
            );
        }
    }
    noflex_plugin_host::Overrides {
        by_plugin,
        disabled,
    }
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match state.storage.health().await {
        Ok(report) => axum::Json(report).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "storage health check failed");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "unhealthy").into_response()
        }
    }
}

/// Deep health endpoint. Reports the status of every
/// component the host depends on, not just storage. Returns 200
/// when everything's healthy; 503 if any subsystem is down. Each
/// subsystem check is bounded so a single broken piece can't
/// stall the whole endpoint.
async fn healthz_deep(State(state): State<AppState>) -> impl IntoResponse {
    use std::time::Duration;

    use serde_json::json;

    let mut checks = serde_json::Map::new();
    let mut all_ok = true;

    // Storage
    match tokio::time::timeout(Duration::from_secs(3), state.storage.health()).await {
        Ok(Ok(report)) => {
            checks.insert(
                "storage".to_string(),
                json!({ "ok": true, "backend": report.backend }),
            );
        }
        Ok(Err(e)) => {
            all_ok = false;
            checks.insert(
                "storage".to_string(),
                json!({ "ok": false, "error": e.to_string() }),
            );
        }
        Err(_) => {
            all_ok = false;
            checks.insert(
                "storage".to_string(),
                json!({ "ok": false, "error": "timeout after 3s" }),
            );
        }
    }

    // Plugin host — does the wasmtime engine still respond? A no-op
    // plugin count is fine; we only need the engine to answer the
    // snapshot call. (Engines don't expose a synchronous "alive"
    // probe, but `snapshot()` is a cheap reader on a RwLock.)
    let snap = state.plugin_registry.snapshot();
    checks.insert(
        "plugin_host".to_string(),
        json!({ "ok": true, "loaded_plugins": snap.len() }),
    );

    // ffmpeg availability. ffmpeg is loaded for transcode + ffprobe;
    // its presence is a strong "the runtime can do media work"
    // signal.
    let ffmpeg_ok = runnable(&noflex_core::bundle::ffmpeg_tool("ffmpeg", "FFMPEG_BIN"));
    let ffprobe_ok = runnable(&noflex_core::bundle::ffmpeg_tool("ffprobe", "FFPROBE_BIN"));
    if !ffmpeg_ok || !ffprobe_ok {
        all_ok = false;
    }
    checks.insert(
        "ffmpeg".to_string(),
        json!({
            "ok": ffmpeg_ok && ffprobe_ok,
            "ffmpeg": ffmpeg_ok,
            "ffprobe": ffprobe_ok,
        }),
    );

    let body = json!({
        "ok": all_ok,
        "checks": checks,
    });
    let status = if all_ok {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (status, axum::Json(body)).into_response()
}

/// Whether `tool` names a file, directly or by a search of `PATH`.
fn runnable(tool: &std::path::Path) -> bool {
    if tool.components().count() > 1 {
        return tool.is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

async fn run_backup_scheduler(
    storage: Arc<dyn noflex_storage::Storage>,
    cfg: noflex_server::config::BackupConfig,
) {
    use std::time::Duration;

    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    let dir = std::path::PathBuf::from(&cfg.dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::error!(error = %e, dir = %dir.display(), "backup scheduler: cannot create dir");
        return;
    }

    loop {
        let stamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "now".to_string())
            .replace(':', "-");
        let path = dir.join(format!("backup-{stamp}.sql"));
        match tokio::fs::File::create(&path).await {
            Ok(mut f) => {
                if let Err(e) = storage.backup_to(&mut f).await {
                    tracing::error!(error = %e, path = %path.display(), "backup write failed");
                } else {
                    tracing::info!(path = %path.display(), "backup written");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, path = %path.display(), "backup file create failed");
            }
        }

        // Prune old backups beyond `keep`. Sort by name (which
        // is timestamp-prefixed) and remove the oldest.
        if cfg.keep > 0 {
            prune_old_backups(&dir, cfg.keep).await;
        }

        if cfg.interval_hours == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_secs(u64::from(cfg.interval_hours) * 3600)).await;
    }
}

async fn run_session_pruner(storage: Arc<dyn noflex_storage::Storage>, refresh_ttl_secs: i64) {
    loop {
        let floor = time::OffsetDateTime::now_utc() - time::Duration::seconds(refresh_ttl_secs);
        match storage.prune_sessions_before(floor).await {
            Ok(0) => {}
            Ok(pruned) => tracing::info!(pruned, "pruned expired sessions"),
            Err(e) => tracing::warn!(error = %e, "session prune failed"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

async fn prune_old_backups(dir: &std::path::Path, keep: u32) {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(e) => e
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("backup-") {
                    Some((e.path(), name))
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => return,
    };
    // Sort lexicographically by name; the timestamp prefix makes
    // this the correct order.
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    let to_remove = entries.len().saturating_sub(keep as usize);
    for (path, _) in entries.into_iter().take(to_remove) {
        if let Err(e) = tokio::fs::remove_file(&path).await {
            tracing::warn!(error = %e, path = %path.display(), "prune failed");
        }
    }
}

/// The throttle key for a connection: the TCP peer, never a forwarded
/// header (there is no trusted-proxy config, and those are spoofable).
/// IPv4-mapped IPv6 peers fold to IPv4 so dual-stack listeners key
/// one client one way.
fn client_addr(peer: std::net::SocketAddr) -> noflex_auth::ClientAddr {
    noflex_auth::ClientAddr(peer.ip().to_canonical())
}

async fn graphql_handler(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<noflex_auth::Principal>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let authenticated = principal.is_authenticated();
    let request_user = authenticated.then_some(noflex_storage::RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    });
    let mut request = req.into_inner().data(principal).data(client_addr(peer));
    if let Some(request_user) = request_user {
        request = request.data(request_user);
    }
    if let Some(session) = principal.session_id {
        request = request.data(noflex_storage::CallerSession(session));
    }
    if is_subscription_request(&request) {
        return state.schema.execute(request).await.into();
    }
    execute_with_deadline(state.schema.execute(request), Duration::from_secs(10))
        .await
        .into()
}

async fn execute_with_deadline<F>(future: F, deadline: Duration) -> async_graphql::Response
where
    F: Future<Output = async_graphql::Response>,
{
    match tokio::time::timeout(deadline, future).await {
        Ok(response) => response,
        Err(_) => async_graphql::Response::from_errors(vec![async_graphql::ServerError::new(
            "GraphQL request timed out",
            None,
        )]),
    }
}

fn is_subscription_request(request: &async_graphql::Request) -> bool {
    let Ok(document) = async_graphql::parser::parse_query(&request.query) else {
        return false;
    };
    document
        .operations
        .iter()
        .find(|(name, _)| match (&request.operation_name, name) {
            (Some(requested), Some(name)) => requested == name.as_str(),
            (None, None) => true,
            _ => false,
        })
        .is_some_and(|(_, operation)| {
            operation.node.ty == async_graphql::parser::types::OperationType::Subscription
        })
}

async fn graphql_ws(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    protocol: async_graphql_axum::GraphQLProtocol,
    headers: axum::http::HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    noflex_server::graphql_ws::upgrade(
        ws,
        protocol,
        state.schema,
        state.jwt,
        state.storage,
        &headers,
        client_addr(peer),
    )
}

async fn graphiql() -> impl IntoResponse {
    Html(
        GraphiQLSource::build()
            .endpoint("/graphql")
            .subscription_endpoint("/graphql/ws")
            .finish(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_addr_is_the_canonical_peer_ip() {
        let v4: std::net::SocketAddr = "192.0.2.7:5000".parse().unwrap();
        let mapped: std::net::SocketAddr = "[::ffff:192.0.2.7]:6000".parse().unwrap();
        let v6: std::net::SocketAddr = "[2001:db8::1]:7000".parse().unwrap();
        assert_eq!(client_addr(v4), client_addr(mapped));
        assert_eq!(client_addr(v6).0, v6.ip());
    }

    #[test]
    fn subscription_detection_respects_operation_name() {
        let request = async_graphql::Request::new(
            "query Read { serverInfo { name } } subscription Events { exportEvents { id } }",
        )
        .operation_name("Events");
        assert!(is_subscription_request(&request));

        let request = async_graphql::Request::new("{ serverInfo { name } }");
        assert!(!is_subscription_request(&request));
    }

    #[tokio::test]
    async fn graphql_deadline_returns_stable_error() {
        let response = execute_with_deadline(
            std::future::pending::<async_graphql::Response>(),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(response.errors[0].message, "GraphQL request timed out");
    }

    #[test]
    fn per_plugin_config_toml_preserves_keys() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "tmdb".to_string(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "api_key".to_string(),
                toml::Value::String("secret".to_string()),
            )])),
        );
        let out = per_plugin_config_toml(&map);
        assert!(out.contains_key("tmdb"));
        let s = &out["tmdb"];
        // Must come back as a TOML document the guest can parse directly.
        let parsed: toml::Value = toml::from_str(s).expect("renders a TOML document");
        assert_eq!(
            parsed.get("api_key").and_then(toml::Value::as_str),
            Some("secret")
        );
    }

    #[test]
    fn per_plugin_config_toml_empty_map() {
        let map = std::collections::HashMap::new();
        let out = per_plugin_config_toml(&map);
        assert!(out.is_empty());
    }
}
