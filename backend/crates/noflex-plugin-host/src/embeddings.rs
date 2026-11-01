// SPDX-License-Identifier: AGPL-3.0-only
//
// Host-driven embedding worker: keeps the vector index filled by
// computing embeddings for works through the highest-priority embedder
// plugin and persisting them itself. The WIT task protocol has no
// channel for a guest to hand vectors back (outcomes are
// `done|more(cursor)`), so the sweep lives host-side: the worker calls
// the plugin's `embed` hook via [`embed_texts`] and writes through
// `Storage::upsert_embedding`.
//
// Two work feeds, serialized on one task so the plugin's single
// instance sees one `embed` call at a time from here:
//
// - `WorkAdded` events, debounced and drained in small batches (the
//   accelerator: fresh works get vectors without waiting for a sweep);
// - full backfill sweeps paging `works_missing_embedding` to
//   completion (the authority: anything the event path missed or
//   failed on stays "missing" and is retried here).

use std::{collections::HashSet, sync::Arc, time::Duration};

use noflex_core::{
    EmbeddingTarget, WorkId,
    events::{DomainEvent, DomainEventBus},
};
use noflex_storage::{Storage, domain::Work};
use tokio::{
    sync::{broadcast::error::RecvError, mpsc},
    task::JoinHandle,
    time::{Instant, interval_at},
};

use crate::{PluginRegistry, registry::embed_texts};

/// Quiet period after the last `WorkAdded` before a batch embeds —
/// scans add works in bursts, and one 32-text call beats thirty-two
/// 1-text calls on a model-bound plugin.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Most works embedded per event-path batch.
const BATCH_SIZE: usize = 32;

/// Sweep page size; also under the 64-text `embed` dispatch cap.
const SWEEP_PAGE: u32 = 64;

/// Idle tick that re-runs the sweep so a silently-dropped event or a
/// hot-reloaded embedder eventually converges.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Cap on buffered `WorkAdded` ids. Overflow is dropped, not queued:
/// the event path is only an accelerator, and the sweep finds every
/// missing work regardless.
const MAX_PENDING: usize = 512;

/// Rough cap on the text sent to an embedder, in bytes (≈chars for the
/// Latin-heavy titles/overviews this shapes). Long overviews add noise,
/// not signal, past this point.
const MAX_TEXT_BYTES: usize = 1000;

/// Internal worker command: embed one work now-ish, or run a full
/// backfill sweep.
enum Cmd {
    Added(WorkId),
    Sweep,
}

/// Spawn the embedding worker and its bus forwarder. The returned
/// handle is the worker's; both tasks exit only when aborted — a
/// closing event bus just ends event-driven feeding, the hourly sweep
/// keeps running.
pub fn spawn_embedding_worker(
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    events: &DomainEventBus,
) -> JoinHandle<()> {
    let (tx, rx) = mpsc::channel::<Cmd>(256);
    tokio::spawn(forward_events(events.clone(), tx.clone()));
    tokio::spawn(worker(registry, storage, rx, tx))
}

/// Forwarder: subscribes to the domain bus and turns the two relevant
/// events into worker commands. Lagging drops events — tolerable, since
/// dropped `WorkAdded`s are picked up by the next sweep and a dropped
/// `ScanCompleted` only delays one.
async fn forward_events(bus: DomainEventBus, tx: mpsc::Sender<Cmd>) {
    let mut rx = bus.subscribe();
    loop {
        let cmd = match rx.recv().await {
            Ok(DomainEvent::WorkAdded { work, .. }) => Cmd::Added(work),
            Ok(DomainEvent::ScanCompleted { .. }) => Cmd::Sweep,
            Ok(_) => continue,
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "embedding worker lagged; events dropped");
                continue;
            }
            Err(RecvError::Closed) => break,
        };
        // A full channel means the worker is behind; the sweep is the
        // backstop, so dropping beats blocking the bus for everyone.
        if let Err(e) = tx.try_send(cmd) {
            tracing::warn!(error = %e, "embedding worker command dropped");
        }
    }
}

