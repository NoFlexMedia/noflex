// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

use async_graphql::{ComplexObject, Context, SimpleObject};
use noflex_storage::Storage;
use time::OffsetDateTime;
use tokio::{sync::Notify, task::JoinHandle};
use wasmtime::{Engine, component::Linker};

use crate::{
    EmbedInput, EngineConfig, HostError, LoadedPlugin, PluginCtx, PluginId, PluginManifest,
    RemoteEmbedder, SecretStore, attribution,
    bindings::embedder::noflex::plugin::types as embedder_types,
    manifest::{EmbeddingClaim, HookKind},
    priority::{Overrides, PriorityTable},
    scheduler,
    settings::{self, ConfigStanzas, PluginSetting, SettingsError},
};

/// Largest batch one `embed` dispatch accepts. Model-bound plugins pay
/// per-call setup, so batching helps; the cap keeps one call well inside
/// its `embedder` deadline.
const MAX_EMBED_BATCH: usize = 64;

/// Public, owned snapshot of one plugin's state. Used by GraphQL and CLI to
/// answer "what's loaded right now?" without exposing internal Wasmtime
/// objects. The internal [`LoadedPlugin`] is not `Clone` (Components aren't),
/// so the registry returns these snapshots instead.
#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "Plugin", complex)]
pub struct Plugin {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub api_version: String,
    /// Path to the loaded `.wasm` component on disk.
    pub path: String,
    /// SHA-256 of the component binary. Stable across hot-reloads only when
    /// the file contents are unchanged; used by future tiered-trust mode to
    /// pin acknowledged plugins.
    pub binary_sha256: String,
    pub hooks: Vec<HookKind>,
    pub capabilities: Vec<String>,
    pub license: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    /// Full manifest, kept on the snapshot so lifecycle events can
    /// carry it without re-parsing the `.wasm` file. Skipped from
    /// the GQL wire shape.
    #[graphql(skip)]
    pub manifest: PluginManifest,
    /// Live health of the plugin's manifest-registered background tasks.
    pub tasks: Vec<PluginTaskHealth>,
    /// Times the instance was recycled after a trap.
    pub instance_restarts: u32,
    /// Circuit breaker: the instance is disabled after repeated traps
    /// and stays down until the plugin is reloaded.
    pub circuit_open: bool,
}

