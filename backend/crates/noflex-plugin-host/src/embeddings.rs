// SPDX-License-Identifier: AGPL-3.0-only
//
// Host-driven embedding worker: keeps the vector index filled by
// computing embeddings for works through the highest-priority embedder
// plugin and persisting them itself. The WIT task protocol has no
// channel for a guest to hand vectors back (outcomes are
// `done|more(cursor)`), so the sweep lives host-side: the worker calls
// the plugin's `embed` hook via [`embed_inputs`] and writes through
// `Storage::upsert_embedding`.
//
// A transformer costs real CPU, and search works without vectors in the
// meantime, so indexing yields to everything else on the host: it embeds one
// small batch at a time, and only while the rest of the machine is quiet.
// There is one feed, `works_missing_embedding`, so a failure or a dropped event
// only leaves work for the next pass. Events and an hourly tick just start one.

use std::{sync::Arc, time::Duration};

use noflex_core::{
    EmbeddingTarget, TagTarget,
    events::{DomainEvent, DomainEventBus},
};
use noflex_storage::{
    Storage,
    domain::{EmbeddingModel, TagAssignment, Work},
};
use sysinfo::{CpuRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::{
    sync::{Notify, broadcast::error::RecvError},
    task::JoinHandle,
    time::{Instant, interval_at, sleep},
};

use crate::{EmbedDocument, EmbedInput, PluginRegistry, registry::embed_inputs};

/// Quiet period after the first event of a burst before a pass starts — scans
/// add works in bursts, and the pass finds them all at once.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// Idle tick that re-runs the sweep so a silently-dropped event or a
/// hot-reloaded embedder eventually converges.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How long a CPU reading averages over. Longer than a burst of page loads,
/// short enough to back off soon after a transcode starts.
const LOAD_WINDOW: Duration = Duration::from_secs(2);

/// Works per call to a remote API; the `embed` dispatch cap.
const REMOTE_BATCH: u32 = 64;

/// Rough cap on the text sent to an embedder, in bytes (≈chars for the
/// Latin-heavy titles/overviews this shapes). Long overviews add noise,
/// not signal, past this point.
const MAX_TEXT_BYTES: usize = 1000;

/// Top-billed people a work's text names; the rest of a long cast is noise, and
/// lexical search finds them regardless.
const MAX_PEOPLE: usize = 6;

/// How the worker shares the host. See `search.*` in `docs/CONFIGURATION.md`.
#[derive(Debug, Clone)]
pub struct EmbeddingWorkerConfig {
    /// Works embedded per `embed` call, at most the 64-input dispatch cap. A
    /// batch can't be interrupted, so this bounds how long a search or a
    /// transcode waits for the CPU back.
    pub batch_size: u32,
    /// Share of the host's CPU that other work may be using, in percent, for a
    /// batch to start.
    pub busy_percent: f32,
    /// How long a host that never quiets can hold off a batch, so a machine busy
    /// around the clock still gets indexed, slowly.
    pub max_wait: Duration,
}

impl Default for EmbeddingWorkerConfig {
    fn default() -> Self {
        Self {
            batch_size: 8,
            busy_percent: 30.0,
            max_wait: Duration::from_secs(10 * 60),
        }
    }
}

/// Spawn the embedding worker and its bus forwarder. The returned
/// handle is the worker's; both tasks exit only when aborted — a
/// closing event bus just ends event-driven feeding, the hourly sweep
/// keeps running.
pub fn spawn_embedding_worker(
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    events: &DomainEventBus,
    config: EmbeddingWorkerConfig,
) -> JoinHandle<()> {
    let wake = Arc::new(Notify::new());
    tokio::spawn(forward_events(events.clone(), wake.clone()));
    tokio::spawn(worker(registry, storage, wake, config))
}

/// Forwarder: subscribes to the domain bus and wakes the worker on the two
/// relevant events. Lagging drops events — tolerable, since the next pass reads
/// what's missing rather than what was announced, and lag itself means there's
/// news.
async fn forward_events(bus: DomainEventBus, wake: Arc<Notify>) {
    let mut rx = bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(DomainEvent::WorkAdded { .. } | DomainEvent::ScanCompleted { .. }) => {
                wake.notify_one();
            }
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => wake.notify_one(),
            Err(RecvError::Closed) => break,
        }
    }
}

