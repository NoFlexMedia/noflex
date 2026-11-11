// SPDX-License-Identifier: AGPL-3.0-only
//
// Per-plugin context threaded into wasmtime's `Store::data` plus the
// host-side implementations of every `noflex:plugin/host-*` import.

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use noflex_storage::Storage;
use wasmtime::{
    AsContextMut as _, StoreContextMut,
    component::{Access, Accessor, GuestTaskId, HasSelf},
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::{
    PluginId, PluginLimitState, PluginLimits,
    bindings::{
        embedder::noflex::plugin::{host_assets, host_inference, types as embedder_types},
        metadata_only::{
            MetadataPlugin,
            noflex::plugin::{
                host_config, host_library, host_log, host_metrics, host_secrets, host_tasks, types,
            },
        },
    },
};

/// Hard cap on one `host-assets.read-file` payload. Model weights are
/// the intended cargo, so the ceiling is generous; it exists to stop a
/// guest from OOMing the host by reading an unbounded file.
const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_SECRET_KEY_BYTES: usize = 128;
pub(crate) const MAX_SECRET_VALUE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_TASK_PAYLOAD_BYTES: usize = 1024 * 1024;

/// The in-flight export calls of one store, keyed by the component-model
/// task the host started for each. Records what a host import or the epoch
/// callback needs to know about "the call currently running": which user it
/// acts for and when its deadline passes. Lookups walk the current async
/// call stack to its root task, so guest-spawned futures inherit their own
/// call's entry and never observe another call's.
///
/// Every dispatched call registers an entry (`user: None` when unbound), so
/// a reused task slot never inherits a stale binding. [`CallRegistration`]
/// removes the entry on drop, which makes cleanup cancel-safe; the sequence
/// number keeps a late drop from evicting a newer call in the same slot.
#[derive(Clone, Default)]
pub(crate) struct CallTable(Arc<Mutex<CallTableInner>>);

#[derive(Default)]
struct CallTableInner {
    next_seq: u64,
    calls: HashMap<GuestTaskId, CallEntry>,
}

struct CallEntry {
    seq: u64,
    user: Option<noflex_core::UserId>,
    deadline: Instant,
}

/// Live registration of one call; dropping it unregisters.
pub(crate) struct CallRegistration {
    table: CallTable,
    task: GuestTaskId,
    seq: u64,
}

impl Drop for CallRegistration {
    fn drop(&mut self) {
        let mut table = self.table.lock();
        if table
            .calls
            .get(&self.task)
            .is_some_and(|entry| entry.seq == self.seq)
        {
            table.calls.remove(&self.task);
        }
    }
}

impl CallTable {
    fn lock(&self) -> std::sync::MutexGuard<'_, CallTableInner> {
        // Plain data with no cross-field invariants: a poisoned lock is
        // still consistent.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn register(
        &self,
        task: GuestTaskId,
        user: Option<noflex_core::UserId>,
        deadline: Instant,
    ) -> CallRegistration {
        let mut table = self.lock();
        table.next_seq += 1;
        let seq = table.next_seq;
        table.calls.insert(
            task,
            CallEntry {
                seq,
                user,
                deadline,
            },
        );
        CallRegistration {
            table: self.clone(),
            task,
            seq,
        }
    }

    fn with_entry<R>(
        &self,
        stack: impl IntoIterator<Item = GuestTaskId>,
        f: impl FnOnce(&CallEntry) -> R,
    ) -> Option<R> {
        let table = self.lock();
        stack
            .into_iter()
            .find_map(|task| table.calls.get(&task))
            .map(f)
    }

    /// True when the call running on `store`'s current thread has passed
    /// its deadline. Called from the epoch callback.
    pub(crate) fn current_overdue(&self, store: &mut StoreContextMut<'_, PluginCtx>) -> bool {
        let Ok(stack) = store.async_call_stack() else {
            return false;
        };
        let now = Instant::now();
        self.with_entry(stack, |entry| entry.deadline <= now)
            .unwrap_or(false)
    }
}

/// The user the calling export task acts for, or `None` for unbound calls.
///
/// Must run before the host import's first `.await`: only the import's
/// first poll is guaranteed to see the calling guest task as the store's
/// current thread.
pub(crate) fn current_principal(
    access: &mut Access<'_, PluginCtx, HasSelf<PluginCtx>>,
) -> Option<noflex_core::UserId> {
    let calls = access.get().calls.clone();
    let mut store = access.as_context_mut();
    let stack = store.async_call_stack().ok()?;
    calls.with_entry(stack, |entry| entry.user).flatten()
}

pub(crate) fn has_capability(capabilities: &[String], capability: &str) -> bool {
    capabilities.iter().any(|declared| declared == capability)
}

fn valid_secret_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_SECRET_KEY_BYTES
}