#[ComplexObject]
impl Plugin {
    /// The settings the plugin's manifest declares, each with its value now
    /// and where that comes from. A secret's value is never included.
    // Only administrators reach a `Plugin`, through `plugins` and
    // `setPluginSettings`, so this field needs no check of its own.
    async fn settings(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<PluginSetting>> {
        let registry = ctx.data::<Arc<PluginRegistry>>()?;
        registry.settings(&self.manifest).await.map_err(|e| {
            tracing::warn!(plugin = %self.id, error = %e, "reading plugin settings failed");
            async_graphql::Error::new("plugin settings are unavailable")
        })
    }
}

/// GraphQL view of one background task's scheduler health.
#[derive(Debug, Clone, SimpleObject)]
pub struct PluginTaskHealth {
    pub name: String,
    pub interval_ms: u64,
    /// When the last invocation finished (success or failure).
    pub last_run_at: Option<OffsetDateTime>,
    pub consecutive_failures: u32,
    pub total_runs: u64,
}

impl From<&LoadedPlugin> for Plugin {
    fn from(p: &LoadedPlugin) -> Self {
        let (tasks, instance_restarts, circuit_open) = {
            let health = p.health.read().expect("health lock poisoned");
            (
                health
                    .tasks
                    .iter()
                    .map(|(name, t)| PluginTaskHealth {
                        name: name.clone(),
                        interval_ms: t.interval_ms,
                        last_run_at: t.last_run_at,
                        consecutive_failures: t.consecutive_failures,
                        total_runs: t.total_runs,
                    })
                    .collect::<Vec<_>>(),
                health.instance_restarts,
                health.circuit_open,
            )
        };
        Plugin {
            id: p.id.0.clone(),
            display_name: p.manifest.display_name.clone(),
            version: p.manifest.version.clone(),
            api_version: p.manifest.noflex_api_version.clone(),
            path: p.path.display().to_string(),
            binary_sha256: p.binary_sha256.clone(),
            hooks: p.exported_hooks.clone(),
            capabilities: p.manifest.capabilities.clone(),
            license: p.manifest.license.clone(),
            author: p.manifest.author.clone(),
            homepage: p.manifest.homepage.clone(),
            manifest: p.manifest.clone(),
            tasks,
            instance_restarts,
            circuit_open,
        }
    }
}

/// The shared plugin registry — process-global handle that the scanner,
/// GraphQL resolver, and hot-reload watcher all hold via `Arc`.
///
/// Stores loaded plugins and exposes lookup to GraphQL, the hot-reload watcher,
/// lifecycle broadcasting, and the priority registry. The registry owns the
/// shared host-import linker: every plugin it loads is instantiated against
/// it, one long-lived instance per plugin.
pub struct PluginRegistry {
    engine: Arc<Engine>,
    linker: Linker<PluginCtx>,
    runtime_config: EngineConfig,
    /// Arc-shared so the domain-event dispatcher (spawned by
    /// [`Self::with_event_bus`]) can watch loads/unloads without
    /// holding the registry itself.
    plugins: Arc<RwLock<HashMap<PluginId, Arc<LoadedPlugin>>>>,
    /// Storage handle for `host-library` lookups and background-task
    /// state. None in minimal embeds: `host-library` misses, scheduled
    /// tasks run with in-memory state, and `host-tasks.enqueue` reports
    /// itself unavailable.
    storage: Option<Arc<dyn Storage>>,
    /// Root directory `host-assets.read-file` serves. None disables the
    /// import: guests get an `unavailable` plugin error.
    assets_dir: Option<PathBuf>,
    /// An embeddings API an administrator chose. While set, it takes the place of
    /// every embedder plugin.
    remote_embedder: RwLock<Option<Arc<RemoteEmbedder>>>,
    /// Signalled whenever the embedder in use may have changed.
    embedder_changed: Arc<Notify>,
    /// Background-task scheduler loops, aborted on unload or hot-reload
    /// replace. JoinHandles detach on drop, so they must live here.
    task_handles: Mutex<HashMap<PluginId, Vec<JoinHandle<()>>>>,
    /// Domain-event dispatcher loop, if [`Self::with_event_bus`] was
    /// used. Detaches on drop like the scheduler handles.
    dispatcher_handle: Mutex<Option<JoinHandle<()>>>,
    /// Plugin ids an operator disabled, which never load.
    disabled: HashSet<String>,
    /// The server's config.toml stanzas, which admin-set settings layer
    /// over (see [`Self::config_for`]).
    config: ConfigStanzas,
    /// Held while a plugin's config is read and the plugin (re)loaded with
    /// it, so a sync that read settings before a change can't load over the
    /// reload that change made.
    load_lock: tokio::sync::Mutex<()>,
}

impl PluginRegistry {
    /// Create an empty registry and wire the host-import linker against
    /// `engine`. Fails only if the WASI/host imports can't be linked.
    pub fn new(engine: Arc<Engine>) -> Result<Self, HostError> {
        Self::new_with_config(engine, EngineConfig::default())
    }

