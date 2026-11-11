// SPDX-License-Identifier: AGPL-3.0-only
//
// Scrobble dispatcher: subscribes to the core `DomainEventBus` and fans
// playback-derived scrobble occurrences out to every loaded plugin
// exporting the `scrobbler` hook. Mirrors the notifier dispatcher's
// shape — the bus is lossy (a lagging dispatcher misses events rather
// than slowing producers) and the fan-out is failure-isolated — but the
// delivery rules differ in two ways:
//
// - Every event is enriched from storage once (kind, year, external ids,
//   runtime), so scrobbler plugins match on provider ids and never need
//   library lookups of their own. An event whose work row is already
//   gone (delete race) is skipped.
// - Delivery is per-user: a plugin is skipped for an event when the
//   acting user holds no secrets for it (never linked an account). All
//   four event kinds reach every linked scrobbler unconditionally —
//   there is no manifest `events` filter on this path.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use noflex_core::{
    UserId, WorkId,
    events::{DomainEvent, DomainEventBus},
};
use noflex_storage::{Storage, domain::Work};
use tokio::{sync::broadcast::error::RecvError, task::JoinHandle};

use crate::{
    HookKind, LoadedPlugin, PluginId, PluginRegistry,
    bindings::scrobbler::noflex::plugin::types as wit, registry::to_wit_kind_id,
};

/// Spawn the scrobble dispatcher loop. Returns its handle; the loop
/// exits when the bus closes (every `DomainEventBus` clone dropped).
pub fn spawn_scrobble_dispatcher(
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    events: &DomainEventBus,
) -> JoinHandle<()> {
    let plugins = registry.plugins_handle();
    let mut rx = events.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Some(occ) = Occurrence::from_event(&event) {
                        dispatch(&plugins, &storage, occ).await;
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "scrobble dispatcher lagged; events dropped");
                }
                Err(RecvError::Closed) => break,
            }
        }
    })
}

/// One scrobble occurrence before enrichment: who, which work, the state
/// transition, and the stop position when the event carries one.
struct Occurrence {
    user: UserId,
    work: WorkId,
    title: String,
    action: wit::ScrobbleAction,
    position_ms: Option<u64>,
}

impl Occurrence {
    /// Map a domain event onto a scrobble occurrence, or `None` when the
    /// event is not playback-derived (scrobblers see exactly four kinds:
    /// session start/stop, progress-at-stop, and manual mark-played —
    /// the last as `stopped` with no position, which the plugin treats
    /// as fully watched).
    fn from_event(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::PlaybackStarted {
                user, work, title, ..
            } => Some(Self {
                user: *user,
                work: *work,
                title: title.clone(),
                action: wit::ScrobbleAction::Started,
                position_ms: None,
            }),
            DomainEvent::PlaybackStopped {
                user, work, title, ..
            } => Some(Self {
                user: *user,
                work: *work,
                title: title.clone(),
                action: wit::ScrobbleAction::Stopped,
                position_ms: None,
            }),
            DomainEvent::ProgressAtStop {
                user,
                work,
                title,
                position_ms,
            } => Some(Self {
                user: *user,
                work: *work,
                title: title.clone(),
                action: wit::ScrobbleAction::Stopped,
                position_ms: Some(*position_ms),
            }),
            DomainEvent::WorkPlayed { user, work, title } => Some(Self {
                user: *user,
                work: *work,
                title: title.clone(),
                action: wit::ScrobbleAction::Stopped,
                position_ms: None,
            }),
            _ => None,
        }
    }
}

