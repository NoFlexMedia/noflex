// SPDX-License-Identifier: AGPL-3.0-only
//
// The server's `AdminStatusService`: a background sampler that keeps a
// fresh host/process snapshot (every `FAST_TICK`) plus slower-moving
// figures (filesystems, database, cache sizes), so admin queries and
// the live subscription only ever read the latest value.

mod ffmpeg;
mod logs;
mod recorder;
mod redact;
mod sampler;

use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use async_graphql::ID;
pub use logs::{LogBuffer, LogCaptureLayer};
use noflex_graphql::{
    AdminStatusError, AdminStatusService,
    admin_status::{
        CodecMediaKind, CodecSupport, ConfigEntry, DatabaseStatus, LadderRung, LibraryActivity,
        LibraryRootStatus, LogFilter, LogPage, LogSummary, PluginEventPage, PluginHealthSummary,
        SearchIndexStatus, ServerRuntime, StatusSample, StorageLocation, StorageLocationKind,
        SystemStatus, TranscoderInfo, embedder_plugin_ids,
    },
};
use noflex_plugin_host::PluginRegistry;
use noflex_storage::{Storage, domain::Library};
use noflex_transcoder::{HwBackend, HwDetect};
pub use recorder::Recorder;
use sampler::{HostSample, HostSampler};
use time::OffsetDateTime;
use tokio::sync::{OnceCell, watch};

use crate::{
    config::{Config, HwAccel, TranscodeFallback},
    hwaccel,
    metrics::{HostGauges, Metrics},
};

const FAST_TICK: Duration = Duration::from_secs(2);
const SLOW_TICK: Duration = Duration::from_secs(15);
/// Cache directory sizes are walked every this many slow ticks.
const CONTENT_EVERY_SLOW_TICKS: u32 = 4;
/// Six minutes of sparkline at `FAST_TICK`.
const HISTORY_LEN: usize = 180;
/// A filesystem that doesn't answer statvfs this fast is reported as
/// unresponsive (typically a dead network mount).
const FS_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const DIR_WALK_MAX_ENTRIES: usize = 200_000;
/// Works counted toward the embedding backlog before giving up.
const EMBED_BACKLOG_CAP: u32 = 1_000;
const FAILURES_PER_LIBRARY: usize = 50;

/// Everything the service reads. Built once in `main`.
pub struct SystemStatusDeps {
    pub config: Config,
    pub storage: Arc<dyn Storage>,
    pub plugins: Arc<PluginRegistry>,
    pub logs: Arc<LogBuffer>,
    pub recorder: Arc<Recorder>,
    pub metrics: Arc<Metrics>,
    pub hw_detect: HwDetect,
    pub resolved_backend: HwBackend,
    pub ffmpeg_path: PathBuf,
    /// Local database file, for size reporting.
    pub db_path: Option<PathBuf>,
}

#[derive(Default)]
struct SlowState {
    database: DatabaseStatus,
    storage: Vec<StorageLocation>,
}

pub struct SystemStatusService {
    deps: SystemStatusDeps,
    started_at: OffsetDateTime,
    config_entries: Vec<ConfigEntry>,
    tx: watch::Sender<Arc<SystemStatus>>,
    history: Mutex<VecDeque<StatusSample>>,
    slow: Mutex<SlowState>,
    /// Paths with a statvfs still outstanding, so a hung mount costs
    /// one blocked thread rather than one per tick.
    fs_in_flight: Arc<Mutex<HashSet<PathBuf>>>,
    transcoder: OnceCell<Arc<TranscoderInfo>>,
}

