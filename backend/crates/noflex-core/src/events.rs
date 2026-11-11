// SPDX-License-Identifier: AGPL-3.0-only
//
// Domain events: things that happened in the library that plugins may care
// about (a user finished an episode, a scan added works, ...). Producers
// (GraphQL mutations, the playback session service, the scanner) publish
// onto one in-process broadcast bus; the plugin host's dispatcher is the
// consumer, fanning events out to notifier hooks. The bus is lossy by
// design — a slow or absent subscriber misses events rather than applying
// backpressure to user-facing mutations.

use tokio::sync::broadcast;

use crate::ids::{LibraryId, MediaFileId, UserId, WorkId};

/// One domain occurrence. Carries resolved display strings (`title`,
/// `library_name`) so consumers never need a storage round-trip to render
/// or forward the event.
#[derive(Debug, Clone)]
pub enum DomainEvent {
    /// A scan created a work that did not exist before.
    WorkAdded {
        work: WorkId,
        library: LibraryId,
        title: String,
    },
    /// A user marked a work played (`markPlayed`).
    WorkPlayed {
        user: UserId,
        work: WorkId,
        title: String,
    },
    /// Playback stopped at a position (`reportProgress` with `Stopped`).
    /// Not emitted for Playing/Paused heartbeats — those are far too hot
    /// for a broadcast every consumer sees.
    ProgressAtStop {
        user: UserId,
        work: WorkId,
        title: String,
        position_ms: u64,
    },
    /// A user favorited or unfavorited a work.
    Favorite {
        user: UserId,
        work: WorkId,
        title: String,
        favorite: bool,
    },
    /// A playback session started (direct play or transcode).
    PlaybackStarted {
        user: UserId,
        work: WorkId,
        title: String,
        media_file: MediaFileId,
    },
    /// A playback session was explicitly stopped.
    PlaybackStopped {
        user: UserId,
        work: WorkId,
        title: String,
        media_file: MediaFileId,
    },
    /// A library scan began.
    ScanStarted {
        library: LibraryId,
        library_name: String,
    },
    /// A library scan finished, with per-outcome file counts and the
    /// works behind them. The `u64` counts feed the activity log; the id
    /// lists feed the post-scan hook. `removed_works` is
    /// always empty today — scan pruning operates at media-file level
    /// and work-level removal isn't tracked.
    ScanCompleted {
        library: LibraryId,
        library_name: String,
        added: u64,
        updated: u64,
        removed: u64,
        added_works: Vec<WorkId>,
        updated_works: Vec<WorkId>,
        removed_works: Vec<WorkId>,
    },
}

impl DomainEvent {
    /// Stable machine name for logging and audit lines.
    pub fn kind_str(&self) -> &'static str {
        match self {
            DomainEvent::WorkAdded { .. } => "work-added",
            DomainEvent::WorkPlayed { .. } => "work-played",
            DomainEvent::ProgressAtStop { .. } => "progress-at-stop",
            DomainEvent::Favorite { .. } => "favorite",
            DomainEvent::PlaybackStarted { .. } => "playback-started",
            DomainEvent::PlaybackStopped { .. } => "playback-stopped",
            DomainEvent::ScanStarted { .. } => "scan-started",
            DomainEvent::ScanCompleted { .. } => "scan-completed",
        }
    }
}

/// In-process fan-out for [`DomainEvent`]s. Cheap to clone — every clone
/// publishes to the same channel. `emit` is infallible: with no
/// subscribers the event is simply dropped, and a lagging subscriber
/// misses events (`broadcast::error::RecvError::Lagged`) rather than
/// blocking producers.
#[derive(Clone)]
pub struct DomainEventBus {
    tx: broadcast::Sender<DomainEvent>,
}

impl DomainEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: DomainEvent) {
        tracing::debug!(target: "noflex::events", kind = event.kind_str(), "domain event");
        // No receivers is normal (plugin host not wired, tests) — not an error.
        let _ = self.tx.send(event);
    }
}

impl Default for DomainEventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_reaches_subscribers() {
        let bus = DomainEventBus::new(8);
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        let work = WorkId::new();
        bus.emit(DomainEvent::WorkAdded {
            work,
            library: LibraryId::new(),
            title: "Alien".into(),
        });
        for rx in [&mut a, &mut b] {
            match rx.recv().await.expect("event delivered") {
                DomainEvent::WorkAdded { work: w, title, .. } => {
                    assert_eq!(w, work);
                    assert_eq!(title, "Alien");
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    #[test]
    fn emit_without_subscribers_is_not_an_error() {
        let bus = DomainEventBus::new(8);
        bus.emit(DomainEvent::ScanStarted {
            library: LibraryId::new(),
            library_name: "Movies".into(),
        });
    }
}