async fn dispatch(
    plugins: &RwLock<HashMap<PluginId, Arc<LoadedPlugin>>>,
    storage: &Arc<dyn Storage>,
    occ: Occurrence,
) {
    // Enrich once per event, not per plugin: every target sees the same
    // payload, and a plugin erroring can't burn storage round-trips for
    // the others.
    let work = match storage.get_work(occ.work).await {
        Ok(Some(work)) => work,
        Ok(None) => {
            tracing::debug!(
                work = %occ.work,
                "scrobble skipped: work row gone (delete race)"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(work = %occ.work, error = %e, "scrobble: work fetch failed");
            return;
        }
    };
    let event = enrich(storage, &work, &occ).await;
    let targets: Vec<(String, Arc<LoadedPlugin>)> = {
        let guard = plugins.read().expect("registry lock poisoned");
        guard
            .values()
            .filter(|p| p.exports(HookKind::Scrobbler))
            .map(|p| (p.id.0.clone(), Arc::clone(p)))
            .collect()
    };
    if targets.is_empty() {
        return;
    }
    let calls = targets.into_iter().map(|(id, plugin)| {
        let storage = Arc::clone(storage);
        let event = event.clone();
        let user = occ.user;
        async move {
            // The WIT contract: users who never linked are skipped. Any
            // stored secret counts as linked — the plugin decides which
            // keys it actually reads back.
            match storage.list_user_plugin_secrets(user, &id).await {
                Ok(secrets) if secrets.is_empty() => {
                    tracing::debug!(plugin = %id, user = %user, "scrobble skipped: user not linked");
                    (id, None)
                }
                Ok(_) => (id, Some(plugin.call_scrobble(event).await)),
                Err(e) => {
                    tracing::warn!(plugin = %id, error = %e,
                        "scrobble: linked-account check failed; skipping plugin");
                    (id, None)
                }
            }
        }
    });
    for (id, result) in futures::future::join_all(calls).await {
        match result {
            Some(Ok(Ok(()))) | None => {}
            Some(Ok(Err(e))) => {
                tracing::warn!(plugin = %id, error = ?e, "scrobbler returned an error");
            }
            Some(Err(e)) => {
                tracing::warn!(plugin = %id, error = %e, "scrobbler dispatch failed");
            }
        }
    }
}

/// Build the wire event: storage-sourced kind/year/external-ids plus the
/// best-effort runtime (the first edition of the work with one set;
/// negative values — never valid runtimes — are dropped).
async fn enrich(storage: &Arc<dyn Storage>, work: &Work, occ: &Occurrence) -> wit::ScrobbleEvent {
    let runtime_ms = match storage.editions_for_work(work.id).await {
        Ok(editions) => editions
            .iter()
            .find_map(|e| e.runtime_ms)
            .and_then(|ms| u64::try_from(ms).ok()),
        Err(e) => {
            tracing::warn!(work = %work.id, error = %e, "scrobble: editions fetch failed");
            None
        }
    };
    wit::ScrobbleEvent {
        user: occ.user.to_string(),
        work: occ.work.to_string(),
        kind: work_kind_to_wit(work.kind),
        title: occ.title.clone(),
        year: work.year,
        external_ids: work
            .external_ids
            .iter()
            .map(|id| wit::ExternalId {
                kind: retag_kind_id(to_wit_kind_id(id.kind)),
                value: id.value.clone(),
            })
            .collect(),
        series_external_ids: Vec::new(),
        season_number: None,
        episode_number: None,
        action: occ.action,
        position_ms: occ.position_ms,
        runtime_ms,
    }
}

fn work_kind_to_wit(k: noflex_core::model::WorkKind) -> wit::WorkKind {
    use noflex_core::model::WorkKind;
    match k {
        WorkKind::Movie => wit::WorkKind::Movie,
        WorkKind::Series => wit::WorkKind::Series,
        WorkKind::Season => wit::WorkKind::Season,
        WorkKind::Episode => wit::WorkKind::Episode,
        WorkKind::Album => wit::WorkKind::Album,
        WorkKind::Track => wit::WorkKind::Track,
        WorkKind::Artist => wit::WorkKind::Artist,
        WorkKind::AudioBook => wit::WorkKind::AudioBook,
        WorkKind::Photo => wit::WorkKind::Photo,
    }
}

/// Re-tag the canonical (metadata-world) external-id kind onto the
/// scrobbler world's copy. Every `bindgen!` emits its own `types`
/// module; the enums are identical over the wire.
fn retag_kind_id(k: crate::plugin_types::ExternalIdKind) -> wit::ExternalIdKind {
    use crate::plugin_types::ExternalIdKind as M;
    match k {
        M::Tmdb => wit::ExternalIdKind::Tmdb,
        M::Tvdb => wit::ExternalIdKind::Tvdb,
        M::Imdb => wit::ExternalIdKind::Imdb,
        M::MusicBrainzReleaseGroup => wit::ExternalIdKind::MusicBrainzReleaseGroup,
        M::MusicBrainzArtist => wit::ExternalIdKind::MusicBrainzArtist,
        M::MusicBrainzRecording => wit::ExternalIdKind::MusicBrainzRecording,
        M::AniDb => wit::ExternalIdKind::AniDb,
        M::Custom => wit::ExternalIdKind::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_mapping_covers_exactly_the_playback_kinds() {
        let user = UserId::new();
        let work = WorkId::new();
        let file = noflex_core::MediaFileId::new();
        let title = "Alien".to_string();

        let occ = Occurrence::from_event(&DomainEvent::PlaybackStarted {
            user,
            work,
            title: title.clone(),
            media_file: file,
        })
        .expect("playback start maps");
        assert!(matches!(occ.action, wit::ScrobbleAction::Started));
        assert_eq!(occ.position_ms, None);

        let occ = Occurrence::from_event(&DomainEvent::PlaybackStopped {
            user,
            work,
            title: title.clone(),
            media_file: file,
        })
        .expect("playback stop maps");
        assert!(matches!(occ.action, wit::ScrobbleAction::Stopped));
        assert_eq!(occ.position_ms, None);

        let occ = Occurrence::from_event(&DomainEvent::ProgressAtStop {
            user,
            work,
            title: title.clone(),
            position_ms: 42_000,
        })
        .expect("progress-at-stop maps");
        assert!(matches!(occ.action, wit::ScrobbleAction::Stopped));
        assert_eq!(occ.position_ms, Some(42_000));

        let occ = Occurrence::from_event(&DomainEvent::WorkPlayed {
            user,
            work,
            title: title.clone(),
        })
        .expect("mark-played maps");
        assert!(matches!(occ.action, wit::ScrobbleAction::Stopped));
        assert_eq!(occ.position_ms, None);

        assert!(
            Occurrence::from_event(&DomainEvent::ScanStarted {
                library: noflex_core::LibraryId::new(),
                library_name: "Movies".into(),
            })
            .is_none()
        );
        assert!(
            Occurrence::from_event(&DomainEvent::WorkAdded {
                work,
                library: noflex_core::LibraryId::new(),
                title,
            })
            .is_none()
        );
    }
}