async fn worker(
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    wake: Arc<Notify>,
    config: EmbeddingWorkerConfig,
) {
    let mut gate = IdleGate::new(HostLoad::new(), config.busy_percent, config.max_wait);
    let mut tick = interval_at(Instant::now() + SWEEP_INTERVAL, SWEEP_INTERVAL);
    let changed = registry.embedder_changed();

    // Startup sweep: backfill everything missing from before we booted.
    run_sweep(&registry, &storage, &mut gate, config.batch_size).await;

    loop {
        tokio::select! {
            () = wake.notified() => sleep(DEBOUNCE).await,
            () = changed.notified() => {}
            _ = tick.tick() => {}
        }
        run_sweep(&registry, &storage, &mut gate, config.batch_size).await;
    }
}

/// CPU use by everything but this process's own, as a reading averaged since
/// the last one. A trait so tests can script a host.
trait Load: Send {
    /// In percent of the whole host. Meaningful from the second call on.
    fn others_percent(&mut self) -> f32;
}

/// [`Load`] read from the OS. Counts ffmpeg as other work, since it's a child
/// process, and excludes the batch that just ran, which would otherwise keep the
/// worker from ever starting another.
struct HostLoad {
    system: Option<System>,
    pid: Option<Pid>,
}

impl HostLoad {
    /// Never fails: a platform sysinfo can't read reports an idle host, which
    /// degrades to embedding without yielding.
    fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_usage()),
        )
        .ok();
        Self {
            system,
            pid: sysinfo::get_current_pid().ok(),
        }
    }
}

impl Load for HostLoad {
    fn others_percent(&mut self) -> f32 {
        let Some(system) = self.system.as_mut() else {
            return 0.0;
        };
        system.refresh_cpu_specifics(CpuRefreshKind::nothing().with_usage());
        let mut own = 0.0;
        if let Some(pid) = self.pid {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_cpu(),
            );
            let cores = system.cpus().len().max(1) as f32;
            own = system.process(pid).map_or(0.0, |p| p.cpu_usage() / cores);
        }
        (system.global_cpu_usage() - own).max(0.0)
    }
}

/// Holds batches back while the host is busy.
struct IdleGate<L> {
    load: L,
    busy_percent: f32,
    max_wait: Duration,
}

impl<L: Load> IdleGate<L> {
    fn new(load: L, busy_percent: f32, max_wait: Duration) -> Self {
        Self {
            load,
            busy_percent,
            max_wait,
        }
    }

    /// Returns once a full [`LOAD_WINDOW`] has passed with other work under the
    /// threshold, or after [`Self::max_wait`] regardless.
    async fn wait(&mut self) {
        let started = Instant::now();
        loop {
            // Discards whatever the last batch or reading left in the average.
            self.load.others_percent();
            sleep(LOAD_WINDOW).await;
            let others = self.load.others_percent();
            if others < self.busy_percent {
                return;
            }
            if started.elapsed() >= self.max_wait {
                tracing::debug!(others, "embedding: host never quieted; embedding anyway");
                return;
            }
            tracing::trace!(others, "embedding: waiting for the host to quiet");
        }
    }
}

/// One backfill pass over `works_missing_embedding`, a batch at a time as the
/// host allows. Aborts the pass on the first embed or storage failure — failed
/// works remain "missing", and the next trigger (event or hourly tick) retries,
/// so a down embedder never tight-loops.
async fn run_sweep<L: Load>(
    registry: &PluginRegistry,
    storage: &Arc<dyn Storage>,
    gate: &mut IdleGate<L>,
    batch_size: u32,
) {
    let Some(claim) = registry.embedder_claim() else {
        tracing::debug!("embedding sweep skipped: no embedder plugin loaded");
        return;
    };
    let model = EmbeddingModel {
        namespace: claim.namespace.clone(),
        model_id: claim.model_id.clone(),
        dimensions: claim.dimensions,
    };
    match storage.activate_embedding_model(&model).await {
        Ok(Some(previous)) => tracing::info!(
            from = %previous.model_id,
            to = %model.model_id,
            namespace = %model.namespace,
            "embedding model changed; reindexing"
        ),
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(error = %e, "embedding sweep: recording the model failed");
            return;
        }
    }
    // An API charges by the text and spends none of this host's CPU, so it gets
    // the largest batches and doesn't wait its turn.
    let remote = registry.remote_embedder().is_some();
    let batch_size = if remote { REMOTE_BATCH } else { batch_size };
    let mut cursor = None;
    loop {
        let page = match storage
            .works_missing_embedding(&claim.namespace, batch_size, cursor)
            .await
        {
            Ok(page) => page,
            Err(e) => {
                tracing::warn!(error = %e, "embedding sweep: listing missing works failed");
                return;
            }
        };
        if !page.items.is_empty() {
            if !remote {
                gate.wait().await;
            }
            if !embed_and_persist(registry, storage, page.items).await {
                return;
            }
        }
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    prune_replaced_models(registry, storage, &model.namespace).await;
}