impl SystemStatusService {
    /// Take a first sample and start the background samplers. The loops
    /// hold only a weak reference and stop once the service is dropped.
    pub async fn start(deps: SystemStatusDeps) -> Arc<Self> {
        let config_entries = serde_json::to_value(&deps.config)
            .map(|v| redact::flatten_config(&v))
            .unwrap_or_default();
        let mut sampler = HostSampler::new();
        let (sampler_back, first) = blocking_sample(sampler).await;
        sampler = sampler_back.unwrap_or_default();

        let started_at = OffsetDateTime::now_utc();
        let (tx, _) = watch::channel(Arc::new(empty_status(started_at)));
        let service = Arc::new(Self {
            deps,
            started_at,
            config_entries,
            tx,
            history: Mutex::new(VecDeque::with_capacity(HISTORY_LEN)),
            slow: Mutex::new(SlowState::default()),
            fs_in_flight: Arc::new(Mutex::new(HashSet::new())),
            transcoder: OnceCell::new(),
        });
        service.refresh_slow(true).await;
        service.publish(first);

        let weak = Arc::downgrade(&service);
        tokio::spawn(fast_loop(weak, sampler));
        let weak = Arc::downgrade(&service);
        tokio::spawn(slow_loop(weak));
        service
    }

    pub fn metrics(&self) -> Arc<Metrics> {
        self.deps.metrics.clone()
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn publish(&self, host: HostSample) {
        let now = OffsetDateTime::now_utc();
        let (warnings, errors) = self.deps.logs.counts();
        let (load_failures, traps) = self.deps.recorder.plugin_failure_counts();
        let plugins = self.deps.plugins.snapshot();
        let (database, storage) = {
            let slow = Self::lock(&self.slow);
            (slow.database.clone(), slow.storage.clone())
        };
        let cfg = &self.deps.config;
        let active_scans = self.deps.recorder.active_scans();
        let status = SystemStatus {
            sampled_at: now,
            server: ServerRuntime {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                started_at: self.started_at,
                uptime_seconds: u64::try_from((now - self.started_at).whole_seconds()).unwrap_or(0),
                bind: cfg.bind.clone(),
                public_url: cfg.public_url.clone(),
                data_dir: cfg.data_dir.display().to_string(),
                cache_dir: cfg.cache_dir.display().to_string(),
                config: self.config_entries.clone(),
                database,
            },
            host: host.host,
            cpu: host.cpu,
            memory: host.memory,
            process: host.process,
            network: host.network,
            network_total: host.network_total,
            storage,
            plugins: PluginHealthSummary {
                loaded: u32::try_from(plugins.len()).unwrap_or(u32::MAX),
                circuit_open: u32::try_from(plugins.iter().filter(|p| p.circuit_open).count())
                    .unwrap_or(u32::MAX),
                instance_restarts: plugins
                    .iter()
                    .fold(0u32, |acc, p| acc.saturating_add(p.instance_restarts)),
                load_failures,
                traps,
            },
            logs: LogSummary {
                warnings,
                errors,
                last_error: self.deps.logs.last_error(),
            },
            active_scans,
        };
        let sample = StatusSample {
            at: now,
            cpu_percent: status.cpu.usage_percent,
            memory_used_bytes: status.memory.used_bytes,
            process_cpu_percent: status.process.cpu_percent,
            process_rss_bytes: status.process.rss_bytes,
            rx_bytes_per_sec: status.network_total.rx_bytes_per_sec,
            tx_bytes_per_sec: status.network_total.tx_bytes_per_sec,
            load_one: status.cpu.load_average.map(|l| l.one),
        };
        {
            let mut history = Self::lock(&self.history);
            while history.len() >= HISTORY_LEN {
                history.pop_front();
            }
            history.push_back(sample);
        }
        self.deps.metrics.set_host_gauges(HostGauges {
            host_cpu_percent: f64::from(status.cpu.usage_percent),
            host_memory_used_bytes: status.memory.used_bytes,
            host_memory_total_bytes: status.memory.total_bytes,
            process_cpu_percent: f64::from(status.process.cpu_percent),
            process_rss_bytes: status.process.rss_bytes,
            log_warnings_total: warnings,
            log_errors_total: errors,
            active_scans: u64::from(active_scans),
        });
        self.tx.send_replace(Arc::new(status));
    }

    async fn refresh_slow(&self, measure_content: bool) {
        let database = self.database_status().await;
        let previous: Vec<StorageLocation> = Self::lock(&self.slow).storage.clone();
        let storage = self.storage_locations(measure_content, &previous).await;
        let mut slow = Self::lock(&self.slow);
        slow.database = database;
        slow.storage = storage;
    }

    async fn database_status(&self) -> DatabaseStatus {
        let mut status = match self.deps.storage.health().await {
            Ok(h) => DatabaseStatus {
                backend: h.backend,
                engine_version: h.version,
                schema_version: h.schema_version,
                healthy: h.healthy,
                notes: h.notes,
                ..DatabaseStatus::default()
            },
            Err(e) => DatabaseStatus {
                error: Some(e.to_string()),
                ..DatabaseStatus::default()
            },
        };
        if let Some(path) = &self.deps.db_path {
            status.path = Some(path.display().to_string());
            status.size_bytes = tokio::fs::metadata(path).await.ok().map(|m| m.len());
            // Turso's MVCC log: commits not yet checkpointed into the file.
            let mut wal = path.as_os_str().to_owned();
            wal.push("-log");
            status.wal_size_bytes = tokio::fs::metadata(PathBuf::from(wal))
                .await
                .ok()
                .map(|m| m.len());
        }
        status
    }

    async fn storage_locations(
        &self,
        measure_content: bool,
        previous: &[StorageLocation],
    ) -> Vec<StorageLocation> {
        let cfg = &self.deps.config;
        let mut targets: Vec<(StorageLocationKind, PathBuf, Option<&Library>)> = vec![
            (StorageLocationKind::Data, cfg.data_dir.clone(), None),
            (StorageLocationKind::Cache, cfg.cache_dir.clone(), None),
            (
                StorageLocationKind::TranscodeCache,
                cache_dir(cfg, &cfg.cache.transcode_dir, "transcode"),
                None,
            ),
            (
                StorageLocationKind::ImageCache,
                cache_dir(cfg, &cfg.cache.image_dir, "image"),
                None,
            ),
            (
                StorageLocationKind::EmbedCache,
                cache_dir(cfg, &cfg.cache.embed_dir, "embed"),
                None,
            ),
            (
                StorageLocationKind::Recordings,
                cfg.data_dir.join("recordings"),
                None,
            ),
        ];
        let libraries = self.deps.storage.list_libraries().await.unwrap_or_default();
        for library in &libraries {
            for root in &library.paths {
                targets.push((
                    StorageLocationKind::LibraryRoot,
                    PathBuf::from(root),
                    Some(library),
                ));
            }
        }
        let mounts = tokio::task::spawn_blocking(sampler::read_mounts)
            .await
            .unwrap_or_default();

        let mut out = Vec::with_capacity(targets.len());
        for (kind, path, library) in targets {
            let measures_content = matches!(
                kind,
                StorageLocationKind::TranscodeCache
                    | StorageLocationKind::ImageCache
                    | StorageLocationKind::EmbedCache
                    | StorageLocationKind::Recordings
            );
            let display = path.display().to_string();
            let mut loc = StorageLocation {
                kind,
                path: display.clone(),
                library_id: library.map(|l| ID(l.id.to_string())),
                library_name: library.map(|l| l.name.clone()),
                exists: false,
                mount_point: None,
                file_system: None,
                total_bytes: None,
                available_bytes: None,
                used_bytes: None,
                content_bytes: None,
                error: None,
            };
            match self.probe_fs(&path).await {
                Ok(usage) => {
                    loc.exists = true;
                    loc.total_bytes = Some(usage.total);
                    loc.available_bytes = Some(usage.available);
                    loc.used_bytes = Some(usage.used);
                    let canonical = tokio::fs::canonicalize(&path).await.unwrap_or(path.clone());
                    if let Some(m) = sampler::mount_for(&mounts, &canonical) {
                        loc.mount_point = Some(m.mount_point.display().to_string());
                        loc.file_system = Some(m.fs_type.clone());
                    }
                }
                Err(e) => loc.error = Some(e),
            }
            if measures_content && loc.exists {
                loc.content_bytes = if measure_content {
                    let walk_path = path.clone();
                    tokio::task::spawn_blocking(move || {
                        sampler::dir_size(&walk_path, DIR_WALK_MAX_ENTRIES)
                    })
                    .await
                    .ok()
                    .map(|(bytes, _complete)| bytes)
                } else {
                    previous
                        .iter()
                        .find(|p| p.kind == kind && p.path == display)
                        .and_then(|p| p.content_bytes)
                };
            }
            out.push(loc);
        }
        out
    }

    async fn probe_fs(&self, path: &Path) -> Result<sampler::FsUsage, String> {
        if !Self::lock(&self.fs_in_flight).insert(path.to_path_buf()) {
            return Err("previous filesystem probe has not returned (unresponsive mount?)".into());
        }
        let in_flight = self.fs_in_flight.clone();
        let owned = path.to_path_buf();
        let task = tokio::task::spawn_blocking(move || {
            let result = sampler::fs_usage(&owned);
            Self::lock(&in_flight).remove(&owned);
            result
        });
        match tokio::time::timeout(FS_PROBE_TIMEOUT, task).await {
            Ok(Ok(Ok(usage))) => Ok(usage),
            Ok(Ok(Err(e))) => Err(e.to_string()),
            Ok(Err(e)) => Err(format!("filesystem probe failed: {e}")),
            Err(_) => Err(format!(
                "filesystem did not respond within {FS_PROBE_TIMEOUT:?}"
            )),
        }
    }

    async fn probe_transcoder(&self) -> TranscoderInfo {
        let cfg = &self.deps.config;
        let det = &self.deps.hw_detect;
        let ffprobe_cmd = noflex_core::bundle::ffmpeg_tool("ffprobe", "FFPROBE_BIN");
        let (ffmpeg, ffprobe) = tokio::join!(
            ffmpeg::probe_binary(&self.deps.ffmpeg_path, "FFMPEG_BIN"),
            ffmpeg::probe_binary(&ffprobe_cmd, "FFPROBE_BIN"),
        );
        let decoder_names: HashSet<String> = match &ffmpeg.path {
            Some(path) => ffmpeg::decoder_names(Path::new(path))
                .await
                .into_iter()
                .collect(),
            None => HashSet::new(),
        };
        let encoder_names: HashSet<&str> = det.encoders.iter().map(String::as_str).collect();
        let resolved = self.deps.resolved_backend;

        let mut encoders: Vec<CodecSupport> = hwaccel::ALL_VIDEO_ENCODERS
            .iter()
            .map(|e| CodecSupport {
                name: e.video_flag().to_owned(),
                media: CodecMediaKind::Video,
                backend: hwaccel::backend_name(e.backend()).to_owned(),
                available: encoder_names.contains(e.video_flag()),
            })
            .collect();
        encoders.extend(hwaccel::AUDIO_ENCODERS.iter().map(|name| CodecSupport {
            name: (*name).to_owned(),
            media: CodecMediaKind::Audio,
            backend: "software".to_owned(),
            available: encoder_names.contains(name),
        }));
        let decoders = hwaccel::DECODERS
            .iter()
            .map(|(name, backend, video)| CodecSupport {
                name: (*name).to_owned(),
                media: if *video {
                    CodecMediaKind::Video
                } else {
                    CodecMediaKind::Audio
                },
                backend: (*backend).to_owned(),
                available: decoder_names.contains(*name),
            })
            .collect();
        let ladder = cfg
            .transcoder
            .ladder
            .iter()
            .map(|rung| {
                let encoder = hwaccel::pick_encoder(resolved, rung.video_codec).video_flag();
                LadderRung {
                    max_pixels: rung.max_pixels,
                    video_codec: rung.video_codec,
                    video_profile: rung.video_profile,
                    audio_codec: rung.audio_codec,
                    container: rung.container.clone(),
                    bitrate_kbps: rung.bitrate_kbps,
                    encoder: encoder.to_owned(),
                    encoder_available: encoder_names.contains(encoder),
                }
            })
            .collect();
        let configured = hwaccel::configured_backend(cfg.transcoder.hwaccel);
        TranscoderInfo {
            ffmpeg,
            ffprobe,
            configured_hwaccel: hwaccel_name(cfg.transcoder.hwaccel).to_owned(),
            fallback: match cfg.transcoder.fallback {
                TranscodeFallback::Auto => "auto",
                TranscodeFallback::Software => "software",
            }
            .to_owned(),
            resolved_backend: hwaccel::backend_name(resolved).to_owned(),
            hardware_accelerated: resolved != HwBackend::Software,
            fell_back: configured.is_some_and(|c| c != resolved),
            hwaccels: det.hwaccels.clone(),
            vaapi_device: det.vaapi_device.as_ref().map(|p| p.display().to_string()),
            gpu_tonemap: det.gpu_tonemap,
            encoders,
            decoders,
            ladder,
            transcode_dir: cache_dir(cfg, &cfg.cache.transcode_dir, "transcode")
                .display()
                .to_string(),
        }
    }
}

#[async_trait::async_trait]
impl AdminStatusService for SystemStatusService {
    fn system_status(&self) -> Arc<SystemStatus> {
        self.tx.borrow().clone()
    }

