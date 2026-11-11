// SPDX-License-Identifier: AGPL-3.0-only
//
// Background metadata worker: after every scan (and at startup and
// hourly) it matches unidentified works against the metadata providers,
// backfills provider artwork for identified works that have none, and asks
// each ratings plugin about the identified works it hasn't rated, which a
// plugin loaded after they were identified otherwise never would. It runs
// apart from the scan, so scans never wait on provider round trips. Works
// that found no match or no art are parked in memory for a day rather than
// re-asked every sweep; a ratings plugin that can't answer for now
// (rate-limited, no key) is left until the next.

use std::{collections::HashMap, sync::Arc};

use noflex_core::{
    WorkId, WorkKind,
    events::{DomainEvent, DomainEventBus},
};
use noflex_storage::{
    ListWorksQuery, SortOrder,
    domain::{ImageSource, Work},
};
use time::OffsetDateTime;
use tokio::{
    sync::{broadcast::error::RecvError, mpsc},
    task::JoinHandle,
    time::{Duration, Instant, interval_at},
};

use crate::{
    HookKind, PluginId,
    metadata_service::{AutoIdentify, MetadataService, RatingsFetch},
};

const SWEEP_PAGE: u32 = 200;
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const FAILURE_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60);

/// Identification order: a season or episode matches through its
/// series, so parents go first.
const TIERS: [&[WorkKind]; 3] = [
    &[
        WorkKind::Movie,
        WorkKind::Series,
        WorkKind::Artist,
        WorkKind::Album,
        WorkKind::AudioBook,
    ],
    &[WorkKind::Season],
    &[WorkKind::Episode],
];

/// Spawn the worker and its bus forwarder. Both live until aborted; a
/// closed bus only ends event triggering. Every pass re-checks for
/// metadata/image plugins, so a hot-reloaded provider is picked up
/// without a restart.
pub fn spawn_metadata_worker(
    service: Arc<MetadataService>,
    events: &DomainEventBus,
) -> JoinHandle<()> {
    let (tx, rx) = mpsc::channel::<()>(8);
    tokio::spawn(forward_events(events.clone(), service.clone(), tx.clone()));
    tokio::spawn(worker(MetadataWorker::new(service), rx, tx))
}

async fn forward_events(bus: DomainEventBus, service: Arc<MetadataService>, tx: mpsc::Sender<()>) {
    let mut rx = bus.subscribe();
    let indexing = service.indexing();
    loop {
        match rx.recv().await {
            Ok(DomainEvent::ScanStarted { library, .. }) => {
                indexing.scan_started(library, OffsetDateTime::now_utc());
            }
            Ok(DomainEvent::ScanCompleted { library, .. }) => {
                // Ahead of the scan ending, so nothing goes quiet between the two.
                if service.has_exporters(HookKind::Metadata) {
                    indexing.sweep_begun();
                }
                indexing.scan_finished(library);
                // A full channel already holds a pending sweep.
                if tx.try_send(()).is_err() {
                    indexing.sweep_ended();
                }
            }
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "metadata worker lagged; events dropped");
                indexing.forget_scans();
            }
            Err(RecvError::Closed) => break,
        }
    }
}

async fn worker(
    worker: MetadataWorker,
    mut rx: mpsc::Receiver<()>,
    // Keeps `rx` open after the forwarder exits so the hourly tick lives on.
    _keepalive: mpsc::Sender<()>,
) {
    let mut tick = interval_at(Instant::now() + SWEEP_INTERVAL, SWEEP_INTERVAL);
    worker.run_sweep().await;
    loop {
        tokio::select! {
            message = rx.recv() => match message {
                Some(()) => {
                    worker.run_sweep().await;
                    worker.service.indexing().sweep_ended();
                }
                None => break,
            },
            _ = tick.tick() => {
                worker.run_sweep().await;
            }
        }
    }
}

/// Totals of one [`MetadataWorker::run_sweep`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Works matched, fetched and applied.
    pub identified: u32,
    /// Works searched without a confident match (then parked).
    pub unmatched: u32,
    /// Provider images stored by the artwork backfill.
    pub artwork_written: u32,
    /// Works a ratings plugin rated for the first time, once per plugin.
    pub rated: u32,
}

/// One sweep's logic, separate from the scheduling so tests can drive it.
pub struct MetadataWorker {
    service: Arc<MetadataService>,
    identify_backoff: std::sync::Mutex<HashMap<WorkId, Instant>>,
    art_backoff: std::sync::Mutex<HashMap<WorkId, Instant>>,
    rating_backoff: std::sync::Mutex<HashMap<(PluginId, WorkId), Instant>>,
}

impl MetadataWorker {
    /// A worker with empty backoff memory.
    pub fn new(service: Arc<MetadataService>) -> Self {
        Self {
            service,
            identify_backoff: Default::default(),
            art_backoff: Default::default(),
            rating_backoff: Default::default(),
        }
    }