/// State threaded into the wasmtime `Store::data` for every plugin call.
/// Identifies which plugin is being dispatched (so capability uses can be
/// audit-logged with the right `plugin_id`) and exposes the limited
/// surface plugins can call back into the host.
///
/// Also embeds a WASI 0.2 context for the runtime-level imports
/// (`wasi:io`, `wasi:cli`, etc.) that wit-bindgen's adapter drags in when
/// targeting `wasm32-wasip2`. v1 grants no filesystem preopens and an
/// inherited stdio; tighter sandboxing follows in the capability-grant
/// rollout.
pub struct PluginCtx {
    pub plugin_id: PluginId,
    /// Plugin-specific config blob (TOML text). The host loads the
    /// stanza scoped to this plugin and hands it through `host-config.read`.
    pub config_toml: String,
    /// Plugin's manifest-declared `secrets_read` allowlist. Keys not
    /// on this list are denied at `host-secrets.read` time.
    pub declared_secrets: Vec<String>,
    /// Plugin's manifest-declared `user_secrets_read` allowlist, gating
    /// `host-secrets.read-for-user` the same way `declared_secrets`
    /// gates `host-secrets.read`.
    pub declared_user_secrets: Vec<String>,
    /// Plugin's manifest-declared `user_secrets_write` allowlist, gating
    /// `host-secrets.write-for-user`. Kept separate from the read list:
    /// reading a token back does not imply license to overwrite it.
    pub declared_user_secrets_write: Vec<String>,
    /// Plugin's validated manifest capabilities; every privileged host
    /// import checks its matching declaration before doing work.
    pub declared_capabilities: Vec<String>,
    /// Names accepted by `host-tasks.enqueue`; unknown names would create
    /// queue rows no scheduler can drain.
    pub declared_tasks: Vec<String>,
    /// Plugin's manifest-declared `http_egress_hosts` allowlist, gating
    /// `host-net.request` (alongside the `http.egress` capability).
    pub http_egress_hosts: Vec<String>,
    /// Host-side storage handle, used by `host-library` to satisfy
    /// read-only lookups without ever exposing the trait itself to
    /// the plugin.
    pub storage: Option<Arc<dyn Storage>>,
    /// Root directory `host-assets.read-file` resolves names under.
    /// `None` disables the import with an `unavailable` plugin error.
    pub assets_dir: Option<PathBuf>,
    /// Runs the models behind `host-inference`, shared across plugins.
    pub inference: Arc<crate::Inference>,
    pub limits: PluginLimitState,
    /// In-flight export calls: who each acts for and when it is due.
    pub(crate) calls: CallTable,
    /// The manifest's `tag_namespaces`, the only ones `host-tags` writes.
    pub tag_namespaces: Vec<String>,
    /// Where library writes are announced, so search, recommendations and
    /// the planner hear of them.
    pub events: Option<noflex_core::DomainEventBus>,

    wasi: WasiCtx,
    table: ResourceTable,
}

/// Everything needed to build a fresh [`PluginCtx`]. The instance task
/// keeps these parts so it can rebuild the store's data when it recycles
/// a trapped instance (a consumed `WasiCtx` can't be reused across
/// stores).
#[derive(Clone)]
pub struct PluginCtxParts {
    pub plugin_id: PluginId,
    pub config_toml: String,
    pub declared_secrets: Vec<String>,
    pub declared_user_secrets: Vec<String>,
    pub declared_user_secrets_write: Vec<String>,
    pub declared_capabilities: Vec<String>,
    pub declared_tasks: Vec<String>,
    pub http_egress_hosts: Vec<String>,
    pub storage: Option<Arc<dyn Storage>>,
    pub assets_dir: Option<PathBuf>,
    pub inference: Arc<crate::Inference>,
    pub limits: PluginLimits,
    pub tag_namespaces: Vec<String>,
    pub events: Option<noflex_core::DomainEventBus>,
}

impl PluginCtxParts {
    pub fn build(&self) -> PluginCtx {
        PluginCtx::with_storage_and_secrets(
            self.plugin_id.clone(),
            self.config_toml.clone(),
            self.declared_secrets.clone(),
            self.declared_user_secrets.clone(),
            self.declared_user_secrets_write.clone(),
            self.declared_capabilities.clone(),
            self.storage.clone(),
        )
        .with_declared_tasks(self.declared_tasks.clone())
        .with_assets_dir(self.assets_dir.clone())
        .with_inference(self.inference.clone())
        .with_egress_hosts(self.http_egress_hosts.clone())
        .with_limits(self.limits.clone())
        .with_tag_namespaces(self.tag_namespaces.clone())
        .with_events(self.events.clone())
    }
}

impl PluginCtx {
    pub fn new(plugin_id: PluginId, config_toml: String) -> Self {
        Self::with_secret_allowlist(plugin_id, config_toml, Vec::new())
    }

