// SPDX-License-Identifier: AGPL-3.0-only
//
// Domain-event dispatcher: subscribes to the core `DomainEventBus` and
// fans each event a notifier hears out to every loaded plugin exporting
// the `notifier` hook, and each finished scan to the `post-scan` hook.
// The bus is lossy by design (a lagging dispatcher misses events
// rather than slowing producers), and the fan-out is failure-isolated:
// one notifier's plugin-error or trap is logged and never stops
// delivery to the others.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use noflex_core::{
    LibraryId, WorkId,
    events::{DomainEvent, DomainEventBus},
};
use tokio::{sync::broadcast::error::RecvError, task::JoinHandle};

use crate::{
    HookKind, LoadedPlugin, PluginId, PluginRegistry,
    bindings::notifier::noflex::plugin::types as wit,
};

/// Spawn the dispatcher loop. Returns its handle; the loop exits when
/// the bus closes (every `DomainEventBus` clone dropped).
pub fn spawn(bus: &DomainEventBus, registry: Arc<PluginRegistry>) -> JoinHandle<()> {
    spawn_on(bus, registry.plugins_handle())
}

/// Shared body for [`spawn`] and [`PluginRegistry::with_event_bus`]: the
/// loop only needs the live plugin map, so the builder can hand it over
/// before the registry itself is wrapped in an `Arc`.
pub(crate) fn spawn_on(
    bus: &DomainEventBus,
    plugins: Arc<RwLock<HashMap<PluginId, Arc<LoadedPlugin>>>>,
) -> JoinHandle<()> {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => dispatch(&plugins, event).await,
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "domain event dispatcher lagged; events dropped");
                }
                Err(RecvError::Closed) => break,
            }
        }
    })
}

async fn dispatch(plugins: &RwLock<HashMap<PluginId, Arc<LoadedPlugin>>>, event: DomainEvent) {
    let kind = event.kind_str();
    if let DomainEvent::ScanCompleted {
        library,
        added_works,
        updated_works,
        removed_works,
        ..
    } = event
    {
        post_scan_fanout(plugins, library, added_works, updated_works, removed_works).await;
        return;
    }
    let Some(event) = to_notify_event(&event) else {
        return;
    };
    let targets: Vec<(String, Arc<LoadedPlugin>)> = {
        let guard = plugins.read().expect("registry lock poisoned");
        guard
            .values()
            .filter(|p| p.exports(HookKind::Notifier) && p.manifest.subscribed_to(kind))
            .map(|p| (p.id.0.clone(), Arc::clone(p)))
            .collect()
    };
    let calls = targets.into_iter().map(|(id, plugin)| {
        let event = event.clone();
        async move { (id, plugin.call_notify(event).await) }
    });
    for (id, result) in futures::future::join_all(calls).await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(plugin = %id, event = kind, error = ?e,
                    "notifier returned an error");
            }
            Err(e) => {
                tracing::warn!(plugin = %id, event = kind, error = %e,
                    "notifier dispatch failed");
            }
        }
    }
}

/// Fan a finished scan's work-id lists out to every loaded plugin
/// exporting the `post-scan` hook. Failure-isolated like the notifier
/// fan-out: one plugin's plugin-error or trap is logged and never stops
/// delivery to the others. Shared by [`dispatch`] and
/// [`PluginRegistry::post_scan`].
pub(crate) async fn post_scan_fanout(
    plugins: &RwLock<HashMap<PluginId, Arc<LoadedPlugin>>>,
    library: LibraryId,
    added: Vec<WorkId>,
    updated: Vec<WorkId>,
    removed: Vec<WorkId>,
) {
    let targets: Vec<(String, Arc<LoadedPlugin>)> = {
        let guard = plugins.read().expect("registry lock poisoned");
        guard
            .values()
            .filter(|p| p.exports(HookKind::PostScan))
            .map(|p| (p.id.0.clone(), Arc::clone(p)))
            .collect()
    };
    if targets.is_empty() {
        return;
    }
    let library = library.to_string();
    let added: Vec<String> = added.iter().map(ToString::to_string).collect();
    let updated: Vec<String> = updated.iter().map(ToString::to_string).collect();
    let removed: Vec<String> = removed.iter().map(ToString::to_string).collect();
    let calls = targets.into_iter().map(|(id, plugin)| {
        let library = library.clone();
        let added = added.clone();
        let updated = updated.clone();
        let removed = removed.clone();
        async move {
            (
                id,
                plugin
                    .call_post_scan(library, added, updated, removed)
                    .await,
            )
        }
    });
    for (id, result) in futures::future::join_all(calls).await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(plugin = %id, error = ?e,
                    "post-scan plugin returned an error");
            }
            Err(e) => {
                tracing::warn!(plugin = %id, error = %e,
                    "post-scan dispatch failed");
            }
        }
    }
}

/// Map a domain event onto the notifier hook's wire shape, or `None` for
/// one notifiers don't hear: pauses and resumes are for scrobblers, and a
/// scan's changes go to `post-scan`. IDs cross as hyphenated UUID strings.
/// `WorkAdded`'s `library` has no counterpart in the WIT record and is
/// dropped by design.
fn to_notify_event(event: &DomainEvent) -> Option<wit::NotifyEvent> {
    Some(match event {
        DomainEvent::WorkAdded { work, title, .. } => {
            wit::NotifyEvent::WorkAdded(wit::WorkAddedEvent {
                work: work.to_string(),
                title: title.clone(),
            })
        }
        DomainEvent::WorkPlayed { user, work, title } => {
            wit::NotifyEvent::WorkPlayed(wit::WorkPlayedEvent {
                user: user.to_string(),
                work: work.to_string(),
                title: title.clone(),
            })
        }
        DomainEvent::ProgressAtStop {
            user,
            work,
            title,
            position_ms,
        } => wit::NotifyEvent::ProgressAtStop(wit::ProgressEvent {
            user: user.to_string(),
            work: work.to_string(),
            title: title.clone(),
            position_ms: *position_ms,
        }),
        DomainEvent::Favorite {
            user,
            work,
            title,
            favorite,
        } => wit::NotifyEvent::Favorite(wit::FavoriteEvent {
            user: user.to_string(),
            work: work.to_string(),
            title: title.clone(),
            favorite: *favorite,
        }),
        DomainEvent::PlaybackStarted {
            user,
            work,
            title,
            media_file,
        } => wit::NotifyEvent::PlaybackStarted(playback_event(user, work, title, media_file)),
        DomainEvent::PlaybackStopped {
            user,
            work,
            title,
            media_file,
        } => wit::NotifyEvent::PlaybackStopped(playback_event(user, work, title, media_file)),
        DomainEvent::PlaybackPaused { .. }
        | DomainEvent::PlaybackResumed { .. }
        | DomainEvent::ScanStarted { .. }
        | DomainEvent::ScanCompleted { .. } => return None,
    })
}

fn playback_event(
    user: &noflex_core::UserId,
    work: &noflex_core::WorkId,
    title: &str,
    media_file: &noflex_core::MediaFileId,
) -> wit::PlaybackEvent {
    wit::PlaybackEvent {
        user: user.to_string(),
        work: work.to_string(),
        title: title.to_string(),
        media_file: media_file.to_string(),
    }
}