    pub fn new_with_config(
        engine: Arc<Engine>,
        runtime_config: EngineConfig,
    ) -> Result<Self, HostError> {
        let linker = crate::build_linker(&engine)
            .map_err(|e| HostError::Wasm(format!("build linker: {e}")))?;
        Ok(Self {
            engine,
            linker,
            runtime_config,
            plugins: Arc::new(RwLock::new(HashMap::new())),
            storage: None,
            assets_dir: None,
            remote_embedder: RwLock::new(None),
            embedder_changed: Arc::new(Notify::new()),
            task_handles: Mutex::new(HashMap::new()),
            dispatcher_handle: Mutex::new(None),
            disabled: HashSet::new(),
            config: ConfigStanzas::new(),
            load_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// Never load the plugins with these ids: they get no instance, so no
    /// hook, task or event reaches them. Applies to loads after the call.
    pub fn with_disabled(mut self, ids: impl IntoIterator<Item = String>) -> Self {
        self.disabled = ids.into_iter().collect();
        self
    }

    /// Whether an operator disabled `id`.
    pub fn is_disabled(&self, id: &str) -> bool {
        self.disabled.contains(id)
    }

    /// Give the registry the server's `[plugin.per_plugin_config.<id>]`
    /// stanzas. A plugin's config is its stanza with admin-set settings
    /// over it; see [`Self::config_for`].
    pub fn with_plugin_config(mut self, config: ConfigStanzas) -> Self {
        self.config = config;
        self
    }

    /// Give the registry (and every plugin it loads) a storage handle.
    pub fn with_storage(mut self, storage: Arc<dyn Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Give the registry the assets directory `host-assets.read-file`
    /// resolves guest names under. Applies to plugins loaded after the
    /// call, like [`Self::with_storage`].
    pub fn with_assets_dir(mut self, assets_dir: Option<PathBuf>) -> Self {
        self.assets_dir = assets_dir;
        self
    }

    /// Subscribe the plugin host to the domain-event bus: spawns the
    /// dispatcher task that fans every event out to loaded plugins
    /// exporting the `notifier` hook. Must be called within a tokio
    /// runtime.
    pub fn with_event_bus(self, bus: noflex_core::events::DomainEventBus) -> Self {
        let handle = crate::dispatcher::spawn_on(&bus, Arc::clone(&self.plugins));
        *self
            .dispatcher_handle
            .lock()
            .expect("dispatcher handle poisoned") = Some(handle);
        self
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The shared host-import linker every registered plugin was
    /// instantiated against.
    pub fn linker(&self) -> &Linker<PluginCtx> {
        &self.linker
    }

    /// Load a single plugin from disk and register it. Returns the
    /// `Plugin` snapshot. On duplicate `plugin_id`, the newer one
    /// shadows the older — matching the tiered search-path policy where
    /// user-tier installs shadow system-tier ones.
    ///
    /// Must be called from within a tokio runtime: the plugin's instance
    /// task is spawned eagerly.
    pub fn load_from(&self, path: &Path) -> Result<Plugin, HostError> {
        self.load_from_with_config(path, String::new())
    }

    /// Load a plugin and attach a per-plugin config blob. The blob
    /// is rendered back to the guest through `host-config::read`.
    ///
    /// # Errors
    /// [`HostError::Disabled`] for a plugin [`Self::with_disabled`] names,
    /// before any of it is instantiated; otherwise whatever
    /// [`LoadedPlugin::load_with_context`] rejects it for.
    pub fn load_from_with_config(
        &self,
        path: &Path,
        config_toml: String,
    ) -> Result<Plugin, HostError> {
        let manifest = crate::plugin::extract_manifest(&std::fs::read(path)?, path)?;
        if self.is_disabled(&manifest.plugin_id) {
            return Err(HostError::Disabled {
                id: manifest.plugin_id,
            });
        }
        let plugin = LoadedPlugin::load_with_context(
            &self.engine,
            &self.linker,
            path,
            config_toml,
            self.storage.clone(),
            self.assets_dir.clone(),
            &self.runtime_config,
        )?;
        let info = Plugin::from(&plugin);
        let id = plugin.id.clone();
        let plugin = Arc::new(plugin);
        {
            self.embedder_changed.notify_one();
            let mut guard = self.plugins.write().expect("registry mutex poisoned");
            guard.insert(id.clone(), Arc::clone(&plugin));
            self.embedder_changed.notify_one();
        }
        if plugin.exports(HookKind::Metadata)
            && let Some(storage) = self.storage.clone()
        {
            let plugin = Arc::clone(&plugin);
            tokio::spawn(async move { attribution::sync(&plugin, storage.as_ref()).await });
        }
        let handles = if plugin.manifest.tasks.is_empty() {
            Vec::new()
        } else {
            scheduler::spawn_for_plugin(&plugin, self.storage.clone())
        };
        let mut map = self.task_handles.lock().expect("task handles poisoned");
        if let Some(old) = map.insert(id, handles) {
            for handle in old {
                handle.abort();
            }
        }
        Ok(info)
    }

    /// The config `host-config.read` hands `manifest`'s plugin: its
    /// config.toml stanza, with each non-secret setting an administrator
    /// set in storage over it and manifest defaults for the rest.
    ///
    /// # Errors
    ///
    /// Storage failing to read the settings.
    pub(crate) async fn config_for(
        &self,
        manifest: &PluginManifest,
    ) -> Result<String, noflex_storage::error::StorageError> {
        let admin = match &self.storage {
            Some(storage) => storage.plugin_settings(&manifest.plugin_id).await?,
            None => Default::default(),
        };
        Ok(settings::render(
            manifest,
            self.config.get(&manifest.plugin_id),
            &admin,
        ))
    }

    /// Taken around reading a plugin's config and loading it with that.
    pub(crate) async fn load_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.load_lock.lock().await
    }

    /// `manifest`'s settings, each resolved to its value and source.
    ///
    /// # Errors
    ///
    /// Storage failing to read the settings.
    pub async fn settings(
        &self,
        manifest: &PluginManifest,
    ) -> Result<Vec<PluginSetting>, SettingsError> {
        let (admin, admin_secrets) = match &self.storage {
            Some(storage) => (
                storage.plugin_settings(&manifest.plugin_id).await?,
                storage
                    .plugin_secret_setting_keys(&manifest.plugin_id)
                    .await?,
            ),
            None => Default::default(),
        };
        Ok(settings::view(
            manifest,
            self.config.get(&manifest.plugin_id),
            &admin,
            &admin_secrets,
            &SecretStore::with_default_tiers(),
        ))
    }

    /// Set or clear several of a loaded plugin's settings at once, then
    /// reload it with them, as plugins may have read their config and
    /// secrets at start. A `None` or empty value clears the key, falling
    /// back to config.toml or a secret file, then the manifest default.
    /// Each secret written is audit-logged against `actor`, without its
    /// value.
    ///
    /// # Errors
    ///
    /// [`SettingsError::is_bad_input`] when `changes` don't fit the
    /// plugin's schema, and nothing is written. Otherwise storage or the
    /// reload failing; a failed reload keeps the values written.
    pub async fn update_settings(
        &self,
        id: &PluginId,
        changes: Vec<(String, Option<String>)>,
        actor: &str,
    ) -> Result<Plugin, SettingsError> {
        let plugin = self.get_plugin(id).ok_or(SettingsError::NotLoaded)?;
        let storage = self.storage.as_ref().ok_or(SettingsError::NoStorage)?;
        let writes = settings::plan(
            &plugin.manifest,
            self.config.get(&id.0),
            &SecretStore::with_default_tiers(),
            changes,
        )?;
        storage.update_plugin_settings(&id.0, &writes).await?;
        for write in writes.iter().filter(|w| w.secret) {
            crate::audit::secret_setting_write(&id.0, &write.key, actor, write.value.is_none());
        }
        let _loading = self.load_lock().await;
        let config = self.config_for(&plugin.manifest).await?;
        Ok(self.load_from_with_config(&plugin.path, config)?)
    }

    pub(crate) fn loaded_matches(
        &self,
        id: &PluginId,
        path: &Path,
        binary_sha256: &str,
        config_toml: &str,
    ) -> bool {
        let guard = self.plugins.read().expect("registry mutex poisoned");
        guard.get(id).is_some_and(|plugin| {
            plugin.path == path
                && plugin.binary_sha256 == binary_sha256
                && plugin.config_toml == config_toml
        })
    }

    /// Remove a plugin from the registry by ID.
    pub fn unload(&self, id: &PluginId) -> bool {
        if let Some(handles) = self
            .task_handles
            .lock()
            .expect("task handles poisoned")
            .remove(id)
        {
            for h in handles {
                h.abort();
            }
        }
        let mut guard = self.plugins.write().expect("registry mutex poisoned");
        guard.remove(id).is_some()
    }

    /// Snapshot of all loaded plugins, in deterministic order by ID.
    pub fn snapshot(&self) -> Vec<Plugin> {
        let guard = self.plugins.read().expect("registry mutex poisoned");
        let mut v: Vec<Plugin> = guard.values().map(|p| Plugin::from(p.as_ref())).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// Lookup a plugin by ID, returning a clone of the public info.
    pub fn get(&self, id: &PluginId) -> Option<Plugin> {
        let guard = self.plugins.read().expect("registry mutex poisoned");
        guard.get(id).map(|p| Plugin::from(p.as_ref()))
    }

    /// Lookup the underlying [`LoadedPlugin`] for internal dispatch use.
    /// The arc is cloned so callers can hold it across awaits without
    /// keeping the registry lock held.
    pub fn get_plugin(&self, id: &PluginId) -> Option<Arc<LoadedPlugin>> {
        let guard = self.plugins.read().expect("registry mutex poisoned");
        guard.get(id).cloned()
    }

    /// The live plugin map, shared with the domain-event dispatcher.
    pub(crate) fn plugins_handle(&self) -> Arc<RwLock<HashMap<PluginId, Arc<LoadedPlugin>>>> {
        Arc::clone(&self.plugins)
    }

    /// Enqueue a dynamic payload for one of a plugin's manifest-registered
    /// background tasks — the host-side entry point for user-initiated
    /// "sync now" flows. Same rules as the guest's `host-tasks.enqueue`
    /// import: the plugin must be loaded and declare the `background`
    /// capability, and `task` must be one of its manifest `[[tasks]]`
    /// names — the scheduler drains the queue per registered name, so an
    /// unregistered name would never drain. The payload persists in
    /// storage and drains FIFO on the plugin's instance.
    pub async fn enqueue_task(
        &self,
        plugin_id: &str,
        task: &str,
        payload: Option<String>,
    ) -> Result<(), HostError> {
        let plugin = self
            .get_plugin(&PluginId(plugin_id.to_string()))
            .ok_or_else(|| HostError::Unknown {
                id: plugin_id.to_string(),
            })?;
        if !crate::manifest::declares_background(&plugin.manifest.capabilities) {
            crate::audit::task_enqueue(plugin_id, task, false);
            return Err(HostError::MissingCapability {
                id: plugin_id.to_string(),
                capability: "background",
            });
        }
        if !plugin.manifest.tasks.iter().any(|t| t.name == task) {
            crate::audit::task_enqueue(plugin_id, task, false);
            let registered = plugin
                .manifest
                .tasks
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(HostError::UnknownTask {
                id: plugin_id.to_string(),
                task: task.to_string(),
                registered,
            });
        }
        if payload
            .as_ref()
            .is_some_and(|payload| payload.len() > crate::dispatch::MAX_TASK_PAYLOAD_BYTES)
        {
            crate::audit::task_enqueue(plugin_id, task, false);
            return Err(HostError::TaskPayloadTooLarge {
                limit_bytes: crate::dispatch::MAX_TASK_PAYLOAD_BYTES,
            });
        }
        let Some(storage) = &self.storage else {
            crate::audit::task_enqueue(plugin_id, task, false);
            return Err(HostError::TaskQueueUnavailable);
        };
        let stored = crate::scheduler::encode_queued(None, payload);
        let result = storage
            .enqueue_plugin_task(plugin_id, task, stored.as_deref())
            .await;
        crate::audit::task_enqueue(plugin_id, task, result.is_ok());
        result.map_err(HostError::from)
    }

    /// Drop `user`'s background chains on `plugin_id`, as when they unlink
    /// from it: an enqueued payload of theirs that already started a chain
    /// stops, and its persisted cursor goes. Returns once no run acting for
    /// them is in flight, or after `wait` at most. A plugin that isn't
    /// loaded has no chain running; one it persisted resumes if it loads.
    pub async fn cancel_user_tasks(
        &self,
        plugin_id: &str,
        user: noflex_core::UserId,
        wait: std::time::Duration,
    ) {
        let Some(plugin) = self.get_plugin(&PluginId(plugin_id.to_string())) else {
            return;
        };
        let acks = plugin.cancel_user_tasks(user);
        drop(plugin);
        if tokio::time::timeout(wait, futures::future::join_all(acks))
            .await
            .is_err()
        {
            tracing::warn!(plugin = plugin_id, %user, "background chains still running after cancel");
        }
    }

    /// Fan a finished scan's work-id lists out to every loaded plugin
    /// exporting the `post-scan` hook. Per-plugin errors and traps are
    /// logged and isolated (mirroring the dispatcher's notifier fan-out).
    pub async fn post_scan(
        &self,
        library: noflex_core::LibraryId,
        added: Vec<noflex_core::WorkId>,
        updated: Vec<noflex_core::WorkId>,
        removed: Vec<noflex_core::WorkId>,
    ) {
        crate::dispatcher::post_scan_fanout(&self.plugins, library, added, updated, removed).await;
    }

    /// IDs of plugins exporting the given hook. Used by the dispatch logic
    /// to fan out search() calls and to drive the priority registry.
    pub fn exporters_of(&self, hook: HookKind) -> Vec<PluginId> {
        let guard = self.plugins.read().expect("registry mutex poisoned");
        guard
            .values()
            .filter(|p| p.exports(hook))
            .map(|p| p.id.clone())
            .collect()
    }

    /// What runs the models behind `host-inference`.
    pub fn inference(&self) -> &Arc<crate::Inference> {
        &self.runtime_config.inference
    }

    /// Use an embeddings API in place of every embedder plugin, or with `None`
    /// go back to them.
    pub fn set_remote_embedder(&self, embedder: Option<Arc<RemoteEmbedder>>) {
        *self
            .remote_embedder
            .write()
            .expect("registry mutex poisoned") = embedder;
        self.embedder_changed.notify_one();
    }

    /// The embeddings API in use, if one is configured.
    pub fn remote_embedder(&self) -> Option<Arc<RemoteEmbedder>> {
        self.remote_embedder
            .read()
            .expect("registry mutex poisoned")
            .clone()
    }

    /// Signalled when the embedder in use may have changed; a permit waits if
    /// nobody is listening yet.
    pub fn embedder_changed(&self) -> Arc<Notify> {
        Arc::clone(&self.embedder_changed)
    }

    /// What the embedder in use stores vectors under: the configured embeddings
    /// API, else the `[embedding]` claim of the highest-priority loaded embedder
    /// plugin, else None. Plugin ordering follows the metadata-candidate rules
    /// (manifest `[priority]` defaults, ties by plugin id); embedders are
    /// work-kind agnostic, so the ranking is hook-level only.
    pub fn embedder_claim(&self) -> Option<EmbeddingClaim> {
        if let Some(remote) = self.remote_embedder() {
            return Some(remote.claim().clone());
        }
        self.embedder_plugin()
            .and_then(|p| p.manifest.embedding.clone())
    }

    /// The highest-priority loaded plugin exporting the embedder hook.
    /// Ranking comes from a [`PriorityTable`] built over manifest
    /// defaults; when no embedder declares a priority, every exporter is
    /// a candidate (mirroring `MetadataService::candidates`' fallback),
    /// ties broken by plugin id for determinism.
    fn embedder_plugin(&self) -> Option<Arc<LoadedPlugin>> {
        let guard = self.plugins.read().expect("registry mutex poisoned");
        let table = PriorityTable::build(
            guard.iter().map(|(id, p)| (id, &p.manifest)),
            &Overrides::default(),
        );
        let mut candidates: Vec<PluginId> = table
            .ranked_hook(HookKind::Embedder)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        if candidates.is_empty() {
            candidates = guard
                .values()
                .filter(|p| p.exports(HookKind::Embedder))
                .map(|p| p.id.clone())
                .collect();
            candidates.sort();
        }
        candidates
            .into_iter()
            .find_map(|id| guard.get(&id).cloned())
    }
}

/// One completed `embed` batch: the winning plugin's claim plus one
/// vector per input, in input order. Downstream (the embedding
/// worker, the GraphQL search resolver) persists `namespace` and
/// `model_id` alongside every vector.
#[derive(Debug, Clone)]
pub struct EmbedOutcome {
    pub namespace: String,
    pub model_id: String,
    pub vectors: Vec<Vec<f32>>,
}

/// Embed a batch with the highest-priority loaded embedder plugin. Returns
/// Ok(None) when no embedder is loaded (and on empty input, which never
/// dispatches). Batches over [`MAX_EMBED_BATCH`] inputs are a host error — split them caller-side. The guest's vectors
/// are validated against its manifest claim before they reach the
/// caller (see [`LoadedPlugin::call_embed`]).
pub async fn embed_inputs(
    registry: &PluginRegistry,
    inputs: Vec<EmbedInput>,
) -> Result<Option<EmbedOutcome>, HostError> {
    if inputs.is_empty() {
        return Ok(None);
    }
    if inputs.len() > MAX_EMBED_BATCH {
        return Err(HostError::Wasm(format!(
            "embed batch of {} inputs exceeds the {MAX_EMBED_BATCH}-input cap",
            inputs.len()
        )));
    }
    if let Some(remote) = registry.remote_embedder() {
        let vectors = remote.embed(&inputs).await?;
        let claim = remote.claim();
        return Ok(Some(EmbedOutcome {
            namespace: claim.namespace.clone(),
            model_id: claim.model_id.clone(),
            vectors,
        }));
    }
    let Some(plugin) = registry.embedder_plugin() else {
        return Ok(None);
    };
    let claim = plugin.manifest.embedding.clone().ok_or_else(|| {
        HostError::Wasm(format!(
            "{}: embedder plugin has no [embedding] claim",
            plugin.id
        ))
    })?;
    let vectors = plugin
        .call_embed(inputs)
        .await?
        .map_err(|e| HostError::Plugin {
            id: plugin.id.0.clone(),
            source: core_plugin_error(e),
        })?;
    Ok(Some(EmbedOutcome {
        namespace: claim.namespace,
        model_id: claim.model_id,
        vectors,
    }))
}

/// Re-tag the embedder binding's `plugin-error` onto the canonical core
/// type for [`HostError::Plugin`]. Variants core lacks (trapped,
/// capability-denied) fold into `Internal`, keeping the message.
fn core_plugin_error(e: embedder_types::PluginError) -> noflex_core::PluginError {
    use embedder_types::PluginError as Wit;
    use noflex_core::PluginError as Core;
    match e {
        Wit::Unavailable(m) => Core::Unavailable(m),
        Wit::BadInput(m) => Core::BadInput(m),
        Wit::NotFound => Core::NotFound,
        Wit::RateLimited(s) => Core::RateLimited {
            retry_after_secs: s,
        },
        Wit::Internal(m) | Wit::Trapped(m) | Wit::CapabilityDenied(m) => Core::Internal(m),
    }
}

/// Fan out a `SearchQuery` (canonical core type) to every loaded plugin
/// exporting the [`HookKind::Metadata`] hook. Returns the merged list of
/// matches; on a per-plugin dispatch error the plugin is skipped and
/// the error is logged but the search continues.
///
/// Providers are queried concurrently — each plugin's single instance
/// services its own call, and plugins manage their own upstream
/// rate-limiting.
///
/// The two crates that consume this (GraphQL resolvers, the scanner)
/// both want the same fan-out semantics, so it's a free function rather
/// than a method — neither caller holds an exclusive borrow of the
/// registry at call time.
pub async fn metadata_fanout(
    registry: &PluginRegistry,
    query: &noflex_core::providers::SearchQuery,
) -> Vec<noflex_core::providers::MetadataMatch> {
    use noflex_core::{model::ExternalId, providers::MetadataMatch};

    use crate::plugin_types::SearchQuery as WitSearchQuery;

    let ids = registry.exporters_of(HookKind::Metadata);
    if ids.is_empty() {
        return Vec::new();
    }
    let wit_query = WitSearchQuery {
        title: query.title.clone(),
        year: query.year,
        kind: to_wit_kind(query.kind),
        hints: query.hints.clone(),
    };
    let mut calls = Vec::new();
    for id in ids {
        let Some(plugin) = registry.get_plugin(&id) else {
            continue;
        };
        let wit_query = wit_query.clone();
        calls.push(async move { (id, plugin.call_search(wit_query).await) });
    }
    let mut out = Vec::new();
    for (id, result) in futures::future::join_all(calls).await {
        match result {
            Ok(Ok(matches)) => {
                for m in matches {
                    out.push(MetadataMatch {
                        external_id: ExternalId {
                            kind: from_wit_kind_id(m.external_id.kind),
                            value: m.external_id.value,
                        },
                        title: m.title,
                        year: m.year,
                        confidence: m.confidence,
                        provider: id.0.clone(),
                    });
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(plugin = %id.0, error = ?e, "metadata fanout: plugin returned error");
            }
            Err(e) => {
                tracing::warn!(plugin = %id.0, error = %e, "metadata fanout: dispatch failed");
            }
        }
    }
    out
}

/// Retries of one candidate's `fetch` while its call queue is full, and the
/// pause before the first retry (doubled per attempt).
const OVERLOADED_RETRIES: u32 = 4;
const OVERLOADED_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// Fetch full metadata for a work from the first candidate plugin that
/// succeeds. Provider-exclusive per the architecture invariants: exactly
/// one provider supplies a work's fields; the next candidate is tried
/// only when the current one's fetch() fails (dispatch error or plugin
/// error — both logged). `external_ids` are tried in order against each
/// candidate. Returns the winning plugin id and payload, or None when
/// every candidate failed for every id.
///
/// [`HostError::Overloaded`] is not a fetch failure — the fetch never ran —
/// so it does not advance to a lower-priority provider: the candidate is
/// retried with backoff, and if it stays overloaded the walk stops and
/// returns `None`, leaving the work to a later attempt rather than letting
/// a transient queue spike pick its provider.
pub async fn metadata_fetch(
    registry: &PluginRegistry,
    candidates: &[PluginId],
    external_ids: &[noflex_core::model::ExternalId],
) -> Option<(PluginId, noflex_core::providers::MetadataPayload)> {
    use crate::plugin_types::ExternalId as WitExternalId;

    for id in candidates {
        let Some(plugin) = registry.get_plugin(id) else {
            continue;
        };
        for external_id in external_ids {
            let wit_id = WitExternalId {
                kind: to_wit_kind_id(external_id.kind),
                value: external_id.value.clone(),
            };
            let mut backoff = OVERLOADED_BACKOFF;
            let mut attempt = 0;
            let outcome = loop {
                match plugin.call_fetch(wit_id.clone()).await {
                    Err(HostError::Overloaded { .. }) if attempt < OVERLOADED_RETRIES => {
                        attempt += 1;
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                    }
                    outcome => break outcome,
                }
            };
            match outcome {
                Ok(Ok(payload)) => {
                    return Some((id.clone(), metadata_payload_from_wit(payload)));
                }
                Ok(Err(e)) => {
                    tracing::warn!(plugin = %id.0, error = ?e, "metadata fetch: plugin returned error");
                }
                Err(e @ HostError::Overloaded { .. }) => {
                    tracing::warn!(plugin = %id.0, error = %e,
                        "metadata fetch: provider stayed overloaded; not falling back");
                    return None;
                }
                Err(e) => {
                    tracing::warn!(plugin = %id.0, error = %e, "metadata fetch: dispatch failed");
                }
            }
        }
    }
    None
}

/// Convert a WIT metadata payload into the canonical core type. Shared
/// by [`metadata_fetch`] and the GraphQL `identifyWork` path, which
/// calls one provider explicitly rather than walking candidates.
pub fn metadata_payload_from_wit(
    payload: crate::plugin_metadata::MetadataPayload,
) -> noflex_core::providers::MetadataPayload {
    use noflex_core::{
        model::ExternalId,
        providers::{
            MetadataPayload, Rating,
            metadata::{MetadataImage, MetadataPerson},
        },
    };

    MetadataPayload {
        external_ids: payload
            .external_ids
            .into_iter()
            .map(|e| ExternalId {
                kind: from_wit_kind_id(e.kind),
                value: e.value,
            })
            .collect(),
        title: payload.title,
        sort_title: payload.sort_title,
        original_title: payload.original_title,
        year: payload.year,
        release_date: payload.release_date,
        overview: payload.overview,
        tagline: payload.tagline,
        runtime_ms: payload.runtime_ms,
        genres: payload.genres,
        studios: payload.studios,
        people: payload
            .people
            .into_iter()
            .map(|p| MetadataPerson {
                name: p.name,
                role: p.role,
                character: p.character,
                external_ids: p
                    .external_ids
                    .into_iter()
                    .map(|e| ExternalId {
                        kind: from_wit_kind_id(e.kind),
                        value: e.value,
                    })
                    .collect(),
            })
            .collect(),
        images: payload
            .images
            .into_iter()
            .map(|i| MetadataImage {
                kind: from_wit_image_kind(i.kind),
                url: i.url,
                language: i.language,
                width: i.width,
                height: i.height,
            })
            .collect(),
        rating: payload.rating,
        ratings: payload
            .ratings
            .into_iter()
            .map(|r| Rating {
                source: r.source,
                value: r.value,
                scale: r.scale,
                votes: r.votes,
            })
            .collect(),
        original_language: payload.original_language,
        origin_countries: payload.origin_countries,
        keywords: payload.keywords,
        budget_usd: payload.budget_usd,
    }
}

pub(crate) fn to_wit_kind(k: noflex_core::model::WorkKind) -> crate::plugin_types::WorkKind {
    use noflex_core::model::WorkKind;

    use crate::plugin_types::WorkKind as WitWorkKind;
    match k {
        WorkKind::Movie => WitWorkKind::Movie,
        WorkKind::Series => WitWorkKind::Series,
        WorkKind::Season => WitWorkKind::Season,
        WorkKind::Episode => WitWorkKind::Episode,
        WorkKind::Album => WitWorkKind::Album,
        WorkKind::Track => WitWorkKind::Track,
        WorkKind::Artist => WitWorkKind::Artist,
        WorkKind::AudioBook => WitWorkKind::AudioBook,
        WorkKind::Photo => WitWorkKind::Photo,
    }
}

pub(crate) fn from_wit_kind_id(
    k: crate::plugin_types::ExternalIdKind,
) -> noflex_core::model::ExternalIdKind {
    use noflex_core::model::ExternalIdKind;

    use crate::plugin_types::ExternalIdKind as WitExternalIdKind;
    match k {
        WitExternalIdKind::Tmdb => ExternalIdKind::Tmdb,
        WitExternalIdKind::Tvdb => ExternalIdKind::Tvdb,
        WitExternalIdKind::Imdb => ExternalIdKind::Imdb,
        WitExternalIdKind::MusicBrainzReleaseGroup => ExternalIdKind::MusicBrainzReleaseGroup,
        WitExternalIdKind::MusicBrainzArtist => ExternalIdKind::MusicBrainzArtist,
        WitExternalIdKind::MusicBrainzRecording => ExternalIdKind::MusicBrainzRecording,
        WitExternalIdKind::AniDb => ExternalIdKind::AniDb,
        WitExternalIdKind::Custom => ExternalIdKind::Custom,
    }
}

/// Core → WIT external-id kind. Public so the GraphQL `identifyWork`
/// mutation can build the WIT id for an explicit single-provider fetch.
pub fn to_wit_kind_id(
    k: noflex_core::model::ExternalIdKind,
) -> crate::plugin_types::ExternalIdKind {
    use noflex_core::model::ExternalIdKind;

    use crate::plugin_types::ExternalIdKind as WitExternalIdKind;
    match k {
        ExternalIdKind::Tmdb => WitExternalIdKind::Tmdb,
        ExternalIdKind::Tvdb => WitExternalIdKind::Tvdb,
        ExternalIdKind::Imdb => WitExternalIdKind::Imdb,
        ExternalIdKind::MusicBrainzReleaseGroup => WitExternalIdKind::MusicBrainzReleaseGroup,
        ExternalIdKind::MusicBrainzArtist => WitExternalIdKind::MusicBrainzArtist,
        ExternalIdKind::MusicBrainzRecording => WitExternalIdKind::MusicBrainzRecording,
        ExternalIdKind::AniDb => WitExternalIdKind::AniDb,
        ExternalIdKind::Custom => WitExternalIdKind::Custom,
    }
}

fn from_wit_image_kind(k: crate::plugin_types::ImageKind) -> noflex_core::model::ImageKind {
    use noflex_core::model::ImageKind;

    use crate::plugin_types::ImageKind as WitImageKind;
    match k {
        WitImageKind::Primary => ImageKind::Primary,
        WitImageKind::Backdrop => ImageKind::Backdrop,
        WitImageKind::Banner => ImageKind::Banner,
        WitImageKind::Thumb => ImageKind::Thumb,
        WitImageKind::Logo => ImageKind::Logo,
        WitImageKind::Disc => ImageKind::Disc,
        WitImageKind::Box => ImageKind::Box,
        WitImageKind::Menu => ImageKind::Menu,
        WitImageKind::Chapter => ImageKind::Chapter,
        WitImageKind::Art => ImageKind::Art,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineConfig, engine::build_engine};

    #[test]
    fn empty_registry_has_no_plugins() {
        let engine = build_engine(&EngineConfig::default()).unwrap();
        let reg = PluginRegistry::new(engine).unwrap();
        assert!(reg.snapshot().is_empty());
    }
}