    pub fn with_secret_allowlist(
        plugin_id: PluginId,
        config_toml: String,
        declared_secrets: Vec<String>,
    ) -> Self {
        Self::with_storage_and_secrets(
            plugin_id,
            config_toml,
            declared_secrets,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
    }

    pub fn with_storage_and_secrets(
        plugin_id: PluginId,
        config_toml: String,
        declared_secrets: Vec<String>,
        declared_user_secrets: Vec<String>,
        declared_user_secrets_write: Vec<String>,
        declared_capabilities: Vec<String>,
        storage: Option<Arc<dyn Storage>>,
    ) -> Self {
        // wasi-libc's startup (`__wasilibc_initialize_environ` and friends)
        // calls into `wasi:cli/environment.get-environment` etc. as part of
        // global-ctor execution. Inherit stdio + an empty env / args so
        // those calls succeed; missing pieces here trap the guest inside
        // `_rt::run_ctors_once` before any user code runs.
        let wasi = WasiCtxBuilder::new()
            .inherit_stdout()
            .inherit_stderr()
            .build();
        let limits = PluginLimitState::new(plugin_id.0.clone(), PluginLimits::default());
        Self {
            plugin_id,
            config_toml,
            declared_secrets,
            declared_user_secrets,
            declared_user_secrets_write,
            declared_capabilities,
            declared_tasks: Vec::new(),
            http_egress_hosts: Vec::new(),
            storage,
            assets_dir: None,
            inference: Arc::default(),
            limits,
            calls: CallTable::default(),
            tag_namespaces: Vec::new(),
            events: None,
            wasi,
            table: ResourceTable::new(),
        }
    }

    pub fn with_declared_tasks(mut self, tasks: Vec<String>) -> Self {
        self.declared_tasks = tasks;
        self
    }

    pub fn with_assets_dir(mut self, assets_dir: Option<PathBuf>) -> Self {
        self.assets_dir = assets_dir;
        self
    }

    pub fn with_inference(mut self, inference: Arc<crate::Inference>) -> Self {
        self.inference = inference;
        self
    }

    pub fn with_egress_hosts(mut self, hosts: Vec<String>) -> Self {
        self.http_egress_hosts = hosts;
        self
    }

    pub fn with_limits(mut self, limits: PluginLimits) -> Self {
        self.limits = PluginLimitState::new(self.plugin_id.0.clone(), limits);
        self
    }

    pub fn with_tag_namespaces(mut self, namespaces: Vec<String>) -> Self {
        self.tag_namespaces = namespaces;
        self
    }

    pub fn with_events(mut self, events: Option<noflex_core::DomainEventBus>) -> Self {
        self.events = events;
        self
    }
}

impl WasiView for PluginCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// ----------------------------------------------------------------------
// `host-log`: bridge into the host's tracing subscriber. All severity
// variants route to `noflex_plugin::<plugin_id>::<namespace>` except for
// `audit` which always lands on the dedicated audit target so it stays
// grep-able when operational logs are filtered down.
// ----------------------------------------------------------------------

impl host_log::Host for PluginCtx {
    // `tracing` macros require a const `target:` literal, so dynamic
    // per-plugin namespacing happens via fields instead. Operators
    // still filter on the static target `noflex_plugin` and on the
    // `plugin` / `ns` fields via `RUST_LOG`.
    fn log(
        &mut self,
        level: host_log::Level,
        namespace: String,
        message: String,
        fields: Vec<(String, String)>,
    ) -> wasmtime::Result<()> {
        let fields_dbg = format!("{fields:?}");
        match level {
            host_log::Level::Audit => tracing::info!(
                target: "noflex_plugin::audit",
                plugin = %self.plugin_id,
                ns = %namespace,
                %message,
                fields = %fields_dbg,
            ),
            host_log::Level::Error => tracing::error!(
                target: "noflex_plugin",
                plugin = %self.plugin_id,
                ns = %namespace,
                %message,
                fields = %fields_dbg,
            ),
            host_log::Level::Warn => tracing::warn!(
                target: "noflex_plugin",
                plugin = %self.plugin_id,
                ns = %namespace,
                %message,
                fields = %fields_dbg,
            ),
            host_log::Level::Info => tracing::info!(
                target: "noflex_plugin",
                plugin = %self.plugin_id,
                ns = %namespace,
                %message,
                fields = %fields_dbg,
            ),
            host_log::Level::Debug => tracing::debug!(
                target: "noflex_plugin",
                plugin = %self.plugin_id,
                ns = %namespace,
                %message,
                fields = %fields_dbg,
            ),
            host_log::Level::Trace => tracing::trace!(
                target: "noflex_plugin",
                plugin = %self.plugin_id,
                ns = %namespace,
                %message,
                fields = %fields_dbg,
            ),
        };
        Ok(())
    }
}

// ----------------------------------------------------------------------
// `host-secrets`: enforce the manifest allowlist, then take an administrator's
// value from storage, else search the configured secret tiers. Undeclared and
// missing keys remain distinguishable to the guest.
// ----------------------------------------------------------------------

// The async `HostWithStore` impls below carry the real logic; the `Host`
// marker traits for the WIT-async interfaces stay empty (the split exists
// so non-async interfaces keep `&mut self` methods while async ones take
// the store accessor instead).
impl host_secrets::Host for PluginCtx {}

impl host_secrets::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn read(
        accessor: &Accessor<PluginCtx, Self>,
        key: String,
    ) -> wasmtime::Result<Result<String, types::PluginError>> {
        let (plugin_id, capabilities, declared_secrets, storage) = accessor.with(|mut access| {
            let ctx = access.get();
            (
                ctx.plugin_id.clone(),
                ctx.declared_capabilities.clone(),
                ctx.declared_secrets.clone(),
                ctx.storage.clone(),
            )
        });
        if !valid_secret_key(&key) {
            crate::audit::secret_read(&plugin_id.0, &key, false);
            return Ok(Err(types::PluginError::BadInput(
                "secret key is empty or too long".into(),
            )));
        }
        if !has_capability(&capabilities, "secrets.read") {
            crate::audit::secret_read(&plugin_id.0, &key, false);
            return Ok(Err(types::PluginError::CapabilityDenied(
                "host-secrets.read requires the `secrets.read` capability".into(),
            )));
        }
        if !declared_secrets.iter().any(|k| k == &key) {
            crate::audit::secret_read(&plugin_id.0, &key, false);
            return Ok(Err(types::PluginError::CapabilityDenied(format!(
                "secret `{}` not declared in plugin manifest",
                key
            ))));
        }
        // An administrator's value from the admin UI wins over the files.
        let admin_set = match &storage {
            Some(storage) => match storage.plugin_secret_setting(&plugin_id.0, &key).await {
                Ok(value) => value,
                Err(e) => {
                    crate::audit::secret_read(&plugin_id.0, &key, false);
                    return Ok(Err(types::PluginError::Internal(format!(
                        "read secret setting: {e}"
                    ))));
                }
            },
            None => None,
        };
        let found = admin_set.or_else(|| {
            crate::secrets::SecretStore::with_default_tiers().read(
                &plugin_id,
                &key,
                &declared_secrets,
            )
        });
        crate::audit::secret_read(&plugin_id.0, &key, found.is_some());
        match found {
            Some(value) if value.len() <= MAX_SECRET_VALUE_BYTES => Ok(Ok(value)),
            Some(_) => Ok(Err(types::PluginError::Internal(format!(
                "secret `{key}` exceeds the host value limit"
            )))),
            None => Ok(Err(types::PluginError::Unavailable(format!(
                "secret `{key}` not configured"
            )))),
        }
    }