    /// Identify what can be identified, then backfill missing provider art,
    /// then ratings. Storage errors end the affected pass early and are
    /// logged.
    pub async fn run_sweep(&self) -> SweepReport {
        let mut report = SweepReport::default();
        if self.service.has_exporters(HookKind::Metadata) {
            self.service.indexing().sweep_begun();
            self.identify_pass(&mut report).await;
            self.service.indexing().sweep_ended();
        }
        if self.service.has_exporters(HookKind::Image) {
            self.artwork_pass(&mut report).await;
        }
        if self.service.has_exporters(HookKind::Ratings) {
            self.ratings_pass(&mut report).await;
        }
        if report != SweepReport::default() {
            tracing::info!(
                identified = report.identified,
                unmatched = report.unmatched,
                artwork = report.artwork_written,
                rated = report.rated,
                "metadata sweep finished"
            );
        }
        report
    }

    async fn identify_pass(&self, report: &mut SweepReport) {
        let storage = self.service.storage().clone();
        let libraries = match storage.list_libraries().await {
            Ok(libraries) => libraries,
            Err(error) => {
                tracing::warn!(%error, "metadata sweep: listing libraries failed");
                return;
            }
        };
        for library in libraries {
            for kinds in TIERS {
                let mut cursor = None;
                loop {
                    let page = match storage
                        .list_works(&ListWorksQuery {
                            library: Some(library.id),
                            kinds: kinds.to_vec(),
                            recursive: true,
                            sort: SortOrder::DateAddedAsc,
                            limit: SWEEP_PAGE,
                            cursor,
                            ..Default::default()
                        })
                        .await
                    {
                        Ok(page) => page,
                        Err(error) => {
                            tracing::warn!(library = %library.id, %error, "metadata sweep: listing works failed");
                            break;
                        }
                    };
                    for work in page.items.iter().filter(|w| w.awaits_identification()) {
                        self.identify_one(work, report).await;
                    }
                    cursor = page.next_cursor();
                    if cursor.is_none() {
                        break;
                    }
                }
            }
        }
    }

    async fn identify_one(&self, work: &Work, report: &mut SweepReport) {
        if backed_off(&self.identify_backoff, &work.id) {
            return;
        }
        let answer = self.service.auto_identify(work.id).await;
        match &answer {
            Ok(AutoIdentify::Matched { provider, .. }) => {
                tracing::debug!(work = %work.id, provider = %provider.0, "auto-identified");
                report.identified += 1;
            }
            // The series isn't identified yet; it may be next sweep.
            Ok(AutoIdentify::Skipped) => {}
            Ok(AutoIdentify::NoMatch) => {
                report.unmatched += 1;
                park(&self.identify_backoff, work.id);
            }
            Err(error) => {
                tracing::warn!(work = %work.id, %error, "auto-identify failed");
                park(&self.identify_backoff, work.id);
            }
        }
        if !matches!(answer, Ok(AutoIdentify::Matched { .. })) {
            self.service.indexing().asked(work.id);
        }
    }

    async fn artwork_pass(&self, report: &mut SweepReport) {
        let storage = self.service.storage().clone();
        let mut cursor = None;
        loop {
            let page = match storage
                .identified_works_missing_image_source(ImageSource::Provider, SWEEP_PAGE, cursor)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "artwork sweep listing failed");
                    return;
                }
            };
            for work in &page.items {
                if backed_off(&self.art_backoff, &work.id) {
                    continue;
                }
                match self.service.backfill_artwork(work).await {
                    Ok(0) => park(&self.art_backoff, work.id),
                    Ok(written) => report.artwork_written += written,
                    Err(error) => {
                        tracing::warn!(work = %work.id, %error, "artwork backfill failed");
                        park(&self.art_backoff, work.id);
                    }
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return,
            }
        }
    }

    async fn ratings_pass(&self, report: &mut SweepReport) {
        for plugin in self.service.ratings_plugins() {
            self.rate_unrated(&plugin, report).await;
        }
    }

    /// Ask `plugin` about every identified work it hasn't rated, until it
    /// can't answer for now.
    async fn rate_unrated(&self, plugin: &crate::LoadedPlugin, report: &mut SweepReport) {
        let storage = self.service.storage().clone();
        let mut cursor = None;
        loop {
            let page = match storage
                .identified_works_unrated_by(&plugin.id.0, SWEEP_PAGE, cursor)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(plugin = %plugin.id.0, %error, "ratings sweep listing failed");
                    return;
                }
            };
            for work in &page.items {
                let key = (plugin.id.clone(), work.id);
                if backed_off(&self.rating_backoff, &key) {
                    continue;
                }
                match self.service.rate(plugin, work).await {
                    RatingsFetch::Stored => report.rated += 1,
                    RatingsFetch::Failed => park(&self.rating_backoff, key),
                    RatingsFetch::Unavailable => {
                        tracing::info!(plugin = %plugin.id.0,
                            "ratings sweep: plugin can't answer for now; the rest waits for the next sweep");
                        return;
                    }
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return,
            }
        }
    }
}

fn backed_off<K: Eq + std::hash::Hash>(
    map: &std::sync::Mutex<HashMap<K, Instant>>,
    key: &K,
) -> bool {
    let Ok(mut map) = map.lock() else {
        return false;
    };
    match map.get(key) {
        Some(until) if *until > Instant::now() => true,
        Some(_) => {
            map.remove(key);
            false
        }
        None => false,
    }
}

fn park<K: Eq + std::hash::Hash>(map: &std::sync::Mutex<HashMap<K, Instant>>, key: K) {
    if let Ok(mut map) = map.lock() {
        map.insert(key, Instant::now() + FAILURE_BACKOFF);
    }
}