/// The index is complete, so older models' vectors have nothing left to cover
/// for. Kept until now so that a plugin failing to load, or a change of mind,
/// doesn't throw away a reindex that may have cost money. Not if the embedder
/// changed during the pass: this one's vectors are then the ones to keep.
async fn prune_replaced_models(
    registry: &PluginRegistry,
    storage: &Arc<dyn Storage>,
    namespace: &str,
) {
    if registry
        .embedder_claim()
        .is_none_or(|claim| claim.namespace != namespace)
    {
        return;
    }
    match storage.prune_embeddings(namespace).await {
        Ok(0) => {}
        Ok(pruned) => tracing::info!(pruned, namespace, "embedding: pruned replaced vectors"),
        Err(e) => tracing::warn!(error = %e, "embedding: pruning replaced vectors failed"),
    }
}

/// Embed one batch of works and persist each vector under the plugin's
/// claimed namespace/model. Returns false when the caller should stop
/// its current pass (embed dispatch failed or a write failed); the
/// affected works stay unembedded for the next trigger.
async fn embed_and_persist(
    registry: &PluginRegistry,
    storage: &Arc<dyn Storage>,
    works: Vec<Work>,
) -> bool {
    if works.is_empty() {
        return true;
    }
    let mut inputs = Vec::with_capacity(works.len());
    for work in &works {
        let target = TagTarget::Work(work.id);
        let tags = match storage.tag_assignments_for_target(target).await {
            Ok(tags) => tags,
            Err(e) => {
                tracing::warn!(work = %work.id, error = %e, "embedding: tag lookup failed");
                return false;
            }
        };
        inputs.push(EmbedInput::Document(work_document(work, &tags)));
    }
    let Some(outcome) = (match embed_inputs(registry, inputs).await {
        Ok(outcome) => outcome,
        Err(e) => {
            tracing::warn!(error = %e, "embedding: embed dispatch failed");
            return false;
        }
    }) else {
        // Embedder was hot-reloaded out between the caller's claim
        // check and the dispatch.
        tracing::debug!("embedding skipped: embedder plugin went away");
        return false;
    };
    // embed_inputs validates one-vector-per-input against the claim, so
    // the zip is total.
    for (work, vector) in works.iter().zip(&outcome.vectors) {
        if let Err(e) = storage
            .upsert_embedding(
                EmbeddingTarget::Work(work.id),
                &outcome.namespace,
                &outcome.model_id,
                vector,
            )
            .await
        {
            tracing::warn!(work = %work.id, error = %e, "embedding: persist failed");
            return false;
        }
    }
    true
}