    async fn read_for_user(
        accessor: &Accessor<PluginCtx, Self>,
        user: String,
        key: String,
    ) -> wasmtime::Result<Result<String, types::PluginError>> {
        let (plugin_id, capabilities, declared_user_secrets, storage, principal) =
            accessor.with(|mut access| {
                let principal = current_principal(&mut access);
                let ctx = access.get();
                (
                    ctx.plugin_id.clone(),
                    ctx.declared_capabilities.clone(),
                    ctx.declared_user_secrets.clone(),
                    ctx.storage.clone(),
                    principal,
                )
            });
        if !valid_secret_key(&key) {
            crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
            return Ok(Err(types::PluginError::BadInput(
                "secret key is empty or too long".into(),
            )));
        }
        if !has_capability(&capabilities, "user-secrets.read") {
            crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
            return Ok(Err(types::PluginError::CapabilityDenied(
                "host-secrets.read-for-user requires the `user-secrets.read` capability".into(),
            )));
        }
        if !declared_user_secrets.iter().any(|k| k == &key) {
            crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
            return Ok(Err(types::PluginError::CapabilityDenied(format!(
                "user secret `{key}` not declared in plugin manifest"
            ))));
        }
        let Ok(user_uuid) = user.parse::<uuid::Uuid>() else {
            crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
            return Ok(Err(types::PluginError::BadInput(format!(
                "invalid user id `{user}`"
            ))));
        };
        let requested_user = noflex_core::UserId::from_uuid(user_uuid);
        let Some(principal) = principal else {
            crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
            return Ok(Err(types::PluginError::Unavailable(
                "per-user secrets require a user-bound invocation".into(),
            )));
        };
        if principal != requested_user {
            crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
            return Ok(Err(types::PluginError::CapabilityDenied(
                "requested user does not match the invocation principal".into(),
            )));
        }
        let Some(storage) = storage else {
            crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
            return Ok(Err(types::PluginError::Unavailable(
                "user secrets not configured on this host".into(),
            )));
        };
        let found = storage
            .user_plugin_secret(requested_user, &plugin_id.0, &key)
            .await;
        match found {
            Ok(Some(secret)) if secret.value.len() <= MAX_SECRET_VALUE_BYTES => {
                crate::audit::user_secret_read(&plugin_id.0, &user, &key, true);
                Ok(Ok(secret.value))
            }
            Ok(Some(_)) => {
                crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
                Ok(Err(types::PluginError::Internal(format!(
                    "secret `{key}` exceeds the host value limit"
                ))))
            }
            Ok(None) => {
                crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
                Ok(Err(types::PluginError::Unavailable(format!(
                    "secret `{key}` not configured for user"
                ))))
            }
            Err(e) => {
                crate::audit::user_secret_read(&plugin_id.0, &user, &key, false);
                Ok(Err(types::PluginError::Internal(format!(
                    "read user secret: {e}"
                ))))
            }
        }
    }

