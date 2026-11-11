// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use notify::{Event, RecursiveMode, Watcher as NotifyWatcher, recommended_watcher};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::Instant,
};

use crate::{
    PluginId, PluginManifest, PluginRegistry,
    discovery::SearchPath,
    events::{LifecycleBus, PluginLifecycleEvent, UnloadReason},
};

const DEBOUNCE: Duration = Duration::from_millis(500);

pub struct PluginWatcher {
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<anyhow::Result<()>>>,
}

impl PluginWatcher {
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

impl Drop for PluginWatcher {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Load every plugin in the search path, then watch its directories and
/// reconcile on change. Each load's config comes from the registry (see
/// [`PluginRegistry::with_plugin_config`]), so a settings change survives
/// later syncs.
pub async fn start(
    registry: Arc<PluginRegistry>,
    bus: LifecycleBus,
    search_path: SearchPath,
) -> anyhow::Result<PluginWatcher> {
    sync_once(&registry, &bus, &search_path).await;

    let (event_tx, mut event_rx) = mpsc::channel::<()>(1);
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    let mut notify_watcher = recommended_watcher(move |res: notify::Result<Event>| match res {
        Ok(_) => {
            let _ = event_tx.try_send(());
        }
        Err(err) => {
            tracing::warn!(target: "noflex_plugin::lifecycle", %err, "notify error");
        }
    })?;
    for dir in search_path.directories() {
        if let Err(err) = notify_watcher.watch(dir, RecursiveMode::NonRecursive) {
            tracing::debug!(
                target: "noflex_plugin::lifecycle",
                dir = %dir.display(),
                %err,
                "skip watching missing/inaccessible plugin dir",
            );
        }
    }

    let handle = tokio::spawn(async move {
        let _keep = notify_watcher;
        let mut pending_until: Option<Instant> = None;
        loop {
            let next_tick = pending_until.map(tokio::time::sleep_until);
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => break,
                ev = event_rx.recv() => {
                    if ev.is_none() {
                        break;
                    }
                    pending_until = Some(Instant::now() + DEBOUNCE);
                }
                _ = async { if let Some(s) = next_tick { s.await } }, if pending_until.is_some() => {
                    pending_until = None;
                    sync_once(&registry, &bus, &search_path).await;
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    });

    Ok(PluginWatcher {
        shutdown_tx: Some(shutdown_tx),
        handle: Some(handle),
    })
}

struct Candidate {
    path: PathBuf,
    manifest: PluginManifest,
    sha256: String,
}

/// Reconcile one discovered snapshot. Later search-path tiers win. Unchanged
/// path, binary, and config triples keep their existing component and store.
/// A plugin the registry has disabled counts as absent, so it never loads.
pub async fn sync_once(registry: &PluginRegistry, bus: &LifecycleBus, search_path: &SearchPath) {
    let _loading = registry.load_lock().await;
    let mut candidates = HashMap::<PluginId, Candidate>::new();
    let mut failed_paths = HashSet::new();

    for (_tier, path) in search_path.discover_wasm_files() {
        let parsed = std::fs::read(&path)
            .map_err(crate::HostError::from)
            .and_then(|bytes| {
                let manifest = crate::plugin::extract_manifest(&bytes, &path)?;
                manifest.check_contract(&path)?;
                Ok((manifest, crate::plugin::binary_sha256(&bytes)))
            });
        match parsed {
            Ok((manifest, _)) if registry.is_disabled(&manifest.plugin_id) => {
                tracing::debug!(
                    target: "noflex_plugin::lifecycle",
                    plugin = %manifest.plugin_id,
                    path = %path.display(),
                    "plugin disabled in config; not loading"
                );
            }
            Ok((manifest, sha256)) => {
                candidates.insert(
                    PluginId(manifest.plugin_id.clone()),
                    Candidate {
                        path,
                        manifest,
                        sha256,
                    },
                );
            }
            Err(error) => {
                failed_paths.insert(path.clone());
                bus.emit(PluginLifecycleEvent::LoadFailed {
                    path,
                    reason: error.to_string(),
                });
            }
        }
    }

    let on_disk: HashSet<PluginId> = candidates.keys().cloned().collect();
    for (id, candidate) in candidates {
        let config_toml = match registry.config_for(&candidate.manifest).await {
            Ok(config) => config,
            Err(error) => {
                bus.emit(PluginLifecycleEvent::LoadFailed {
                    path: candidate.path,
                    reason: format!("reading its settings: {error}"),
                });
                continue;
            }
        };
        if registry.loaded_matches(&id, &candidate.path, &candidate.sha256, &config_toml) {
            continue;
        }
        match registry.load_from_with_config(&candidate.path, config_toml) {
            Ok(info) => {
                bus.emit(PluginLifecycleEvent::Discovered(Box::new(
                    crate::events::DiscoveredEvent {
                        path: candidate.path,
                        manifest: candidate.manifest,
                        binary_sha256: info.binary_sha256.clone(),
                    },
                )));
                bus.emit(PluginLifecycleEvent::Loaded {
                    plugin_id: PluginId(info.id),
                    binary_sha256: info.binary_sha256,
                    hooks: info.hooks,
                });
            }
            Err(error) => bus.emit(PluginLifecycleEvent::LoadFailed {
                path: candidate.path,
                reason: error.to_string(),
            }),
        }
    }

    for info in registry.snapshot() {
        let id = PluginId(info.id);
        if !on_disk.contains(&id)
            && !failed_paths.contains(&PathBuf::from(&info.path))
            && registry.unload(&id)
        {
            bus.emit(PluginLifecycleEvent::Unloaded {
                plugin_id: id,
                reason: UnloadReason::FileRemoved,
            });
        }
    }
}
