// SPDX-License-Identifier: AGPL-3.0-only
//
// Event-driven state the admin view needs but no subsystem keeps:
// plugin lifecycle history, observed scan runs (the scan registry only
// sees manual scans; watcher rescans bypass it, but every scan emits
// domain events), and per-library watcher state. Subscribes at
// construction so events emitted during boot are not missed.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

use noflex_core::{DomainEvent, DomainEventBus, LibraryId};
use noflex_graphql::admin_status::{
    IndexScanRun, LibraryWatcherStatus, PluginEventPage, PluginEventRecord, WatcherState,
};
use noflex_plugin_host::{LifecycleBus, PluginLifecycleEvent};
use noflex_storage::{Storage, domain::ActivityKind};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::broadcast::error::RecvError;

use super::logs::LogBuffer;
use crate::metrics::Metrics;

const PLUGIN_EVENT_CAPACITY: usize = 500;
/// Completed-scan rows read back from the activity log at boot.
const PERSISTED_SCAN_LOOKBACK: u32 = 500;

/// `detail` JSON of the `scan.completed` activity rows this recorder
/// writes; also how the last run is recovered after a restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ScanDetail {
    library_name: String,
    started_at: Option<String>,
    added: u32,
    updated: u32,
    removed: u32,
    failures: u32,
}

#[derive(Debug, Clone, Default)]
struct LibraryTrack {
    active: Option<IndexScanRun>,
    last: Option<IndexScanRun>,
    watcher: LibraryWatcherStatus,
}

pub struct Recorder {
    plugin_events: Mutex<VecDeque<PluginEventRecord>>,
    next_seq: AtomicU64,
    load_failures: AtomicU32,
    traps: AtomicU32,
    libraries: Mutex<HashMap<LibraryId, LibraryTrack>>,
    storage: Arc<dyn Storage>,
    logs: Arc<LogBuffer>,
    metrics: Arc<Metrics>,
}