    async fn write_for_user(
        accessor: &Accessor<PluginCtx, Self>,
        user: String,
        key: String,
        value: String,
    ) -> wasmtime::Result<Result<(), types::PluginError>> {
        let (plugin_id, capabilities, declared_user_secrets_write, storage, principal) = accessor
            .with(|mut access| {
                let principal = current_principal(&mut access);
                let ctx = access.get();
                (
                    ctx.plugin_id.clone(),
                    ctx.declared_capabilities.clone(),
                    ctx.declared_user_secrets_write.clone(),
                    ctx.storage.clone(),
                    principal,
                )
            });
        if !valid_secret_key(&key) || value.len() > MAX_SECRET_VALUE_BYTES {
            return Ok(Err(types::PluginError::BadInput(
                "secret key or value exceeds host limits".into(),
            )));
        }
        if !has_capability(&capabilities, "user-secrets.write") {
            return Ok(Err(types::PluginError::CapabilityDenied(
                "host-secrets.write-for-user requires the `user-secrets.write` capability".into(),
            )));
        }
        if !declared_user_secrets_write.iter().any(|k| k == &key) {
            return Ok(Err(types::PluginError::CapabilityDenied(format!(
                "user secret `{key}` not declared writable in plugin manifest"
            ))));
        }
        let Ok(user_uuid) = user.parse::<uuid::Uuid>() else {
            return Ok(Err(types::PluginError::BadInput(format!(
                "invalid user id `{user}`"
            ))));
        };
        let requested_user = noflex_core::UserId::from_uuid(user_uuid);
        let Some(principal) = principal else {
            return Ok(Err(types::PluginError::Unavailable(
                "per-user secrets require a user-bound invocation".into(),
            )));
        };
        if principal != requested_user {
            return Ok(Err(types::PluginError::CapabilityDenied(
                "requested user does not match the invocation principal".into(),
            )));
        }
        let Some(storage) = storage else {
            return Ok(Err(types::PluginError::Unavailable(
                "user secrets not configured on this host".into(),
            )));
        };
        // The audit helper has no granted/denied flag (unlike the read
        // helper), so only a persisted write is logged — a denied attempt
        // surfaces to the guest as capability-denied, not as a write.
        match storage
            .set_user_plugin_secret(requested_user, &plugin_id.0, &key, &value)
            .await
        {
            Ok(()) => {
                crate::audit::user_secret_write(&plugin_id.0, &user, &key);
                Ok(Ok(()))
            }
            Err(e) => Ok(Err(types::PluginError::Internal(format!(
                "write user secret: {e}"
            )))),
        }
    }
}

// host-config and host-library are trappable too — they all return wasmtime::Result.

// ----------------------------------------------------------------------
// `host-config`: returns the plugin's config stanza. v1 returns the TOML
// blob handed in at construction time (which may be empty); v2 wires this
// to the running server config.
// ----------------------------------------------------------------------

impl host_config::Host for PluginCtx {
    fn read(&mut self) -> wasmtime::Result<String> {
        Ok(self.config_toml.clone())
    }
}

// ----------------------------------------------------------------------
// `host-library`: narrow read-only DB access. `lookup-work-by-id` is
// wired to `Storage::get_work`.
// ----------------------------------------------------------------------

impl host_library::Host for PluginCtx {}

impl host_library::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn lookup_work_by_id(
        accessor: &Accessor<PluginCtx, Self>,
        id: String,
    ) -> wasmtime::Result<Option<host_library::WorkSummary>> {
        // Clone the storage handle out of the store data so the async
        // storage call can run while the guest task (and this host call)
        // is suspended — the WIT-async import means no executor blocking.
        let (storage, plugin_id) = accessor.with(|mut access| {
            let ctx = access.get();
            (ctx.storage.clone(), ctx.plugin_id.clone())
        });
        let parsed_uuid: Result<uuid::Uuid, _> = id.parse();
        let hit = match (parsed_uuid, storage) {
            (Ok(u), Some(storage)) => {
                let work_id = noflex_core::WorkId::from_uuid(u);
                storage.get_work(work_id).await.ok().flatten()
            }
            _ => None,
        };
        crate::audit::library_lookup(&plugin_id.0, "lookup_work_by_id", &id, hit.is_some());
        Ok(hit.map(work_summary))
    }

    async fn lookup_edition_by_id(
        accessor: &Accessor<PluginCtx, Self>,
        id: String,
    ) -> wasmtime::Result<Option<host_library::EditionSummary>> {
        // Same shape as lookup_work_by_id: an unparseable id or a host
        // without storage answers `none`, never an error — detection
        // inputs are best-effort by design.
        let (storage, plugin_id) = accessor.with(|mut access| {
            let ctx = access.get();
            (ctx.storage.clone(), ctx.plugin_id.clone())
        });
        let parsed_uuid: Result<uuid::Uuid, _> = id.parse();
        let hit = match (parsed_uuid, storage) {
            (Ok(u), Some(storage)) => {
                let edition_id = noflex_core::EditionId::from_uuid(u);
                storage.get_edition(edition_id).await.ok().flatten()
            }
            _ => None,
        };
        crate::audit::library_lookup(&plugin_id.0, "lookup_edition_by_id", &id, hit.is_some());
        Ok(hit.map(|e| host_library::EditionSummary {
            id: e.id.to_string(),
            work_id: e.work_id.to_string(),
            name: e.name,
            runtime_ms: e.runtime_ms,
            chapters: e
                .chapters
                .into_iter()
                .map(|c| host_library::Chapter {
                    position_ms: c.position_ms,
                    title: c.title,
                })
                .collect(),
            intro_start_ms: e.intro_start_ms,
            intro_end_ms: e.intro_end_ms,
            credits_start_ms: e.credits_start_ms,
        }))
    }

    async fn find_works(
        accessor: &Accessor<PluginCtx, Self>,
        id: types::ExternalId,
    ) -> wasmtime::Result<Vec<host_library::WorkSummary>> {
        let (storage, plugin_id) = accessor.with(|mut access| {
            let ctx = access.get();
            (ctx.storage.clone(), ctx.plugin_id.clone())
        });
        let kind = crate::registry::from_wit_kind_id(id.kind);
        let works = match storage {
            Some(storage) => works_by_id(storage.as_ref(), kind, &id.value).await,
            None => Vec::new(),
        };
        crate::audit::library_lookup(
            &plugin_id.0,
            "find_works",
            &format!("{kind:?}:{}", id.value),
            !works.is_empty(),
        );
        Ok(works.into_iter().map(work_summary).collect())
    }

    async fn find_episode(
        accessor: &Accessor<PluginCtx, Self>,
        series_ids: Vec<types::ExternalId>,
        season: u32,
        episode: u32,
    ) -> wasmtime::Result<Option<host_library::WorkSummary>> {
        let (storage, plugin_id) = accessor.with(|mut access| {
            let ctx = access.get();
            (ctx.storage.clone(), ctx.plugin_id.clone())
        });
        let mut hit = None;
        if let Some(storage) = storage {
            'ids: for id in series_ids.iter().take(MAX_SERIES_IDS) {
                let kind = crate::registry::from_wit_kind_id(id.kind);
                for series in works_by_id(storage.as_ref(), kind, &id.value).await {
                    if series.kind != noflex_core::model::WorkKind::Series {
                        continue;
                    }
                    match storage.episodes_numbered(series.id, season, episode).await {
                        Ok(episodes) => {
                            if let Some(found) = episodes.into_iter().next() {
                                hit = Some(found);
                                break 'ids;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(plugin = %plugin_id, error = %e, "find-episode: lookup failed");
                        }
                    }
                }
            }
        }
        crate::audit::library_lookup(
            &plugin_id.0,
            "find_episode",
            &format!("s{season}e{episode}"),
            hit.is_some(),
        );
        Ok(hit.map(work_summary))
    }
}

