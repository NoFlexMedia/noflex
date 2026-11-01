// SPDX-License-Identifier: AGPL-3.0-only
//
// Filesystem watcher built on `notify`. Filesystem events are coalesced via a
// fixed-duration debounce, then trigger a full `scan_library` on the library
// they belong to. Per-file diffing is deliberately omitted because scans are
// tiny and the simplest correct rescan is to walk the whole tree.

use std::{path::PathBuf, sync::Arc, time::Duration};

use noflex_core::{DomainEventBus, LibraryId};
use noflex_storage::Storage;
use notify::{Event, RecursiveMode, Watcher as NotifyWatcher, recommended_watcher};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::Instant,
};

use crate::scan;

const DEBOUNCE: Duration = Duration::from_secs(2);

/// Handle to a running watcher. Drop or `shutdown().await` to stop.
pub struct Watcher {
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<anyhow::Result<()>>>,
}

impl Watcher {
    /// Stop the watcher and wait for the background task to finish.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.await??;
        }
        Ok(())
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Start watching every root in the library. Returns a [`Watcher`] handle.
///
/// Errors here come from `notify` initialisation (missing root, EPERM, …);
/// per-event errors are logged and swallowed.
pub fn start(
    storage: Arc<dyn Storage>,
    library: LibraryId,
    roots: Vec<PathBuf>,
    events: Option<DomainEventBus>,
) -> anyhow::Result<Watcher> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<()>();
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    let mut notify_watcher = recommended_watcher(move |res: notify::Result<Event>| match res {
        Ok(_event) => {
            // We coalesce: any event is just a poke to trigger a rescan.
            let _ = event_tx.send(());
        }
        Err(err) => {
            tracing::warn!(%err, "notify error");
        }
    })?;
    for root in &roots {
        notify_watcher.watch(root, RecursiveMode::Recursive)?;
    }

    let handle = tokio::spawn(async move {
        // Keep the watcher alive in this task; drop on exit.
        let _keep = notify_watcher;
        let mut pending_until: Option<Instant> = None;
        loop {
            let next_tick = pending_until.map(|t| tokio::time::sleep_until(t));
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => {
                    tracing::debug!("watcher: shutdown signal");
                    break;
                }
                ev = event_rx.recv() => {
                    if ev.is_none() {
                        // All senders dropped; no more events possible.
                        break;
                    }
                    pending_until = Some(Instant::now() + DEBOUNCE);
                }
                _ = async { if let Some(s) = next_tick { s.await } }, if pending_until.is_some() => {
                    pending_until = None;
                    if let Err(err) = scan::run(storage.as_ref(), library, events.as_ref()).await {
                        tracing::warn!(%err, "watcher rescan failed");
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    });

    Ok(Watcher {
        shutdown_tx: Some(shutdown_tx),
        handle: Some(handle),
    })
}