async fn worker(
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    mut rx: mpsc::Receiver<Cmd>,
    // Kept so `rx` never reports closed after the forwarder exits (bus
    // dropped); the worker lives on its hourly tick.
    _keepalive: mpsc::Sender<Cmd>,
) {
    let mut pending: HashSet<WorkId> = HashSet::new();
    let mut debounce: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
    let mut tick = interval_at(Instant::now() + SWEEP_INTERVAL, SWEEP_INTERVAL);

    // Startup sweep: backfill everything missing from before we booted.
    run_sweep(&registry, &storage).await;

    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(Cmd::Added(id)) => {
                    if pending.len() < MAX_PENDING {
                        pending.insert(id);
                    }
                    if debounce.is_none() {
                        debounce = Some(Box::pin(tokio::time::sleep(DEBOUNCE)));
                    }
                }
                Some(Cmd::Sweep) => run_sweep(&registry, &storage).await,
                None => break,
            },
            () = async {
                if let Some(d) = debounce.as_mut() {
                    d.await;
                }
            }, if debounce.is_some() => {
                debounce = None;
                let batch: Vec<WorkId> = pending.iter().take(BATCH_SIZE).copied().collect();
                for id in &batch {
                    pending.remove(id);
                }
                embed_ids(&registry, &storage, batch).await;
                if !pending.is_empty() {
                    debounce = Some(Box::pin(tokio::time::sleep(DEBOUNCE)));
                }
            }
            _ = tick.tick() => run_sweep(&registry, &storage).await,
        }
    }
}

/// Event-path batch: resolve ids to works (skipping any deleted since
/// the event), embed, persist. A failure drops the batch — the works
/// stay missing and the next sweep retries them.
async fn embed_ids(registry: &PluginRegistry, storage: &Arc<dyn Storage>, ids: Vec<WorkId>) {
    if registry.embedder_claim().is_none() {
        return;
    }
    let mut works = Vec::with_capacity(ids.len());
    for id in ids {
        match storage.get_work(id).await {
            Ok(Some(work)) => works.push(work),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(work = %id, error = %e, "embedding: work fetch failed");
                return;
            }
        }
    }
    embed_and_persist(registry, storage, works).await;
}

/// One backfill pass over `works_missing_embedding`. Aborts the pass on
/// the first embed or storage failure — failed works remain "missing",
/// and the next trigger (event or hourly tick) retries, so a down
/// embedder never tight-loops.
async fn run_sweep(registry: &PluginRegistry, storage: &Arc<dyn Storage>) {
    let Some(claim) = registry.embedder_claim() else {
        tracing::debug!("embedding sweep skipped: no embedder plugin loaded");
        return;
    };
    let mut cursor = None;
    loop {
        let page = match storage
            .works_missing_embedding(&claim.namespace, SWEEP_PAGE, cursor)
            .await
        {
            Ok(page) => page,
            Err(e) => {
                tracing::warn!(error = %e, "embedding sweep: listing missing works failed");
                return;
            }
        };
        if page.items.is_empty() {
            return;
        }
        let next = page.next_cursor;
        if !embed_and_persist(registry, storage, page.items).await {
            return;
        }
        match next {
            Some(c) => cursor = Some(c),
            None => return,
        }
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
    let texts: Vec<String> = works.iter().map(work_text).collect();
    let Some(outcome) = (match embed_texts(registry, texts).await {
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
    // embed_texts validates one-vector-per-text against the claim, so
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

/// The text an embedder sees for a work: `"{title}"`, plus
/// `" ({year})"` when the year is known, plus `". {overview}"` when one
/// is set — truncated to [`MAX_TEXT_BYTES`] on a char boundary.
fn work_text(work: &Work) -> String {
    let mut text = work.title.clone();
    if let Some(year) = work.year {
        use std::fmt::Write as _;
        let _ = write!(text, " ({year})");
    }
    if let Some(overview) = &work.overview {
        if !overview.is_empty() {
            text.push_str(". ");
            text.push_str(overview);
        }
    }
    if text.len() > MAX_TEXT_BYTES {
        let mut end = MAX_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
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
            added_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn text_shaping() {
        assert_eq!(work_text(&work("Alien", None, None)), "Alien");
        assert_eq!(work_text(&work("Alien", Some(1979), None)), "Alien (1979)");
        assert_eq!(
            work_text(&work(
                "Alien",
                Some(1979),
                Some("In space no one can hear you scream.")
            )),
            "Alien (1979). In space no one can hear you scream."
        );
        // Empty overviews shape like absent ones.
        assert_eq!(work_text(&work("Alien", None, Some(""))), "Alien");
    }

    #[test]
    fn text_truncates_on_a_char_boundary() {
        // "t. " (3 bytes) + multi-byte chars: the raw cap lands
        // mid-char, and truncation backs off to the boundary.
        let long = "é".repeat(MAX_TEXT_BYTES);
        let text = work_text(&work("t", None, Some(&long)));
        assert!(text.len() <= MAX_TEXT_BYTES);
        assert_eq!(text.chars().count(), 3 + (MAX_TEXT_BYTES - 3) / 2);
    }
}