/// Most series ids one `find-episode` tries; a show has a handful.
const MAX_SERIES_IDS: usize = 16;

/// The works carrying `(kind, value)`; a storage failure is logged and
/// answers none, like the other lookups.
async fn works_by_id(
    storage: &dyn Storage,
    kind: noflex_core::model::ExternalIdKind,
    value: &str,
) -> Vec<noflex_storage::domain::Work> {
    match storage.works_by_external_id(kind, value).await {
        Ok(works) => works,
        Err(e) => {
            tracing::warn!(error = %e, "host-library: external id lookup failed");
            Vec::new()
        }
    }
}

fn work_summary(w: noflex_storage::domain::Work) -> host_library::WorkSummary {
    host_library::WorkSummary {
        id: w.id.to_string(),
        kind: work_kind_to_wit(w.kind),
        title: w.title,
        year: w.year,
    }
}

fn work_kind_to_wit(k: noflex_core::model::WorkKind) -> host_library::WorkKind {
    use noflex_core::model::WorkKind;
    use types::WorkKind as Wit;
    match k {
        WorkKind::Movie => Wit::Movie,
        WorkKind::Series => Wit::Series,
        WorkKind::Season => Wit::Season,
        WorkKind::Episode => Wit::Episode,
        WorkKind::Album => Wit::Album,
        WorkKind::Track => Wit::Track,
        WorkKind::Artist => Wit::Artist,
        WorkKind::AudioBook => Wit::AudioBook,
        WorkKind::Photo => Wit::Photo,
    }
}

// ----------------------------------------------------------------------
// `host-tasks`: dynamic follow-up work. Enqueues persist host-side and
// drain on the plugin's instance FIFO per task; the guest only learns
// the payload when `run-task` is invoked with it.
// ----------------------------------------------------------------------

impl host_tasks::Host for PluginCtx {}

impl host_tasks::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn enqueue(
        accessor: &Accessor<PluginCtx, Self>,
        task: String,
        payload: Option<String>,
    ) -> wasmtime::Result<Result<(), types::PluginError>> {
        // A task enqueued by a user-bound call runs bound to that user, so
        // device-flow style links can finish in the background.
        let (plugin_id, capabilities, declared_tasks, storage, principal) =
            accessor.with(|mut access| {
                let principal = current_principal(&mut access);
                let ctx = access.get();
                (
                    ctx.plugin_id.clone(),
                    ctx.declared_capabilities.clone(),
                    ctx.declared_tasks.clone(),
                    ctx.storage.clone(),
                    principal,
                )
            });
        if !crate::manifest::declares_background(&capabilities) {
            crate::audit::task_enqueue(&plugin_id.0, &task, false);
            return Ok(Err(types::PluginError::CapabilityDenied(
                "host-tasks.enqueue requires the `background` capability".into(),
            )));
        }
        if !declared_tasks.iter().any(|declared| declared == &task) {
            crate::audit::task_enqueue(&plugin_id.0, &task, false);
            return Ok(Err(types::PluginError::BadInput(format!(
                "task `{task}` is not registered in the plugin manifest"
            ))));
        }
        if payload
            .as_ref()
            .is_some_and(|payload| payload.len() > MAX_TASK_PAYLOAD_BYTES)
        {
            crate::audit::task_enqueue(&plugin_id.0, &task, false);
            return Ok(Err(types::PluginError::BadInput(format!(
                "task payload exceeds the {} KiB cap",
                MAX_TASK_PAYLOAD_BYTES / 1024
            ))));
        }
        let Some(storage) = storage else {
            crate::audit::task_enqueue(&plugin_id.0, &task, false);
            return Ok(Err(types::PluginError::Unavailable(
                "task queue not configured on this host".into(),
            )));
        };
        let stored = crate::scheduler::encode_queued(principal, payload);
        let result = storage
            .enqueue_plugin_task(&plugin_id.0, &task, stored.as_deref())
            .await;
        crate::audit::task_enqueue(&plugin_id.0, &task, result.is_ok());
        match result {
            Ok(()) => Ok(Ok(())),
            Err(e) => Ok(Err(types::PluginError::Internal(format!(
                "persist task enqueue: {e}"
            )))),
        }
    }
}