    fn status_history(&self) -> Vec<StatusSample> {
        Self::lock(&self.history).iter().copied().collect()
    }

    fn watch_system_status(&self) -> watch::Receiver<Arc<SystemStatus>> {
        self.tx.subscribe()
    }

    async fn transcoder_info(&self) -> Arc<TranscoderInfo> {
        self.transcoder
            .get_or_init(|| async { Arc::new(self.probe_transcoder().await) })
            .await
            .clone()
    }

    async fn library_activity(&self, library: &Library) -> LibraryActivity {
        let (active_scan, last_scan, watcher) = self.deps.recorder.library(library.id);
        let (recent_failures, failure_count) = self
            .deps
            .logs
            .scan_failures_under(&library.paths, FAILURES_PER_LIBRARY);
        let mut roots = Vec::with_capacity(library.paths.len());
        for root in &library.paths {
            roots.push(root_status(root).await);
        }
        LibraryActivity {
            active_scan,
            last_scan,
            watcher,
            roots,
            recent_failures,
            failure_count,
        }
    }

    async fn search_index_status(&self) -> Result<SearchIndexStatus, AdminStatusError> {
        let plugins = embedder_plugin_ids(&self.deps.plugins);
        let Some(claim) = self.deps.plugins.embedder_claim() else {
            return Ok(SearchIndexStatus {
                embedder_plugins: plugins,
                ..SearchIndexStatus::default()
            });
        };
        let mut pending = 0u32;
        let mut cursor = None;
        let lower_bound = loop {
            let page = self
                .deps
                .storage
                .works_missing_embedding(&claim.namespace, 500, cursor)
                .await?;
            pending = pending.saturating_add(u32::try_from(page.items.len()).unwrap_or(u32::MAX));
            match page.next_cursor {
                Some(next) if pending < EMBED_BACKLOG_CAP => cursor = Some(next),
                Some(_) => break true,
                None => break false,
            }
        };
        Ok(SearchIndexStatus {
            embedder_plugins: plugins,
            remote_host: self
                .deps
                .plugins
                .remote_embedder()
                .and_then(|remote| remote.host()),
            inference_device: self.deps.plugins.inference().device_in_use(),
            namespace: Some(claim.namespace),
            model_id: Some(claim.model_id),
            dimensions: Some(claim.dimensions),
            pending_works: pending,
            pending_is_lower_bound: lower_bound,
        })
    }

