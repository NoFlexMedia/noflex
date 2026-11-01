// SPDX-License-Identifier: AGPL-3.0-only
//
// Admin diagnostics surface: host resources, transcoder/codec
// capabilities, indexing health, plugin lifecycle history and the
// recent tracing log. The schema only shapes and gates the data; the
// sampling, ring buffers and ffmpeg probes live in the server behind
// [`AdminStatusService`], so this crate never touches `/proc`, sysinfo
// or a subprocess.

use std::{pin::Pin, sync::Arc};

use async_graphql::{
    Context, Enum, ID, Object, SimpleObject,
    connection::{Connection, Edge, EmptyFields},
};
use futures::Stream;
use noflex_core::{AudioCodec, VideoCodec};
use noflex_plugin_host::{HookKind, PluginRegistry};
use noflex_storage::{
    Storage,
    domain::{Library, LibraryScanStatus, ScanState},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{PluginLifecycleEvent, access};

/// Failure reading a diagnostic that depends on another subsystem.
/// Sampled host data never fails; only storage-backed reads can.
#[derive(Debug, thiserror::Error)]
pub enum AdminStatusError {
    #[error("storage: {0}")]
    Storage(#[from] noflex_storage::error::StorageError),
}

/// Server-side diagnostics provider, implemented in `noflex-server`
/// and wired through [`crate::SchemaContext::admin_status`]. Every
/// method is cheap: host figures come from a background sampler's
/// latest snapshot and logs/events from bounded in-memory rings, so
/// resolvers never block on a probe.
#[async_trait::async_trait]
pub trait AdminStatusService: Send + Sync {
    /// The most recent host/process/server sample.
    fn system_status(&self) -> Arc<SystemStatus>;

    /// Retained samples, oldest first. The window is bounded by the
    /// implementation (minutes, not hours).
    fn status_history(&self) -> Vec<StatusSample>;

    /// Receiver that observes every new sample. The current value is
    /// the latest sample, so a fresh subscriber can render immediately.
    fn watch_system_status(&self) -> tokio::sync::watch::Receiver<Arc<SystemStatus>>;

    /// ffmpeg/ffprobe identity, hardware-acceleration resolution, codec
    /// availability and the configured ladder. Probed once, then cached.
    async fn transcoder_info(&self) -> Arc<TranscoderInfo>;

    /// Indexing detail for one library: observed scan runs (manual and
    /// watcher-triggered), filesystem watcher state, root reachability
    /// and per-file failures still in the retention window.
    async fn library_activity(&self, library: &Library) -> LibraryActivity;

    /// Vector-index coverage for the active embedder.
    async fn search_index_status(&self) -> Result<SearchIndexStatus, AdminStatusError>;

    /// Retained log entries matching `filter`, newest first, starting
    /// strictly below sequence `before` (all when `None`). Returns at
    /// most `first` entries plus whether older matches remain.
    fn recent_logs(&self, filter: &LogFilter, before: Option<u64>, first: usize) -> LogPage;

    /// Retained plugin lifecycle events, newest first, strictly below
    /// sequence `before`. Returns at most `first` records plus whether
    /// older records remain.
    fn plugin_events(&self, before: Option<u64>, first: usize) -> PluginEventPage;
}

// ---- Host / server snapshot ---------------------------------------------

/// One sample of host, process and server state.
#[derive(SimpleObject, Debug, Clone)]
pub struct SystemStatus {
    pub sampled_at: OffsetDateTime,
    pub server: ServerRuntime,
    pub host: HostInfo,
    pub cpu: CpuStatus,
    pub memory: MemoryStatus,
    pub process: ProcessStatus,
    /// Every interface the OS reports, loopback included.
    pub network: Vec<NetworkInterface>,
    /// Aggregate rates across non-loopback interfaces.
    pub network_total: NetworkTotals,
    /// Usage of the filesystems backing NoFlex's directories and every
    /// library root.
    pub storage: Vec<StorageLocation>,
    pub plugins: PluginHealthSummary,
    pub logs: LogSummary,
    /// Libraries with a scan in flight (manual or watcher-triggered).
    pub active_scans: u32,
}

#[derive(SimpleObject, Debug, Clone)]
pub struct ServerRuntime {
    pub version: String,
    pub started_at: OffsetDateTime,
    pub uptime_seconds: u64,
    pub bind: String,
    pub public_url: Option<String>,
    pub data_dir: String,
    pub cache_dir: String,
    /// Resolved configuration, flattened to dotted keys. Secret-looking
    /// values are replaced and flagged `redacted`.
    pub config: Vec<ConfigEntry>,
    pub database: DatabaseStatus,
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    /// Dotted path, arrays indexed: `transcoder.ladder[0].max_height`.
    pub key: String,
    /// Scalar rendered as text; `null` for unset options.
    pub value: Option<String>,
    pub redacted: bool,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct DatabaseStatus {
    pub backend: String,
    pub engine_version: String,
    pub schema_version: u32,
    pub healthy: bool,
    pub notes: Vec<String>,
    /// Main database file. `None` for engines without a local file.
    pub path: Option<String>,
    /// Main file plus write-ahead log, in bytes.
    pub size_bytes: Option<u64>,
    pub wal_size_bytes: Option<u64>,
    /// Set when the health probe itself failed.
    pub error: Option<String>,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct HostInfo {
    pub hostname: Option<String>,
    pub os_name: Option<String>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
    pub arch: String,
    pub booted_at: Option<OffsetDateTime>,
    pub uptime_seconds: Option<u64>,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct CpuStatus {
    /// Whole-machine utilisation, 0–100.
    pub usage_percent: f32,
    pub brand: String,
    pub physical_cores: Option<u32>,
    pub logical_cores: u32,
    pub cores: Vec<CpuCore>,
    /// `None` where the OS has no load average (Windows).
    pub load_average: Option<LoadAverage>,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct CpuCore {
    pub index: u32,
    /// 0–100.
    pub usage_percent: f32,
    pub frequency_mhz: u64,
}

#[derive(SimpleObject, Debug, Clone, Copy, Default)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct MemoryStatus {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    /// Memory ceiling of the enclosing cgroup (containers), when one
    /// applies.
    pub cgroup_limit_bytes: Option<u64>,
}

/// The NoFlex server process itself.
#[derive(SimpleObject, Debug, Clone, Default)]
pub struct ProcessStatus {
    pub pid: u32,
    /// Summed across cores: 200 means two cores fully busy.
    pub cpu_percent: f32,
    /// `cpu_percent` divided by the logical core count, 0–100.
    pub cpu_percent_of_host: f32,
    pub rss_bytes: u64,
    pub virtual_bytes: u64,
    pub threads: Option<u32>,
    pub open_files: Option<u32>,
    pub open_files_limit: Option<u32>,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct NetworkInterface {
    pub name: String,
    pub loopback: bool,
    /// OS operational state (`up`, `down`, `unknown`, …).
    pub state: String,
    pub mac_address: Option<String>,
    pub addresses: Vec<String>,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
    pub rx_total_bytes: u64,
    pub tx_total_bytes: u64,
    pub rx_errors_total: u64,
    pub tx_errors_total: u64,
}

#[derive(SimpleObject, Debug, Clone, Copy, Default)]
pub struct NetworkTotals {
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageLocationKind {
    Data,
    Cache,
    TranscodeCache,
    ImageCache,
    EmbedCache,
    Recordings,
    LibraryRoot,
}

/// A directory NoFlex depends on and the filesystem holding it.
#[derive(SimpleObject, Debug, Clone)]
pub struct StorageLocation {
    pub kind: StorageLocationKind,
    pub path: String,
    /// Set for `LIBRARY_ROOT`.
    pub library_id: Option<ID>,
    pub library_name: Option<String>,
    pub exists: bool,
    pub mount_point: Option<String>,
    pub file_system: Option<String>,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    /// Bytes under the directory itself. Measured only for caches, on a
    /// slower cadence than the filesystem figures.
    pub content_bytes: Option<u64>,
    /// Why the filesystem figures are missing.
    pub error: Option<String>,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct PluginHealthSummary {
    pub loaded: u32,
    /// Plugins whose trap circuit breaker is open.
    pub circuit_open: u32,
    /// Instance recycles after traps, summed over loaded plugins.
    pub instance_restarts: u32,
    /// `LOAD_FAILED` events since startup.
    pub load_failures: u32,
    /// `TRAPPED` events since startup.
    pub traps: u32,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct LogSummary {
    /// WARN events since startup, including ones evicted from the ring.
    pub warnings: u64,
    /// ERROR events since startup, including ones evicted from the ring.
    pub errors: u64,
    pub last_error: Option<LogEntry>,
}

/// A compact point for sparklines.
#[derive(SimpleObject, Debug, Clone, Copy)]
pub struct StatusSample {
    pub at: OffsetDateTime,
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub process_cpu_percent: f32,
    pub process_rss_bytes: u64,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
    pub load_one: Option<f64>,
}

// ---- Transcoder -------------------------------------------------------------

#[derive(SimpleObject, Debug, Clone)]
pub struct TranscoderInfo {
    pub ffmpeg: BinaryInfo,
    pub ffprobe: BinaryInfo,
    /// `transcoder.hwaccel` as configured (`auto`, `none`, `vaapi`, …).
    pub configured_hwaccel: String,
    /// `transcoder.fallback` as configured.
    pub fallback: String,
    /// Backend every transcode uses (`software`, `vaapi`, `nvenc`, …).
    pub resolved_backend: String,
    pub hardware_accelerated: bool,
    /// An explicitly configured backend was unavailable at startup and
    /// `fallback` picked `resolved_backend` instead.
    pub fell_back: bool,
    /// `ffmpeg -hwaccels`.
    pub hwaccels: Vec<String>,
    pub vaapi_device: Option<String>,
    /// Encoders the ladder can map to, with availability in this ffmpeg.
    pub encoders: Vec<CodecSupport>,
    /// Decoders for common source codecs, including hardware variants.
    pub decoders: Vec<CodecSupport>,
    pub ladder: Vec<LadderRung>,
    pub transcode_dir: String,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct BinaryInfo {
    /// What NoFlex invokes: the configured override or the bare name.
    pub command: String,
    /// The environment variable that overrides `command`.
    pub override_env: String,
    /// Absolute path `command` resolves to, when found.
    pub path: Option<String>,
    /// First line of `-version`.
    pub version: Option<String>,
    /// `--enable-*` flags from the build configuration.
    pub build_flags: Vec<String>,
    pub error: Option<String>,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecMediaKind {
    Video,
    Audio,
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
pub struct CodecSupport {
    /// ffmpeg codec name (`hevc_vaapi`, `libx264`, `h264_cuvid`, …).
    pub name: String,
    pub media: CodecMediaKind,
    /// `software`, or the hardware backend the codec runs on.
    pub backend: String,
    pub available: bool,
}

#[derive(SimpleObject, Debug, Clone)]
pub struct LadderRung {
    pub max_height: u32,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub container: String,
    pub bitrate_kbps: u32,
    /// The video encoder this rung runs on the resolved backend.
    pub encoder: String,
    /// `encoder` is present in this ffmpeg build.
    pub encoder_available: bool,
}

// ---- Indexing ---------------------------------------------------------------

/// What the server observed about one library's indexing, beyond the
/// registry's manual-scan status.
#[derive(Debug, Clone, Default)]
pub struct LibraryActivity {
    pub active_scan: Option<IndexScanRun>,
    pub last_scan: Option<IndexScanRun>,
    pub watcher: LibraryWatcherStatus,
    pub roots: Vec<LibraryRootStatus>,
    /// Newest first, capped.
    pub recent_failures: Vec<ScanFailure>,
    /// All retained failures for this library (≥ `recent_failures.len()`).
    pub failure_count: u32,
}

#[derive(SimpleObject, Debug, Clone, Default, PartialEq)]
pub struct IndexScanRun {
    pub started_at: Option<OffsetDateTime>,
    /// `None` while running.
    pub finished_at: Option<OffsetDateTime>,
    /// Final counts once finished; works created so far while running.
    pub added: u32,
    pub updated: u32,
    pub removed: u32,
    /// Per-file failures logged during the run.
    pub failures: u32,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WatcherState {
    /// No watcher is running: watching is disabled, or the library is gone.
    #[default]
    NotStarted,
    Active,
    Failed,
}

#[derive(SimpleObject, Debug, Clone, Default, PartialEq)]
pub struct LibraryWatcherStatus {
    pub state: WatcherState,
    pub since: Option<OffsetDateTime>,
    pub error: Option<String>,
}

#[derive(SimpleObject, Debug, Clone, Default, PartialEq)]
pub struct LibraryRootStatus {
    pub path: String,
    pub exists: bool,
    pub is_dir: bool,
    pub readable: bool,
    pub error: Option<String>,
}

/// A media file the scanner failed on.
#[derive(SimpleObject, Debug, Clone, PartialEq)]
pub struct ScanFailure {
    pub at: OffsetDateTime,
    pub path: String,
    /// Scanner stage message (`scan: ffprobe failed`, …).
    pub stage: String,
    pub error: Option<String>,
}

#[derive(SimpleObject, Debug, Clone)]
pub struct LibraryIndexStatus {
    pub library: Library,
    /// Manual (`scanLibrary`) scan status from the scan registry.
    pub scan: LibraryScanStatus,
    pub active_scan: Option<IndexScanRun>,
    /// Most recent finished run; survives restarts via the activity log.
    pub last_scan: Option<IndexScanRun>,
    pub watcher: LibraryWatcherStatus,
    pub roots: Vec<LibraryRootStatus>,
    pub recent_failures: Vec<ScanFailure>,
    pub failure_count: u32,
}

#[derive(SimpleObject, Debug, Clone, Default)]
pub struct SearchIndexStatus {
    /// Plugins exporting the embedder hook; the first is active.
    pub embedder_plugins: Vec<String>,
    pub namespace: Option<String>,
    pub model_id: Option<String>,
    pub dimensions: Option<u32>,
    /// Works with no vector in `namespace`.
    pub pending_works: u32,
    /// `pending_works` hit the counting cap; the true backlog is larger.
    pub pending_is_lower_bound: bool,
}

// ---- Logs -------------------------------------------------------------------

/// Ordered least to most severe.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<tracing::Level> for LogLevel {
    fn from(level: tracing::Level) -> Self {
        match level {
            tracing::Level::TRACE => LogLevel::Trace,
            tracing::Level::DEBUG => LogLevel::Debug,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::ERROR => LogLevel::Error,
        }
    }
}

/// One captured tracing event.
#[derive(SimpleObject, Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// Monotonic capture sequence; also the pagination cursor.
    pub id: ID,
    #[graphql(skip)]
    pub seq: u64,
    pub at: OffsetDateTime,
    pub level: LogLevel,
    /// Emitting module path (`noflex_scanner::scan`, …).
    pub target: String,
    pub message: String,
    pub fields: Vec<LogField>,
    /// Enclosing spans, outermost first, as `name{k=v …}`.
    pub spans: Vec<String>,
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
pub struct LogField {
    pub name: String,
    pub value: String,
}

/// Retention filter for [`AdminStatusService::recent_logs`].
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// Inclusive floor; `None` keeps every retained level.
    pub min_level: Option<LogLevel>,
    /// Keeps entries whose target starts with this prefix.
    pub target_prefix: Option<String>,
    /// Case-insensitive substring over message and field values.
    pub contains: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LogPage {
    pub entries: Vec<LogEntry>,
    pub has_more: bool,
}

// ---- Plugin events ----------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PluginEventRecord {
    pub seq: u64,
    pub at: OffsetDateTime,
    pub event: noflex_plugin_host::PluginLifecycleEvent,
}

#[derive(Debug, Clone, Default)]
pub struct PluginEventPage {
    pub records: Vec<PluginEventRecord>,
    pub has_more: bool,
}

/// A retained plugin lifecycle event with its capture time.
#[derive(SimpleObject)]
pub struct PluginEventEntry {
    pub id: ID,
    pub at: OffsetDateTime,
    pub event: PluginLifecycleEvent,
}

// ---- Resolvers --------------------------------------------------------------

const DEFAULT_PAGE: i32 = 100;
const MAX_PAGE: i32 = 500;
/// Rows per complexity unit on ring-buffer connections: the reads are
/// in-memory, so cost scales with response size, not with rows.
const ROWS_PER_COMPLEXITY_UNIT: usize = 50;

fn service(ctx: &Context<'_>) -> async_graphql::Result<Arc<dyn AdminStatusService>> {
    ctx.data::<Arc<dyn AdminStatusService>>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("admin status not configured"))
}

fn page_size(first: Option<i32>) -> async_graphql::Result<usize> {
    match first {
        Some(n) if n < 0 => Err(async_graphql::Error::new("first must be non-negative")),
        Some(n) => Ok(n.min(MAX_PAGE) as usize),
        None => Ok(DEFAULT_PAGE as usize),
    }
}

fn parse_cursor(after: Option<String>) -> async_graphql::Result<Option<u64>> {
    after
        .map(|s| {
            s.parse::<u64>()
                .map_err(|_| async_graphql::Error::new("invalid cursor"))
        })
        .transpose()
}

pub(crate) fn connection_complexity(first: Option<i32>, child_complexity: usize) -> usize {
    let rows = first.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE) as usize;
    child_complexity * rows.div_ceil(ROWS_PER_COMPLEXITY_UNIT)
}

#[derive(Default)]
pub(crate) struct AdminStatusQuery;

#[Object]
impl AdminStatusQuery {
    /// Latest host, process and server sample. Administrator only.
    async fn system_status(&self, ctx: &Context<'_>) -> async_graphql::Result<Arc<SystemStatus>> {
        access::require_admin(ctx)?;
        Ok(service(ctx)?.system_status())
    }

    /// Retained samples for sparklines, oldest first. Administrator only.
    async fn system_status_history(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<StatusSample>> {
        access::require_admin(ctx)?;
        Ok(service(ctx)?.status_history())
    }

    /// ffmpeg, hardware acceleration and codec capabilities.
    /// Administrator only.
    async fn transcoder_info(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Arc<TranscoderInfo>> {
        access::require_admin(ctx)?;
        Ok(service(ctx)?.transcoder_info().await)
    }

    /// Indexing health of every library. Administrator only.
    async fn scanner_status(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<LibraryIndexStatus>> {
        access::require_admin(ctx)?;
        let service = service(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let registry = ctx.data::<Arc<noflex_scanner::ScanRegistry>>().ok();
        let libraries = storage
            .list_libraries()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let mut out = Vec::with_capacity(libraries.len());
        for library in libraries {
            out.push(index_status(service.as_ref(), registry, library).await);
        }
        Ok(out)
    }

    /// Vector-index coverage for the active embedder. Administrator only.
    async fn search_index_status(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<SearchIndexStatus> {
        access::require_admin(ctx)?;
        service(ctx)?
            .search_index_status()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Recent server log events, newest first, from an in-memory ring
    /// (INFO and above; WARN/ERROR are retained longer). `minLevel` is
    /// an inclusive floor, `target` a module-path prefix, `contains` a
    /// case-insensitive match on message and field values. Lost on
    /// restart. Administrator only.
    #[graphql(complexity = "connection_complexity(first, child_complexity)")]
    async fn recent_logs(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        min_level: Option<LogLevel>,
        target: Option<String>,
        contains: Option<String>,
    ) -> async_graphql::Result<Connection<u64, LogEntry>> {
        access::require_admin(ctx)?;
        let service = service(ctx)?;
        let first = page_size(first)?;
        let before = parse_cursor(after)?;
        let filter = LogFilter {
            min_level,
            target_prefix: target.filter(|t| !t.is_empty()),
            contains: contains.filter(|c| !c.is_empty()),
        };
        let page = service.recent_logs(&filter, before, first);
        let mut conn = Connection::new(before.is_some(), page.has_more);
        conn.edges
            .extend(page.entries.into_iter().map(|e| Edge::new(e.seq, e)));
        Ok(conn)
    }

    /// Recent plugin lifecycle events (discovery, loads, load failures,
    /// unloads, traps), newest first. Lost on restart. Administrator
    /// only.
    #[graphql(complexity = "connection_complexity(first, child_complexity)")]
    async fn plugin_events(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<u64, PluginEventEntry, EmptyFields, EmptyFields>> {
        access::require_admin(ctx)?;
        let service = service(ctx)?;
        let first = page_size(first)?;
        let before = parse_cursor(after)?;
        let page = service.plugin_events(before, first);
        let mut conn = Connection::new(before.is_some(), page.has_more);
        conn.edges.extend(page.records.into_iter().map(|r| {
            Edge::new(
                r.seq,
                PluginEventEntry {
                    id: ID(r.seq.to_string()),
                    at: r.at,
                    event: r.event.into(),
                },
            )
        }));
        Ok(conn)
    }
}

async fn index_status(
    service: &dyn AdminStatusService,
    registry: Option<&Arc<noflex_scanner::ScanRegistry>>,
    library: Library,
) -> LibraryIndexStatus {
    let scan = match registry {
        Some(registry) => crate::scan_status_to_wire(registry.status(library.id).await),
        None => LibraryScanStatus::default(),
    };
    let activity = service.library_activity(&library).await;
    // A failed scan emits no completion event, so the observed run
    // would otherwise stay "active" forever; the registry's failure
    // record is what closes it.
    let failed_at = (scan.state == ScanState::Failed)
        .then_some(scan.finished_at.as_deref())
        .flatten()
        .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok());
    let active_scan = activity.active_scan.filter(|run| {
        !matches!((failed_at, run.started_at), (Some(failed), Some(started)) if failed >= started)
    });
    LibraryIndexStatus {
        library,
        scan,
        active_scan,
        last_scan: activity.last_scan,
        watcher: activity.watcher,
        roots: activity.roots,
        recent_failures: activity.recent_failures,
        failure_count: activity.failure_count,
    }
}

/// Body of the `systemStatusUpdates` subscription: the current sample
/// immediately, then one per sampler tick. Ends when the service drops.
pub(crate) fn system_status_stream(
    ctx: &Context<'_>,
) -> async_graphql::Result<Pin<Box<dyn Stream<Item = Arc<SystemStatus>> + Send + 'static>>> {
    access::require_admin(ctx)?;
    let mut rx = service(ctx)?.watch_system_status();
    Ok(Box::pin(async_stream::stream! {
        let current = rx.borrow_and_update().clone();
        yield current;
        while rx.changed().await.is_ok() {
            let next = rx.borrow_and_update().clone();
            yield next;
        }
    }))
}

/// Embedder plugins in dispatch order, for [`SearchIndexStatus`].
pub fn embedder_plugin_ids(registry: &PluginRegistry) -> Vec<String> {
    registry
        .exporters_of(HookKind::Embedder)
        .into_iter()
        .map(|id| id.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_levels_order_by_severity() {
        assert!(LogLevel::Error > LogLevel::Warn);
        assert!(LogLevel::Warn > LogLevel::Info);
        assert!(LogLevel::Debug > LogLevel::Trace);
    }

    #[test]
    fn connection_complexity_scales_per_page() {
        assert_eq!(connection_complexity(Some(10), 12), 12);
        assert_eq!(connection_complexity(Some(51), 12), 24);
        assert_eq!(connection_complexity(Some(10_000), 12), 120);
        assert_eq!(connection_complexity(None, 12), 24);
    }

    #[test]
    fn page_size_rejects_negative_and_caps() {
        assert!(page_size(Some(-1)).is_err());
        assert_eq!(page_size(Some(9999)).unwrap(), MAX_PAGE as usize);
        assert_eq!(page_size(None).unwrap(), DEFAULT_PAGE as usize);
    }
}