// ----------------------------------------------------------------------
// `host-assets`: read-only files from the host-configured assets
// directory (embedding model weights, tokenizers, ...). Gated on the
// `assets.read` capability; every attempt is audit-logged. Only the
// `embedder-plugin` and umbrella worlds import this interface.
// ----------------------------------------------------------------------

impl host_assets::Host for PluginCtx {}

impl host_assets::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn read_file(
        accessor: &Accessor<PluginCtx, Self>,
        name: String,
    ) -> wasmtime::Result<Result<Vec<u8>, embedder_types::PluginError>> {
        use embedder_types::PluginError;

        let (plugin_id, capabilities, assets_dir) = accessor.with(|mut access| {
            let ctx = access.get();
            (
                ctx.plugin_id.clone(),
                ctx.declared_capabilities.clone(),
                ctx.assets_dir.clone(),
            )
        });
        let deny = |e: PluginError| {
            crate::audit::asset_read(&plugin_id.0, &name, None);
            Ok(Err(e))
        };
        if !capabilities.iter().any(|c| c == "assets.read") {
            return deny(PluginError::CapabilityDenied(
                "host-assets.read-file requires the `assets.read` capability".into(),
            ));
        }
        let Some(root) = assets_dir else {
            return deny(PluginError::Unavailable(
                "asset store not configured on this host".into(),
            ));
        };
        let path = match resolve_asset_path(&root, &name).await {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return deny(PluginError::NotFound);
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return deny(PluginError::CapabilityDenied(format!(
                    "asset `{name}` escapes the configured asset directory"
                )));
            }
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                return deny(PluginError::BadInput(format!(
                    "invalid asset name `{name}`"
                )));
            }
            Err(e) => {
                return deny(PluginError::Internal(format!(
                    "resolve asset `{name}`: {e}"
                )));
            }
        };
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.len() > MAX_ASSET_BYTES => {
                return deny(PluginError::BadInput(format!(
                    "asset `{name}` exceeds the 512 MiB size cap"
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return deny(PluginError::NotFound);
            }
            Err(e) => {
                return deny(PluginError::Internal(format!("stat asset `{name}`: {e}")));
            }
        }
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return deny(PluginError::NotFound);
            }
            Err(e) => {
                return deny(PluginError::Internal(format!("read asset `{name}`: {e}")));
            }
        };
        crate::audit::asset_read(&plugin_id.0, &name, Some(bytes.len() as u64));
        Ok(Ok(bytes))
    }
}

// ----------------------------------------------------------------------
// `host-inference`: native model runs over a model in the assets directory. Gated
// on the `inference.run` capability; every attempt is audit-logged.
// ----------------------------------------------------------------------

impl host_inference::Host for PluginCtx {}

impl host_inference::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn embed_tokens(
        accessor: &Accessor<PluginCtx, Self>,
        model: String,
        tokens: Vec<Vec<u32>>,
    ) -> wasmtime::Result<Result<Vec<Vec<f32>>, embedder_types::PluginError>> {
        use embedder_types::PluginError;

        let (plugin_id, capabilities, assets_dir, inference) = accessor.with(|mut access| {
            let ctx = access.get();
            (
                ctx.plugin_id.clone(),
                ctx.declared_capabilities.clone(),
                ctx.assets_dir.clone(),
                ctx.inference.clone(),
            )
        });
        let sequences = tokens.len() as u64;
        let finish = |result: Result<Vec<Vec<f32>>, PluginError>| {
            crate::audit::inference_run(&plugin_id.0, &model, sequences, result.is_ok());
            Ok(result)
        };
        if !has_capability(&capabilities, "inference.run") {
            return finish(Err(PluginError::CapabilityDenied(
                "host-inference requires the `inference.run` capability".into(),
            )));
        }
        let Some(root) = assets_dir else {
            return finish(Err(PluginError::Unavailable(
                "asset store not configured on this host".into(),
            )));
        };
        let path = match resolve_asset_path(&root, &model).await {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return finish(Err(PluginError::NotFound));
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return finish(Err(PluginError::CapabilityDenied(format!(
                    "model `{model}` escapes the configured asset directory"
                ))));
            }
            Err(e) => {
                return finish(Err(PluginError::BadInput(format!(
                    "invalid model `{model}`: {e}"
                ))));
            }
        };
        finish(
            inference
                .embed_tokens(&path, tokens)
                .await
                .map_err(|e| PluginError::Unavailable(e.to_string())),
        )
    }
}

async fn resolve_asset_path(root: &Path, name: &str) -> std::io::Result<PathBuf> {
    if name.is_empty() || name.contains('\\') {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let mut lexical = root.to_path_buf();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => lexical.push(part),
            _ => return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput)),
        }
    }
    let canonical_root = tokio::fs::canonicalize(root).await?;
    let canonical_path = tokio::fs::canonicalize(lexical).await?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    Ok(canonical_path)
}