    fn recent_logs(&self, filter: &LogFilter, before: Option<u64>, first: usize) -> LogPage {
        self.deps.logs.query(filter, before, first)
    }

    fn plugin_events(&self, before: Option<u64>, first: usize) -> PluginEventPage {
        self.deps.recorder.plugin_events(before, first)
    }
}

async fn fast_loop(service: Weak<SystemStatusService>, mut sampler: HostSampler) {
    let mut tick = tokio::time::interval(FAST_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    loop {
        tick.tick().await;
        let (back, sample) = blocking_sample(sampler).await;
        let Some(service) = service.upgrade() else {
            return;
        };
        service.publish(sample);
        sampler = back.unwrap_or_default();
    }
}

async fn slow_loop(service: Weak<SystemStatusService>) {
    let mut tick = tokio::time::interval(SLOW_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    let mut n: u32 = 0;
    loop {
        tick.tick().await;
        n = n.wrapping_add(1);
        let Some(service) = service.upgrade() else {
            return;
        };
        service
            .refresh_slow(n.is_multiple_of(CONTENT_EVERY_SLOW_TICKS))
            .await;
    }
}

/// Run one sample on the blocking pool, handing the sampler back. A
/// panicked sample loses the sampler (rates restart from zero) but
/// never the loop.
async fn blocking_sample(mut sampler: HostSampler) -> (Option<HostSampler>, HostSample) {
    match tokio::task::spawn_blocking(move || {
        let sample = sampler.sample();
        (sampler, sample)
    })
    .await
    {
        Ok((sampler, sample)) => (Some(sampler), sample),
        Err(_) => (None, HostSample::default()),
    }
}

async fn root_status(root: &str) -> LibraryRootStatus {
    let mut status = LibraryRootStatus {
        path: root.to_owned(),
        ..LibraryRootStatus::default()
    };
    let probe = async {
        match tokio::fs::metadata(root).await {
            Ok(meta) => {
                status.exists = true;
                status.is_dir = meta.is_dir();
                match tokio::fs::read_dir(root).await {
                    Ok(_) => status.readable = true,
                    Err(e) => status.error = Some(e.to_string()),
                }
            }
            Err(e) => status.error = Some(e.to_string()),
        }
    };
    if tokio::time::timeout(FS_PROBE_TIMEOUT, probe).await.is_err() {
        return LibraryRootStatus {
            path: root.to_owned(),
            error: Some(format!(
                "filesystem did not respond within {FS_PROBE_TIMEOUT:?}"
            )),
            ..LibraryRootStatus::default()
        };
    }
    status
}

/// A cache directory: the configured path, or `<cache_dir>/<name>`
/// when unset — the same rule `main` uses when creating them.
pub fn cache_dir(cfg: &Config, configured: &str, name: &str) -> PathBuf {
    if configured.is_empty() {
        cfg.cache_dir.join(name)
    } else {
        PathBuf::from(configured)
    }
}

fn hwaccel_name(h: HwAccel) -> &'static str {
    match h {
        HwAccel::None => "none",
        HwAccel::Auto => "auto",
        HwAccel::Vaapi => "vaapi",
        HwAccel::VideoToolbox => "videotoolbox",
        HwAccel::Mf => "mf",
        HwAccel::Nvenc => "nvenc",
        HwAccel::Qsv => "qsv",
    }
}

fn empty_status(started_at: OffsetDateTime) -> SystemStatus {
    SystemStatus {
        sampled_at: started_at,
        server: ServerRuntime {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            started_at,
            uptime_seconds: 0,
            bind: String::new(),
            public_url: None,
            data_dir: String::new(),
            cache_dir: String::new(),
            config: Vec::new(),
            database: DatabaseStatus::default(),
        },
        host: Default::default(),
        cpu: Default::default(),
        memory: Default::default(),
        process: Default::default(),
        network: Vec::new(),
        network_total: Default::default(),
        storage: Vec::new(),
        plugins: Default::default(),
        logs: Default::default(),
        active_scans: 0,
    }
}
