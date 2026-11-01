// SPDX-License-Identifier: AGPL-3.0-only

use std::path::PathBuf;

use tokio::sync::broadcast;

use crate::{HookKind, PluginId, PluginManifest};

/// Plugin lifecycle events broadcast to all subscribers (tracing bridge,
/// Notifier plugins, GraphQL subscription in v2).
///
/// `Discovered` and `Loaded` carry enough information that v2's tiered-trust
/// mode can drive its ack list off the same wire format without breaking
/// existing subscribers — additive evolution only.
#[derive(Debug, Clone)]
pub enum PluginLifecycleEvent {
    /// A `.wasm` file appeared in a watched directory. Fires *before* load.
    /// Boxed because `PluginManifest` is significantly larger than the other
    /// variants and Clippy flags the size disparity otherwise.
    Discovered(Box<DiscoveredEvent>),
    /// Plugin loaded successfully and is now dispatching.
    Loaded {
        plugin_id: PluginId,
        binary_sha256: String,
        hooks: Vec<HookKind>,
    },
    /// Plugin load failed (bad manifest, API mismatch, malformed binary).
    LoadFailed { path: PathBuf, reason: String },
    /// Plugin removed from the registry (file deleted, admin unloaded,
    /// or replaced by hot-reload).
    Unloaded {
        plugin_id: PluginId,
        reason: UnloadReason,
    },
    /// Plugin hit an unrecoverable trap during dispatch.
    Trapped {
        plugin_id: PluginId,
        hook: HookKind,
        trap: String,
    },
}

#[derive(Debug, Clone)]
pub struct DiscoveredEvent {
    pub path: PathBuf,
    pub manifest: PluginManifest,
    pub binary_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnloadReason {
    FileRemoved,
    ReplacedByHotReload,
    TooManyTraps,
    AdminRequested,
    Shutdown,
}

/// Thin wrapper around `tokio::sync::broadcast` so callers don't have to
/// know the channel's exact capacity or the receiver type explicitly.
#[derive(Clone)]
pub struct LifecycleBus {
    tx: broadcast::Sender<PluginLifecycleEvent>,
}

impl LifecycleBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PluginLifecycleEvent> {
        self.tx.subscribe()
    }

    /// Send an event. Bridges into the tracing subscriber under
    /// `noflex_plugin::lifecycle` so events show up in operational logs
    /// even when no async subscriber is wired up.
    pub fn emit(&self, ev: PluginLifecycleEvent) {
        match &ev {
            PluginLifecycleEvent::Discovered(d) => tracing::info!(
                target: "noflex_plugin::lifecycle",
                kind = "discovered",
                path = %d.path.display(),
                plugin_id = %d.manifest.plugin_id,
                version = %d.manifest.version,
                sha256 = %d.binary_sha256,
                hooks = ?d.manifest.hooks,
            ),
            PluginLifecycleEvent::Loaded {
                plugin_id,
                binary_sha256,
                hooks,
            } => tracing::info!(
                target: "noflex_plugin::lifecycle",
                kind = "loaded",
                plugin_id = %plugin_id,
                sha256 = %binary_sha256,
                hooks = ?hooks,
            ),
            PluginLifecycleEvent::LoadFailed { path, reason } => tracing::warn!(
                target: "noflex_plugin::lifecycle",
                kind = "load_failed",
                path = %path.display(),
                reason = %reason,
            ),
            PluginLifecycleEvent::Unloaded { plugin_id, reason } => tracing::info!(
                target: "noflex_plugin::lifecycle",
                kind = "unloaded",
                plugin_id = %plugin_id,
                reason = ?reason,
            ),
            PluginLifecycleEvent::Trapped {
                plugin_id,
                hook,
                trap,
            } => tracing::error!(
                target: "noflex_plugin::lifecycle",
                kind = "trapped",
                plugin_id = %plugin_id,
                hook = ?hook,
                trap = %trap,
            ),
        }
        // Best-effort: dropped if no subscribers; bus is fire-and-forget.
        let _ = self.tx.send(ev);
    }
}

impl Default for LifecycleBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_and_subscribe() {
        let bus = LifecycleBus::new(8);
        let mut rx = bus.subscribe();
        bus.emit(PluginLifecycleEvent::Unloaded {
            plugin_id: PluginId("test".into()),
            reason: UnloadReason::Shutdown,
        });
        let got = rx.recv().await.expect("event delivered");
        assert!(matches!(
            got,
            PluginLifecycleEvent::Unloaded {
                reason: UnloadReason::Shutdown,
                ..
            }
        ));
    }
}