impl Recorder {
    /// Subscribe to both buses now and spawn the consumers. Also loads
    /// the last completed scan per library from the activity log.
    pub fn spawn(
        lifecycle: &LifecycleBus,
        events: &DomainEventBus,
        storage: Arc<dyn Storage>,
        logs: Arc<LogBuffer>,
        metrics: Arc<Metrics>,
    ) -> Arc<Self> {
        let recorder = Arc::new(Self {
            plugin_events: Mutex::new(VecDeque::with_capacity(64)),
            next_seq: AtomicU64::new(1),
            load_failures: AtomicU32::new(0),
            traps: AtomicU32::new(0),
            libraries: Mutex::new(HashMap::new()),
            storage,
            logs,
            metrics,
        });
        let mut lifecycle_rx = lifecycle.subscribe();
        let mut domain_rx = events.subscribe();

        let r = recorder.clone();
        tokio::spawn(async move {
            loop {
                match lifecycle_rx.recv().await {
                    Ok(ev) => r.record_plugin_event(ev),
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let r = recorder.clone();
        tokio::spawn(async move {
            loop {
                match domain_rx.recv().await {
                    Ok(ev) => r.on_domain_event(ev).await,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let r = recorder.clone();
        tokio::spawn(async move { r.load_persisted_scans().await });
        recorder
    }

    fn libs(&self) -> std::sync::MutexGuard<'_, HashMap<LibraryId, LibraryTrack>> {
        self.libraries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn events(&self) -> std::sync::MutexGuard<'_, VecDeque<PluginEventRecord>> {
        self.plugin_events.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn record_plugin_event(&self, event: PluginLifecycleEvent) {
        match &event {
            PluginLifecycleEvent::LoadFailed { .. } => {
                self.load_failures.fetch_add(1, Ordering::Relaxed);
            }
            PluginLifecycleEvent::Trapped { .. } => {
                self.traps.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        let record = PluginEventRecord {
            seq: self.next_seq.fetch_add(1, Ordering::Relaxed),
            at: OffsetDateTime::now_utc(),
            event,
        };
        let mut ring = self.events();
        while ring.len() >= PLUGIN_EVENT_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(record);
    }

    /// `LOAD_FAILED` and `TRAPPED` counts since startup.
    pub fn plugin_failure_counts(&self) -> (u32, u32) {
        (
            self.load_failures.load(Ordering::Relaxed),
            self.traps.load(Ordering::Relaxed),
        )
    }

    pub fn plugin_events(&self, before: Option<u64>, first: usize) -> PluginEventPage {
        let ring = self.events();
        let mut hits = ring
            .iter()
            .rev()
            .filter(|r| before.is_none_or(|b| r.seq < b));
        let records: Vec<PluginEventRecord> = hits.by_ref().take(first).cloned().collect();
        let has_more = hits.next().is_some();
        PluginEventPage { records, has_more }
    }

    /// Record the outcome of starting `library`'s filesystem watcher.
    pub fn watcher_started(&self, library: LibraryId, result: Result<(), String>) {
        let watcher = match result {
            Ok(()) => LibraryWatcherStatus {
                state: WatcherState::Active,
                since: Some(OffsetDateTime::now_utc()),
                error: None,
            },
            Err(e) => LibraryWatcherStatus {
                state: WatcherState::Failed,
                since: Some(OffsetDateTime::now_utc()),
                error: Some(e),
            },
        };
        self.libs().entry(library).or_default().watcher = watcher;
    }

    /// Record that `library` has no watcher (disabled or deleted).
    pub fn watcher_stopped(&self, library: LibraryId) {
        self.libs().entry(library).or_default().watcher = LibraryWatcherStatus {
            state: WatcherState::NotStarted,
            since: Some(OffsetDateTime::now_utc()),
            error: None,
        };
    }

    /// Scan-run and watcher state for `library`.
    pub fn library(
        &self,
        library: LibraryId,
    ) -> (
        Option<IndexScanRun>,
        Option<IndexScanRun>,
        LibraryWatcherStatus,
    ) {
        let libs = self.libs();
        let track = libs.get(&library).cloned().unwrap_or_default();
        (track.active, track.last, track.watcher)
    }

    pub fn active_scans(&self) -> u32 {
        let n = self.libs().values().filter(|t| t.active.is_some()).count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    async fn on_domain_event(&self, event: DomainEvent) {
        match event {
            DomainEvent::ScanStarted {
                library,
                library_name,
            } => {
                let now = OffsetDateTime::now_utc();
                self.libs().entry(library).or_default().active = Some(IndexScanRun {
                    started_at: Some(now),
                    ..IndexScanRun::default()
                });
                self.append_activity(ActivityKind::ScanStarted, library, &library_name, None)
                    .await;
            }
            DomainEvent::WorkAdded { library, .. } => {
                if let Some(run) = self
                    .libs()
                    .get_mut(&library)
                    .and_then(|t| t.active.as_mut())
                {
                    run.added = run.added.saturating_add(1);
                }
            }
            DomainEvent::ScanCompleted {
                library,
                library_name,
                added,
                updated,
                removed,
                ..
            } => {
                let finished = OffsetDateTime::now_utc();
                let started = self
                    .libs()
                    .get(&library)
                    .and_then(|t| t.active.as_ref())
                    .and_then(|r| r.started_at);
                let roots = match self.storage.get_library(library).await {
                    Ok(Some(lib)) => lib.paths,
                    _ => Vec::new(),
                };
                let failures = started.map_or(0, |from| {
                    self.logs.scan_failures_between(&roots, from, finished)
                });
                let run = IndexScanRun {
                    started_at: started,
                    finished_at: Some(finished),
                    added: saturating_u32(added),
                    updated: saturating_u32(updated),
                    removed: saturating_u32(removed),
                    failures,
                };
                {
                    let mut libs = self.libs();
                    let track = libs.entry(library).or_default();
                    track.active = None;
                    track.last = Some(run.clone());
                }
                self.metrics.inc_scan_completed();
                let detail = ScanDetail {
                    library_name: library_name.clone(),
                    started_at: started.and_then(|s| {
                        s.format(&time::format_description::well_known::Rfc3339)
                            .ok()
                    }),
                    added: run.added,
                    updated: run.updated,
                    removed: run.removed,
                    failures,
                };
                self.append_activity(
                    ActivityKind::ScanCompleted,
                    library,
                    &library_name,
                    Some(&detail),
                )
                .await;
            }
            _ => {}
        }
    }

    async fn append_activity(
        &self,
        kind: ActivityKind,
        library: LibraryId,
        library_name: &str,
        detail: Option<&ScanDetail>,
    ) {
        let detail = match detail {
            Some(d) => serde_json::to_string(d).ok(),
            None => {
                serde_json::to_string(&serde_json::json!({ "library_name": library_name })).ok()
            }
        };
        let subject = library.to_string();
        if let Err(e) = self
            .storage
            .append_activity(
                kind,
                None,
                Some(&subject),
                detail.as_deref(),
                OffsetDateTime::now_utc(),
            )
            .await
        {
            tracing::warn!(error = %e, library = %library, "recording scan activity failed");
        }
    }

    /// Seed `last` from the newest `scan.completed` row per library so
    /// "last scanned" survives restarts. Never overwrites a run already
    /// observed live.
    async fn load_persisted_scans(&self) {
        let page = match self
            .storage
            .list_activity(
                &[ActivityKind::ScanCompleted],
                None,
                PERSISTED_SCAN_LOOKBACK,
                None,
            )
            .await
        {
            Ok(page) => page,
            Err(e) => {
                tracing::warn!(error = %e, "loading persisted scan history failed");
                return;
            }
        };
        let mut libs = self.libs();
        // Newest first, so the first row seen per library wins.
        for row in page.items {
            let Some(library) = row
                .subject
                .as_deref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(LibraryId::from_uuid)
            else {
                continue;
            };
            let track = libs.entry(library).or_default();
            if track.last.is_none() {
                track.last = Some(run_from_activity(row.occurred_at, row.detail.as_deref()));
            }
        }
    }
}

fn saturating_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn run_from_activity(occurred_at: OffsetDateTime, detail: Option<&str>) -> IndexScanRun {
    let detail = detail.and_then(|d| serde_json::from_str::<ScanDetail>(d).ok());
    let started_at = detail
        .as_ref()
        .and_then(|d| d.started_at.as_deref())
        .and_then(|s| {
            OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
        });
    IndexScanRun {
        started_at,
        finished_at: Some(occurred_at),
        added: detail.as_ref().map_or(0, |d| d.added),
        updated: detail.as_ref().map_or(0, |d| d.updated),
        removed: detail.as_ref().map_or(0, |d| d.removed),
        failures: detail.as_ref().map_or(0, |d| d.failures),
    }
}

#[cfg(test)]
mod tests {
    use noflex_plugin_host::PluginId;
    use noflex_storage_turso::TursoStore;

    use super::*;

    async fn recorder() -> (
        Arc<Recorder>,
        LifecycleBus,
        DomainEventBus,
        Arc<dyn Storage>,
    ) {
        let store = TursoStore::open_temp().await.expect("temp store");
        store.migrate().await.expect("migrate");
        let storage: Arc<dyn Storage> = Arc::new(store);
        let lifecycle = LifecycleBus::default();
        let events = DomainEventBus::default();
        let r = Recorder::spawn(
            &lifecycle,
            &events,
            storage.clone(),
            Arc::new(LogBuffer::default()),
            Metrics::new(),
        );
        (r, lifecycle, events, storage)
    }

    async fn settle() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn plugin_events_page_newest_first_and_count_failures() {
        let (r, bus, _, _) = recorder().await;
        bus.emit(PluginLifecycleEvent::LoadFailed {
            path: "/p/a.wasm".into(),
            reason: "bad".into(),
        });
        bus.emit(PluginLifecycleEvent::Trapped {
            plugin_id: PluginId("tmdb".into()),
            hook: noflex_plugin_host::HookKind::Metadata,
            trap: "unreachable".into(),
        });
        settle().await;
        let page = r.plugin_events(None, 1);
        assert_eq!(page.records.len(), 1);
        assert!(page.has_more);
        assert!(matches!(
            page.records[0].event,
            PluginLifecycleEvent::Trapped { .. }
        ));
        let older = r.plugin_events(Some(page.records[0].seq), 10);
        assert_eq!(older.records.len(), 1);
        assert!(!older.has_more);
        assert_eq!(r.plugin_failure_counts(), (1, 1));
    }

    #[tokio::test]
    async fn scan_runs_are_tracked_and_persisted() {
        let (r, _, events, storage) = recorder().await;
        let lib = LibraryId::new();
        events.emit(DomainEvent::ScanStarted {
            library: lib,
            library_name: "Movies".into(),
        });
        settle().await;
        events.emit(DomainEvent::WorkAdded {
            work: noflex_core::WorkId::new(),
            library: lib,
            title: "A".into(),
        });
        settle().await;
        let (active, last, _) = r.library(lib);
        assert_eq!(active.map(|a| a.added), Some(1));
        assert!(last.is_none());
        assert_eq!(r.active_scans(), 1);

        events.emit(DomainEvent::ScanCompleted {
            library: lib,
            library_name: "Movies".into(),
            added: 3,
            updated: 2,
            removed: 1,
            added_works: Vec::new(),
            updated_works: Vec::new(),
            removed_works: Vec::new(),
        });
        settle().await;
        let (active, last, _) = r.library(lib);
        assert!(active.is_none());
        let last = last.expect("last run");
        assert_eq!((last.added, last.updated, last.removed), (3, 2, 1));
        assert!(last.started_at.is_some());

        // A fresh recorder over the same storage recovers the run.
        let reborn = Recorder::spawn(
            &LifecycleBus::default(),
            &DomainEventBus::default(),
            storage,
            Arc::new(LogBuffer::default()),
            Metrics::new(),
        );
        settle().await;
        let (_, recovered, _) = reborn.library(lib);
        let recovered = recovered.expect("persisted run");
        assert_eq!((recovered.added, recovered.updated), (3, 2));
        assert!(recovered.started_at.is_some());
    }

    #[test]
    fn watcher_outcomes_are_recorded() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let (r, _, _, _) = recorder().await;
            let ok = LibraryId::new();
            let bad = LibraryId::new();
            r.watcher_started(ok, Ok(()));
            r.watcher_started(bad, Err("EACCES".into()));
            assert_eq!(r.library(ok).2.state, WatcherState::Active);
            let failed = r.library(bad).2;
            assert_eq!(failed.state, WatcherState::Failed);
            assert_eq!(failed.error.as_deref(), Some("EACCES"));
            assert_eq!(
                r.library(LibraryId::new()).2.state,
                WatcherState::NotStarted
            );
        });
    }
}