/// What an embedder sees for a work: `"{title}"`, plus `" ({year})"` when the
/// year is known, as the title, and `"{genres}"`, `"{keywords}"`, `"{people}"` and
/// `"{overview}"` when present as the text, cut at [`MAX_TEXT_BYTES`] on a char
/// boundary. Overviews rarely name a genre or theme outright, and a long one
/// comes last so truncation cuts its tail rather than the tags.
fn work_document(work: &Work, tags: &[TagAssignment]) -> EmbedDocument {
    let mut title = work.title.clone();
    if let Some(year) = work.year {
        use std::fmt::Write as _;
        let _ = write!(title, " ({year})");
    }
    let names = |namespace: &str| {
        tags.iter()
            .filter(|assignment| assignment.tag.namespace == namespace)
            .map(|assignment| assignment.tag.display.as_str())
            .collect::<Vec<_>>()
    };
    let mut people = tags
        .iter()
        .filter(|assignment| assignment.tag.namespace == "person")
        .collect::<Vec<_>>();
    people.sort_by_key(|assignment| assignment.metadata.0["order"].as_u64().unwrap_or(u64::MAX));
    people.truncate(MAX_PEOPLE);
    let people = people
        .iter()
        .map(|assignment| assignment.tag.display.as_str())
        .collect::<Vec<_>>();

    let mut parts = [names("genre"), names("keyword"), people]
        .iter()
        .filter(|list| !list.is_empty())
        .map(|list| list.join(", "))
        .collect::<Vec<_>>();
    if let Some(overview) = work.overview.as_ref().filter(|text| !text.is_empty()) {
        parts.push(overview.clone());
    }
    let mut text = parts.join(". ");
    if text.len() > MAX_TEXT_BYTES {
        let mut end = MAX_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    EmbedDocument { title, text }
}

#[cfg(test)]
mod tests {
    use noflex_storage::domain::Tag;

    use super::*;

    fn work(title: &str, year: Option<i32>, overview: Option<&str>) -> Work {
        let now = time::OffsetDateTime::now_utc();
        Work {
            id: noflex_core::WorkId::new(),
            kind: noflex_core::WorkKind::Movie,
            parent_id: None,
            library_id: noflex_core::LibraryId::new(),
            title: title.to_string(),
            sort_title: title.to_lowercase(),
            year,
            release_date: None,
            overview: overview.map(str::to_string),
            tagline: None,
            original_title: None,
            community_rating: None,
            external_ids: vec![],
            index_number: None,
            index_end: None,
            parent_index: None,
            metadata_status: noflex_core::MetadataStatus::Unconfirmed,
            locked_fields: vec![],
            metadata_provider: None,
            added_at: now,
            updated_at: now,
        }
    }

    fn tag(namespace: &str, display: &str, metadata: serde_json::Value) -> TagAssignment {
        TagAssignment {
            tag: Tag {
                id: noflex_core::TagId::new(),
                namespace: namespace.to_string(),
                parent_id: None,
                slug: display.to_lowercase(),
                display: display.to_string(),
                metadata: async_graphql::Json(serde_json::json!({})),
            },
            weight: 1.0,
            source: "test".to_string(),
            metadata: async_graphql::Json(metadata),
        }
    }

    #[test]
    fn document_shaping() {
        let plain = work_document(&work("Alien", None, None), &[]);
        assert_eq!((plain.title.as_str(), plain.text.as_str()), ("Alien", ""));

        let full = work_document(
            &work(
                "Alien",
                Some(1979),
                Some("In space no one can hear you scream."),
            ),
            &[],
        );
        assert_eq!(full.title, "Alien (1979)");
        assert_eq!(full.text, "In space no one can hear you scream.");

        // Empty overviews shape like absent ones.
        assert_eq!(work_document(&work("Alien", None, Some("")), &[]).text, "");
    }

    #[test]
    fn document_keeps_tags_ahead_of_a_long_overview() {
        let tags = [
            tag("studio", "Ignored", serde_json::json!({})),
            tag("keyword", "xenomorph", serde_json::json!({})),
            tag("genre", "Horror", serde_json::json!({})),
            tag("genre", "Science Fiction", serde_json::json!({})),
            tag(
                "person",
                "Sigourney Weaver",
                serde_json::json!({ "order": 0 }),
            ),
        ];
        let long = "x".repeat(MAX_TEXT_BYTES);
        let text = work_document(&work("Alien", None, Some(&long)), &tags).text;
        assert!(
            text.starts_with("Horror, Science Fiction. xenomorph. Sigourney Weaver. xxx"),
            "{text}"
        );
        assert!(text.len() <= MAX_TEXT_BYTES);
    }

    #[test]
    fn document_names_only_the_top_billed() {
        let tags = (0..MAX_PEOPLE + 2)
            .rev()
            .map(|order| {
                tag(
                    "person",
                    &format!("P{order}"),
                    serde_json::json!({ "order": order }),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            work_document(&work("Alien", None, None), &tags).text,
            "P0, P1, P2, P3, P4, P5"
        );
    }

    #[test]
    fn document_truncates_on_a_char_boundary() {
        let long = "é".repeat(MAX_TEXT_BYTES);
        let text = work_document(&work("t", None, Some(&long)), &[]).text;
        assert!(text.len() <= MAX_TEXT_BYTES);
        assert_eq!(text.chars().count(), MAX_TEXT_BYTES / 2);
    }

    /// Reports each scripted reading in turn; every other call, the one
    /// [`IdleGate::wait`] discards, reads as noise that must not count.
    struct Scripted(std::vec::IntoIter<f32>, bool);

    impl Load for Scripted {
        fn others_percent(&mut self) -> f32 {
            self.1 = !self.1;
            if self.1 {
                return 0.0;
            }
            self.0.next().unwrap_or(100.0)
        }
    }

    fn gate(readings: Vec<f32>) -> IdleGate<Scripted> {
        IdleGate::new(
            Scripted(readings.into_iter(), false),
            30.0,
            Duration::from_secs(60),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn gate_waits_out_a_busy_host() {
        let started = Instant::now();
        gate(vec![95.0, 80.0, 10.0]).wait().await;
        assert_eq!(started.elapsed(), LOAD_WINDOW * 3);
    }

    #[tokio::test(start_paused = true)]
    async fn gate_passes_a_batch_on_a_host_that_never_quiets() {
        let started = Instant::now();
        gate(vec![]).wait().await;
        assert!(started.elapsed() >= Duration::from_secs(60));
        assert!(started.elapsed() < Duration::from_secs(60) + LOAD_WINDOW * 2);
    }
}