// ----------------------------------------------------------------------
// `host-metrics`: emit into the host's tracing subscriber for now.
// A real exporter (Prometheus / OTLP) wires in via a metrics module
// added to noflex-server in a follow-up.
// ----------------------------------------------------------------------

impl host_metrics::Host for PluginCtx {
    fn histogram(&mut self, name: String, value: f64) -> wasmtime::Result<()> {
        tracing::debug!(
            target: "noflex_plugin::metrics",
            plugin = %self.plugin_id,
            kind = "histogram",
            metric = %name,
            value,
        );
        Ok(())
    }

    fn counter(&mut self, name: String, delta: u64) -> wasmtime::Result<()> {
        tracing::debug!(
            target: "noflex_plugin::metrics",
            plugin = %self.plugin_id,
            kind = "counter",
            metric = %name,
            delta,
        );
        Ok(())
    }
}

// `types` is import-imported because the world's interfaces `use` it.
// The interface has no functions, only type definitions, so the host
// trait impl is empty.
impl crate::bindings::metadata_only::noflex::plugin::types::Host for PluginCtx {}

/// Build a `wasmtime::component::Linker` with every NoFlex host import
/// wired up. The linker is shared across plugins; each plugin instance
/// gets its own `Store<PluginCtx>` so audit events carry the right
/// `plugin_id`.
pub fn build_linker(
    engine: &wasmtime::Engine,
) -> wasmtime::Result<wasmtime::component::Linker<PluginCtx>> {
    let mut linker = wasmtime::component::Linker::<PluginCtx>::new(engine);
    // WASI 0.2 async implementations — Rust's `wasm32-wasip2` std imports
    // `wasi:cli/*@0.2.0` and friends for runtime support. The async linker
    // variants let those imports run inside an async store; the plugin's
    // own async ABI (streams, concurrent calls) is canonical-ABI level and
    // needs no wasi:io-style imports at all.
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    // WASI 0.3 clocks: their async `wait-for` / `wait-until` let a guest
    // sleep (e.g. to pace upstream requests) without blocking the
    // instance's other calls. Grants nothing beyond what 0.2 clocks do.
    wasmtime_wasi::p3::clocks::add_to_linker(&mut linker)?;
    MetadataPlugin::add_to_linker::<_, wasmtime::component::HasSelf<PluginCtx>>(
        &mut linker,
        |ctx| ctx,
    )?;
    // `host-assets` is only imported by the embedder + umbrella worlds,
    // so the metadata-plugin world's `add_to_linker` doesn't cover it.
    // `host-net` needs no separate registration: every world imports it,
    // so the metadata-plugin `add_to_linker` above already wires it.
    host_assets::add_to_linker::<_, wasmtime::component::HasSelf<PluginCtx>>(&mut linker, |ctx| {
        ctx
    })?;
    host_inference::add_to_linker::<_, wasmtime::component::HasSelf<PluginCtx>>(
        &mut linker,
        |ctx| ctx,
    )?;
    crate::library_writes::add_to_linker(&mut linker)?;
    Ok(linker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_asset_path_accepts_relative_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("models");
        tokio::fs::create_dir_all(root.join("potion"))
            .await
            .unwrap();
        tokio::fs::write(root.join("potion/base.bin"), b"model")
            .await
            .unwrap();
        assert_eq!(
            resolve_asset_path(&root, "potion/base.bin").await.unwrap(),
            root.canonicalize().unwrap().join("potion/base.bin")
        );
    }

    #[tokio::test]
    async fn resolve_asset_path_rejects_lexical_and_symlink_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("models");
        tokio::fs::create_dir(&root).await.unwrap();
        assert!(resolve_asset_path(&root, "../secret").await.is_err());
        assert!(resolve_asset_path(&root, "./model.bin").await.is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let outside = dir.path().join("secret");
            std::fs::write(&outside, b"secret").unwrap();
            symlink(&outside, root.join("final-link")).unwrap();
            assert_eq!(
                resolve_asset_path(&root, "final-link")
                    .await
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );

            let outside_dir = dir.path().join("outside");
            std::fs::create_dir(&outside_dir).unwrap();
            std::fs::write(outside_dir.join("nested"), b"secret").unwrap();
            symlink(&outside_dir, root.join("dir-link")).unwrap();
            assert_eq!(
                resolve_asset_path(&root, "dir-link/nested")
                    .await
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
    }

    #[test]
    fn plugin_ctx_carries_storage_when_provided() {
        let ctx = PluginCtx::with_storage_and_secrets(
            PluginId("tmdb".to_string()),
            String::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        );
        assert!(ctx.storage.is_none());
    }

    #[test]
    fn work_kind_round_trip() {
        for k in [
            noflex_core::model::WorkKind::Movie,
            noflex_core::model::WorkKind::Series,
            noflex_core::model::WorkKind::Season,
            noflex_core::model::WorkKind::Episode,
            noflex_core::model::WorkKind::Album,
            noflex_core::model::WorkKind::Track,
            noflex_core::model::WorkKind::Artist,
            noflex_core::model::WorkKind::AudioBook,
            noflex_core::model::WorkKind::Photo,
        ] {
            let _ = work_kind_to_wit(k);
        }
    }
}
