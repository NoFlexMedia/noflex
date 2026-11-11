// SPDX-License-Identifier: AGPL-3.0-only
//
// Host-driven intro/credits detection worker: keeps edition markers
// filled by sweeping editions that have never been through detection,
// calling each loaded intro-detector plugin's `detect` hook, and
// persisting the outcome itself — plugins never see storage (detection
// inputs reach them through `host-library.lookup-edition-by-id`).
// Mirrors the embedding worker's shape: a startup sweep, a
// `ScanCompleted` trigger, and an hourly tick so a silently-dropped
// event or a hot-reloaded detector eventually converges.
//
// Scan rows are per (edition, plugin), so detectors progress
// independently: an edition leaves one plugin's missing list after its
// first recorded scan while remaining visible to the others.

use std::sync::Arc;

use noflex_core::events::{DomainEvent, DomainEventBus};
use noflex_storage::{
    Storage,
    domain::{Edition, IntroMarkers},
};
use tokio::{
    sync::{broadcast::error::RecvError, mpsc},
    task::JoinHandle,
    time::{Duration, Instant, interval_at},
};

use crate::{HookKind, LoadedPlugin, PluginRegistry};

/// Sweep page size; one `detect` call per edition keeps a pass
/// resumable at page granularity.
const SWEEP_PAGE: u32 = 64;

/// Idle tick that re-runs the sweep so a dropped `ScanCompleted` or a
/// hot-reloaded detector eventually converges. Detection results are
/// recorded per edition, so a re-sweep only costs one page query per
/// plugin when everything is already done.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Spawn the intro-detection worker and its bus forwarder. The returned
/// handle is the worker's; both tasks exit only when aborted — a closing
/// event bus just ends event-driven triggering, the hourly sweep keeps
/// running. The worker spawns unconditionally and re-checks for
/// intro-detector plugins on every pass (mirroring the embedding
/// worker), so a hot-reload install later picks up work without a
/// restart.
pub fn spawn_intro_detector_worker(
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    events: &DomainEventBus,
) -> JoinHandle<()> {
    let (tx, rx) = mpsc::channel::<()>(8);
    tokio::spawn(forward_events(events.clone(), tx.clone()));
    tokio::spawn(worker(registry, storage, rx, tx))
}

/// Forwarder: subscribes to the domain bus and turns a finished scan
/// into a sweep trigger. Lagging drops events — tolerable, since the
/// hourly tick is the backstop.
async fn forward_events(bus: DomainEventBus, tx: mpsc::Sender<()>) {
    let mut rx = bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(DomainEvent::ScanCompleted { .. }) => {
                // A full channel means a sweep is already queued; the
                // hourly tick is the backstop, so dropping beats
                // blocking the bus for everyone.
                if let Err(e) = tx.try_send(()) {
                    tracing::warn!(error = %e, "intro-detector sweep trigger dropped");
                }
            }
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "intro-detector worker lagged; events dropped");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

async fn worker(
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    mut rx: mpsc::Receiver<()>,
    // Kept so `rx` never reports closed after the forwarder exits (bus
    // dropped); the worker lives on its hourly tick.
    _keepalive: mpsc::Sender<()>,
) {
    let mut tick = interval_at(Instant::now() + SWEEP_INTERVAL, SWEEP_INTERVAL);

    // Startup sweep: detect everything missed while the server was down.
    run_sweep(&registry, &storage).await;

    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(()) => run_sweep(&registry, &storage).await,
                None => break,
            },
            _ = tick.tick() => run_sweep(&registry, &storage).await,
        }
    }
}

/// One pass over every loaded intro-detector plugin. Plugins sweep
/// sequentially — each drives its own single instance, and one slow or
/// trapping detector never starves the others of a pass boundary.
async fn run_sweep(registry: &PluginRegistry, storage: &Arc<dyn Storage>) {
    let detectors: Vec<Arc<LoadedPlugin>> = registry
        .exporters_of(HookKind::IntroDetector)
        .into_iter()
        .filter_map(|id| registry.get_plugin(&id))
        .collect();
    if detectors.is_empty() {
        tracing::debug!("intro-detection sweep skipped: no intro-detector plugin loaded");
        return;
    }
    for plugin in detectors {
        sweep_plugin(&plugin, storage).await;
    }
}

/// Sweep one plugin's missing editions to completion, one page at a
/// time. A listing failure aborts the pass — unrecorded editions stay
/// "missing" and the next trigger retries, so a storage hiccup never
/// tight-loops.
async fn sweep_plugin(plugin: &Arc<LoadedPlugin>, storage: &Arc<dyn Storage>) {
    let plugin_id = plugin.id.0.clone();
    let mut cursor = None;
    loop {
        let page = match storage
            .editions_missing_intro_scan(&plugin_id, SWEEP_PAGE, cursor)
            .await
        {
            Ok(page) => page,
            Err(e) => {
                tracing::warn!(plugin = %plugin_id, error = %e,
                    "intro-detection sweep: listing missing editions failed");
                return;
            }
        };
        if page.items.is_empty() {
            return;
        }
        let next = page.next_cursor;
        for edition in page.items {
            detect_one(plugin, storage, edition).await;
        }
        match next {
            Some(c) => cursor = Some(c),
            None => return,
        }
    }
}

/// Detect one edition and persist the outcome. `Some(detection)` writes
/// the markers, unless another detector already found some with more
/// confidence, and records a found scan; `None` records a not-found scan.
/// Ties keep the markers already there. A plugin error records nothing —
/// the edition stays missing and is retried on the next sweep. Persist
/// failures are logged and leave the edition missing for the same reason.
async fn detect_one(plugin: &Arc<LoadedPlugin>, storage: &Arc<dyn Storage>, edition: Edition) {
    let plugin_id = plugin.id.0.as_str();
    let detection = match plugin.call_detect_intro(edition.id.to_string()).await {
        Ok(Ok(detection)) => detection,
        Ok(Err(e)) => {
            tracing::warn!(plugin = %plugin_id, edition = %edition.id, error = ?e,
                "intro detector returned an error");
            return;
        }
        Err(e) => {
            tracing::warn!(plugin = %plugin_id, edition = %edition.id, error = %e,
                "intro-detector dispatch failed");
            return;
        }
    };
    match detection {
        Some(d) => {
            let rival = match storage.rival_intro_confidence(edition.id, plugin_id).await {
                Ok(rival) => rival,
                Err(e) => {
                    tracing::warn!(plugin = %plugin_id, edition = %edition.id, error = %e,
                        "intro-detection: rival lookup failed");
                    return;
                }
            };
            if rival.is_none_or(|r| d.confidence > r) {
                let markers = IntroMarkers {
                    intro_start_ms: d.intro_start_ms,
                    intro_end_ms: d.intro_end_ms,
                    credits_start_ms: d.credits_start_ms,
                };
                if let Err(e) = storage.set_intro_markers(edition.id, markers).await {
                    tracing::warn!(plugin = %plugin_id, edition = %edition.id, error = %e,
                        "intro-detection: marker persist failed");
                    return;
                }
            }
            if let Err(e) = storage
                .record_intro_scan(edition.id, plugin_id, Some(d.confidence))
                .await
            {
                tracing::warn!(plugin = %plugin_id, edition = %edition.id, error = %e,
                    "intro-detection: scan record failed");
            }
        }
        None => {
            if let Err(e) = storage.record_intro_scan(edition.id, plugin_id, None).await {
                tracing::warn!(plugin = %plugin_id, edition = %edition.id, error = %e,
                    "intro-detection: scan record failed");
            }
        }
    }
}
