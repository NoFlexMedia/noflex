// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::{Sink as _, StreamExt as _};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};
use wasmtime::{
    Engine, Store, StoreContextMut, UpdateDeadline,
    component::{
        Accessor, Component, ComponentNamedList, Lift, Linker, Lower, Source, StreamConsumer,
        StreamReader, StreamResult, TypedFunc,
    },
};

use crate::{
    EmbedInput, EngineConfig, HookKind, HostError, PluginCtx, PluginLimits, PluginManifest,
    bindings::{
        embedder::{
            EmbedderPlugin as EmbedderPluginBinding, noflex::plugin::types as embedder_types,
        },
        external_auth::{
            ExternalAuthPlugin, exports::noflex::plugin::external_auth as external_auth_exports,
            noflex::plugin::types as external_auth_types,
        },
        intro_detector::{
            IntroDetectorPlugin,
            exports::noflex::plugin::intro_detector::IntroDetection as WitIntroDetection,
            noflex::plugin::types as intro_detector_types,
        },
        metadata_and_image::MetadataImagePlugin as ImagePlugin,
        metadata_only::{
            MetadataPlugin,
            noflex::plugin::types::{
                ExternalId as WitExternalId, PluginError as WitPluginError,
                SearchQuery as WitSearchQuery, TaskOutcome as WitTaskOutcome,
                WorkKind as WitWorkKind,
            },
        },
        notifier::{NotifierPlugin, noflex::plugin::types as notifier_types},
        post_scan::{PostScanPlugin, noflex::plugin::types as post_scan_types},
        scrobbler::{ScrobblerPlugin, noflex::plugin::types as scrobbler_types},
        user_link::{
            UserLinkPlugin, exports::noflex::plugin::user_link as user_link_exports,
            noflex::plugin::types as user_link_types,
        },
    },
    dispatch::{CallTable, PluginCtxParts},
    manifest::{EmbeddingClaim, TaskRegistration},
    metadata_and_image::noflex::plugin::types as image_types,
    plugin_metadata::{MetadataMatch as WitMetadataMatch, MetadataPayload as WitMetadataPayload},
};

const CALL_QUEUE_CAPACITY: usize = 64;

/// Most export calls one instance runs at once. Past it the instance stops
/// draining its queue, so the bounded queue fills and callers see
/// [`HostError::Overloaded`] instead of an unbounded in-flight set.
const MAX_INFLIGHT_CALLS: usize = 64;
const IMAGE_STREAM_CHUNKS: usize = 8;
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

/// Consecutive traps that open the circuit breaker. While it is open the
/// instance task answers every call with the circuit error and waits for
/// the channel to close; the plugin stays down until it is reloaded
/// (hot-reload, admin action, or restart).
const MAX_CONSECUTIVE_TRAPS: u32 = 5;

/// Recycle backoff: doubles per consecutive trap, capped. A trap during
/// concurrent calls counts once per recycle, not once per failed call.
const RECYCLE_BACKOFF_INITIAL: Duration = Duration::from_millis(100);
const RECYCLE_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Stable, plugin-author-chosen identifier (from manifest).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId(pub String);

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for PluginId {
    fn from(s: String) -> Self {
        PluginId(s)
    }
}

/// `call_search`'s wire result: dispatch error outside, plugin error inside.
type SearchOutcome = Result<Result<Vec<WitMetadataMatch>, WitPluginError>, HostError>;

/// `call_supports`'s wire result.
type SupportsOutcome = Result<bool, HostError>;

/// `call_fetch_image`'s wire result: dispatch error outside, plugin error inside.
type FetchImageOutcome =
    Result<Result<(image_types::ImageMeta, Vec<u8>), image_types::PluginError>, HostError>;

/// `call_fetch`'s wire result: dispatch error outside, plugin error inside.
type FetchOutcome = Result<Result<WitMetadataPayload, WitPluginError>, HostError>;

/// `call_notify`'s wire result: dispatch error outside, plugin error inside.
type NotifyOutcome = Result<Result<(), notifier_types::PluginError>, HostError>;

/// `call_post_scan`'s wire result: dispatch error outside, plugin error inside.
type PostScanOutcome = Result<Result<(), post_scan_types::PluginError>, HostError>;

/// `call_link_start`'s wire result: dispatch error outside, plugin error inside.
type LinkStartOutcome =
    Result<Result<user_link_exports::LinkChallenge, user_link_types::PluginError>, HostError>;

/// `call_link_complete`'s wire result: dispatch error outside, plugin error inside.
type LinkCompleteOutcome =
    Result<Result<Vec<user_link_exports::UserSecret>, user_link_types::PluginError>, HostError>;
type ExternalAuthStartOutcome = Result<Result<String, external_auth_types::PluginError>, HostError>;
type ExternalAuthCompleteOutcome = Result<
    Result<external_auth_exports::VerifiedIdentity, external_auth_types::PluginError>,
    HostError,
>;

/// `call_embed`'s wire result: dispatch error outside, plugin error inside.
type EmbedOutcome = Result<Result<Vec<Vec<f32>>, embedder_types::PluginError>, HostError>;

/// `call_scrobble`'s wire result: dispatch error outside, plugin error inside.
type ScrobbleOutcome = Result<Result<(), scrobbler_types::PluginError>, HostError>;

/// `call_detect_intro`'s wire result: dispatch error outside, plugin error
/// inside; `None` means the detector found nothing worth marking.
type IntroDetectOutcome =
    Result<Result<Option<WitIntroDetection>, intro_detector_types::PluginError>, HostError>;

/// Host-side mirror of the WIT `task-outcome` variant: `done` retires the
/// invocation, `more` carries the guest-owned cursor and asks for a prompt
/// re-invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    Done,
    More(String),
}

impl From<WitTaskOutcome> for TaskOutcome {
    fn from(o: WitTaskOutcome) -> Self {
        match o {
            WitTaskOutcome::Done => TaskOutcome::Done,
            WitTaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

// Each bindgen module emits its own `TaskOutcome`; they are identical
// over the wire, so conversion is a mechanical re-tag.
impl From<image_types::TaskOutcome> for TaskOutcome {
    fn from(o: image_types::TaskOutcome) -> Self {
        match o {
            image_types::TaskOutcome::Done => TaskOutcome::Done,
            image_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

impl From<notifier_types::TaskOutcome> for TaskOutcome {
    fn from(o: notifier_types::TaskOutcome) -> Self {
        match o {
            notifier_types::TaskOutcome::Done => TaskOutcome::Done,
            notifier_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

impl From<user_link_types::TaskOutcome> for TaskOutcome {
    fn from(o: user_link_types::TaskOutcome) -> Self {
        match o {
            user_link_types::TaskOutcome::Done => TaskOutcome::Done,
            user_link_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

impl From<embedder_types::TaskOutcome> for TaskOutcome {
    fn from(o: embedder_types::TaskOutcome) -> Self {
        match o {
            embedder_types::TaskOutcome::Done => TaskOutcome::Done,
            embedder_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

impl From<post_scan_types::TaskOutcome> for TaskOutcome {
    fn from(o: post_scan_types::TaskOutcome) -> Self {
        match o {
            post_scan_types::TaskOutcome::Done => TaskOutcome::Done,
            post_scan_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

impl From<scrobbler_types::TaskOutcome> for TaskOutcome {
    fn from(o: scrobbler_types::TaskOutcome) -> Self {
        match o {
            scrobbler_types::TaskOutcome::Done => TaskOutcome::Done,
            scrobbler_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

impl From<external_auth_types::TaskOutcome> for TaskOutcome {
    fn from(o: external_auth_types::TaskOutcome) -> Self {
        match o {
            external_auth_types::TaskOutcome::Done => TaskOutcome::Done,
            external_auth_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

impl From<intro_detector_types::TaskOutcome> for TaskOutcome {
    fn from(o: intro_detector_types::TaskOutcome) -> Self {
        match o {
            intro_detector_types::TaskOutcome::Done => TaskOutcome::Done,
            intro_detector_types::TaskOutcome::More(cursor) => TaskOutcome::More(cursor),
        }
    }
}

/// `run_task_on` normalizes every world's `plugin-error` onto the
/// `metadata-plugin` binding's, so [`RunTaskOutcome`] stays one type
/// regardless of which world supplied the `background` export. Like
/// `TaskOutcome`, the per-module variants are identical over the wire.
macro_rules! impl_plugin_error_from {
    ($src:path) => {
        impl From<$src> for WitPluginError {
            fn from(e: $src) -> Self {
                use $src as Src;
                match e {
                    Src::Unavailable(m) => WitPluginError::Unavailable(m),
                    Src::BadInput(m) => WitPluginError::BadInput(m),
                    Src::NotFound => WitPluginError::NotFound,
                    Src::RateLimited(s) => WitPluginError::RateLimited(s),
                    Src::Internal(m) => WitPluginError::Internal(m),
                    Src::Trapped(m) => WitPluginError::Trapped(m),
                    Src::CapabilityDenied(m) => WitPluginError::CapabilityDenied(m),
                }
            }
        }
    };
}

impl_plugin_error_from!(image_types::PluginError);
impl_plugin_error_from!(notifier_types::PluginError);
impl_plugin_error_from!(post_scan_types::PluginError);
impl_plugin_error_from!(user_link_types::PluginError);
impl_plugin_error_from!(embedder_types::PluginError);
impl_plugin_error_from!(scrobbler_types::PluginError);
impl_plugin_error_from!(intro_detector_types::PluginError);
impl_plugin_error_from!(external_auth_types::PluginError);

/// `call_run_task`'s wire result: dispatch error outside, plugin error inside.
type RunTaskOutcome = Result<Result<TaskOutcome, WitPluginError>, HostError>;

/// Live health of one manifest-registered background task, updated by the
/// scheduler after every invocation. `last_run_at` is when the last
/// invocation *finished*, success or failure.
#[derive(Debug, Clone)]
pub struct TaskHealth {
    pub interval_ms: u64,
    pub last_run_at: Option<OffsetDateTime>,
    pub consecutive_failures: u32,
    pub total_runs: u64,
}

impl TaskHealth {
    fn new(reg: &TaskRegistration) -> Self {
        Self {
            interval_ms: reg.interval_ms,
            last_run_at: None,
            consecutive_failures: 0,
            total_runs: 0,
        }
    }
}

/// Mutable health shared between the instance task, the background-task
/// scheduler, and the registry's GraphQL snapshots. One `RwLock` per
/// plugin; critical sections never cross an `.await`.
#[derive(Debug, Default)]
pub struct PluginHealth {
    /// Times the instance was torn down and re-instantiated after a trap.
    pub instance_restarts: u32,
    /// Circuit breaker: set after [`MAX_CONSECUTIVE_TRAPS`] traps in a
    /// row. The plugin stays down until reloaded.
    pub circuit_open: bool,
    pub last_trap: Option<String>,
    /// Per-task health, keyed by manifest task name.
    pub tasks: BTreeMap<String, TaskHealth>,
}

impl PluginHealth {
    fn from_manifest(manifest: &PluginManifest) -> Self {
        Self {
            tasks: manifest
                .tasks
                .iter()
                .map(|t| (t.name.clone(), TaskHealth::new(t)))
                .collect(),
            ..Self::default()
        }
    }
}

/// One hook invocation handed to the plugin's instance task. The response
/// channel is owned by the caller; the task answers exactly once.
enum HookCall {
    Search {
        query: WitSearchQuery,
        respond: oneshot::Sender<SearchOutcome>,
    },
    Supports {
        kind: WitWorkKind,
        respond: oneshot::Sender<SupportsOutcome>,
    },
    FetchImage {
        id: image_types::ExternalId,
        kind: image_types::ImageKind,
        language: Option<String>,
        respond: oneshot::Sender<FetchImageOutcome>,
    },
    FetchMetadata {
        id: WitExternalId,
        respond: oneshot::Sender<FetchOutcome>,
    },
    RunTask {
        user: Option<noflex_core::UserId>,
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
        respond: oneshot::Sender<RunTaskOutcome>,
    },
    Notify {
        event: notifier_types::NotifyEvent,
        respond: oneshot::Sender<NotifyOutcome>,
    },
    PostScan {
        library: String,
        added: Vec<String>,
        updated: Vec<String>,
        removed: Vec<String>,
        respond: oneshot::Sender<PostScanOutcome>,
    },
    LinkStart {
        user_id: String,
        respond: oneshot::Sender<LinkStartOutcome>,
    },
    LinkComplete {
        user_id: String,
        state: String,
        callback_query: String,
        respond: oneshot::Sender<LinkCompleteOutcome>,
    },
    ExternalAuthStart {
        callback_uri: String,
        state: String,
        pkce_challenge: String,
        respond: oneshot::Sender<ExternalAuthStartOutcome>,
    },
    ExternalAuthComplete {
        callback_uri: String,
        code: String,
        pkce_verifier: String,
        respond: oneshot::Sender<ExternalAuthCompleteOutcome>,
    },
    Embed {
        inputs: Vec<EmbedInput>,
        respond: oneshot::Sender<EmbedOutcome>,
    },
    Scrobble {
        event: scrobbler_types::ScrobbleEvent,
        respond: oneshot::Sender<ScrobbleOutcome>,
    },
    IntroDetect {
        edition: String,
        respond: oneshot::Sender<IntroDetectOutcome>,
    },
}

impl HookCall {
    fn is_cancelled(&self) -> bool {
        match self {
            Self::Search { respond, .. } => respond.is_closed(),
            Self::Supports { respond, .. } => respond.is_closed(),
            Self::FetchImage { respond, .. } => respond.is_closed(),
            Self::FetchMetadata { respond, .. } => respond.is_closed(),
            Self::RunTask { respond, .. } => respond.is_closed(),
            Self::Notify { respond, .. } => respond.is_closed(),
            Self::PostScan { respond, .. } => respond.is_closed(),
            Self::LinkStart { respond, .. } => respond.is_closed(),
            Self::LinkComplete { respond, .. } => respond.is_closed(),
            Self::ExternalAuthStart { respond, .. } => respond.is_closed(),
            Self::ExternalAuthComplete { respond, .. } => respond.is_closed(),
            Self::Embed { respond, .. } => respond.is_closed(),
            Self::Scrobble { respond, .. } => respond.is_closed(),
            Self::IntroDetect { respond, .. } => respond.is_closed(),
        }
    }

    /// Answer the caller with a dispatch-level failure. Used when the
    /// instance never came up (instantiate trap) so callers see the real
    /// cause instead of a silently-closed channel.
    fn fail(self, msg: &str) {
        match self {
            HookCall::Search { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::Supports { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::FetchImage { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::FetchMetadata { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::RunTask { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::Notify { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::PostScan { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::LinkStart { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::LinkComplete { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::ExternalAuthStart { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::ExternalAuthComplete { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::Embed { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::Scrobble { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
            HookCall::IntroDetect { respond, .. } => {
                let _ = respond.send(Err(HostError::Wasm(msg.to_string())));
            }
        }
    }
}

/// The hook interfaces a component actually exports, bound at
/// instantiation. A plugin may export any subset; each field is `None`
/// when the component doesn't provide that interface. At least one must
/// be `Some` — a component with no recognized hook is an instantiate
/// failure.
struct BoundHooks {
    metadata: Option<MetadataPlugin>,
    image: Option<ImagePlugin>,
    notifier: Option<NotifierPlugin>,
    post_scan: Option<PostScanPlugin>,
    user_link: Option<UserLinkPlugin>,
    external_auth: Option<ExternalAuthPlugin>,
    embedder: Option<EmbedderPluginBinding>,
    scrobbler: Option<ScrobblerPlugin>,
    intro_detector: Option<IntroDetectorPlugin>,
}

impl BoundHooks {
    fn any_bound(&self) -> bool {
        self.metadata.is_some()
            || self.image.is_some()
            || self.notifier.is_some()
            || self.post_scan.is_some()
            || self.user_link.is_some()
            || self.external_auth.is_some()
            || self.embedder.is_some()
            || self.scrobbler.is_some()
            || self.intro_detector.is_some()
    }
}

/// Handle to the plugin's single long-lived instance. Dropping the last
/// handle closes the call channel, which lets the instance task drain
/// in-flight calls and shut the store down.
struct PluginInstance {
    calls: mpsc::Sender<HookCall>,
}

impl PluginInstance {
    fn enqueue(&self, id: &PluginId, call: HookCall) -> Result<(), HostError> {
        self.calls.try_send(call).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => HostError::Overloaded { id: id.0.clone() },
            mpsc::error::TrySendError::Closed(_) => {
                HostError::Wasm(format!("{id}: instance not running"))
            }
        })
    }
}

/// Stream consumer that funnels a guest-produced byte stream into a
/// channel as it arrives. EOF is observed by the receiving end as the
/// channel closing: when the guest drops its write end, the runtime
/// drops this consumer and with it the sender.
struct ByteCollector {
    tx: futures::channel::mpsc::Sender<Vec<u8>>,
}

impl StreamConsumer<PluginCtx> for ByteCollector {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        store: StoreContextMut<PluginCtx>,
        source: Source<'_, u8>,
        _finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let mut source = source.as_direct(store);
        let chunk = source.remaining();
        if chunk.is_empty() {
            // An empty source is a writability probe on the guest's behalf
            // (WebAssembly/component-model#561), not data; report progress
            // so the guest is re-driven when real bytes arrive.
            return Poll::Ready(Ok(StreamResult::Completed));
        }
        let this = self.get_mut();
        match Pin::new(&mut this.tx).poll_ready(cx) {
            Poll::Ready(Ok(())) => {
                let n = chunk.len();
                let bytes = chunk.to_vec();
                source.mark_read(n);
                match Pin::new(&mut this.tx).start_send(bytes) {
                    Ok(()) => Poll::Ready(Ok(StreamResult::Completed)),
                    Err(_) => Poll::Ready(Ok(StreamResult::Dropped)),
                }
            }
            Poll::Ready(Err(_)) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A loaded plugin — its WASM component plus the host's view of what it
/// exports. One instance services every hook call; callers reach it
/// through the async `call_*` methods.
pub struct LoadedPlugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub binary_sha256: String,
    pub exported_hooks: Vec<HookKind>,
    pub component: Component,
    /// Per-plugin config blob from the server's
    /// `[plugins.per_plugin_config.<id>]` table, serialized as TOML.
    /// Returned to the guest via `host-config::read`.
    pub config_toml: String,
    /// Live health: instance restarts, circuit breaker, per-task scheduler
    /// state. Shared with the instance task and the task scheduler; the
    /// registry reads it for GraphQL snapshots.
    pub health: Arc<RwLock<PluginHealth>>,
    instance: PluginInstance,
}

impl LoadedPlugin {
    /// Run the plugin's `metadata.search()` export on its single instance.
    ///
    /// The call is queued to the instance task, which runs it inside the
    /// store's `run_concurrent` scope — concurrent `call_search` invocations
    /// genuinely overlap in the guest. Returns the list of matches the
    /// plugin produced, or a plugin error.
    pub async fn call_search(&self, query: WitSearchQuery) -> SearchOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance
            .enqueue(&self.id, HookCall::Search { query, respond })?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Ask the plugin's `metadata.supports()` export whether it serves
    /// `kind`. Errors when the plugin exports no metadata hook or the
    /// call fails; callers treat that as "no".
    pub async fn call_supports(&self, kind: WitWorkKind) -> SupportsOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance
            .enqueue(&self.id, HookCall::Supports { kind, respond })?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `metadata.fetch()` export on its single instance.
    /// Queued and overlapped exactly like [`Self::call_search`]; the
    /// registry's provider-exclusive walk decides which candidate gets
    /// called, this is only the transport.
    pub async fn call_fetch(&self, id: WitExternalId) -> FetchOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance
            .enqueue(&self.id, HookCall::FetchMetadata { id, respond })?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `image.fetch()` export. The TMDb/TVDB plugins
    /// implement this alongside `metadata.search` so the host can grab
    /// posters + backdrops on demand.
    ///
    /// The guest returns the payload as a `stream<u8>`; it is read to
    /// completion here because the metadata pipeline above keeps `Vec<u8>`.
    pub async fn call_fetch_image(
        &self,
        external_id: image_types::ExternalId,
        kind: image_types::ImageKind,
        language: Option<String>,
    ) -> FetchImageOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance.enqueue(
            &self.id,
            HookCall::FetchImage {
                id: external_id,
                kind,
                language,
                respond,
            },
        )?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `background.run-task()` export with no user bound.
    /// Only the task scheduler (and tests) call this — foreground hook
    /// callers never see background invocations, though both overlap on
    /// the one instance.
    pub async fn call_run_task(
        &self,
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> RunTaskOutcome {
        self.call_run_task_as(None, task, cursor, payload).await
    }

    /// [`Self::call_run_task`] acting for `user`: per-user host imports
    /// inside the invocation are granted for that user only. The scheduler
    /// binds the user whose call enqueued the payload.
    pub(crate) async fn call_run_task_as(
        &self,
        user: Option<noflex_core::UserId>,
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> RunTaskOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance.enqueue(
            &self.id,
            HookCall::RunTask {
                user,
                task,
                cursor,
                payload,
                respond,
            },
        )?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `notifier.notify()` export with one domain event.
    /// Fire-and-report: the domain-event dispatcher awaits the outcome so
    /// a trapping notifier is recycled like any other call, but a plugin
    /// error never stops the fan-out to other notifiers.
    pub async fn call_notify(&self, event: notifier_types::NotifyEvent) -> NotifyOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance
            .enqueue(&self.id, HookCall::Notify { event, respond })?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `post-scan.on-scan-complete()` export with the
    /// work-id lists from a finished scan. Fire-and-report like
    /// [`Self::call_notify`]: the fan-out awaits each outcome so a
    /// trapping plugin is recycled, but one plugin's error never stops
    /// delivery to the others.
    pub async fn call_post_scan(
        &self,
        library: String,
        added: Vec<String>,
        updated: Vec<String>,
        removed: Vec<String>,
    ) -> PostScanOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance.enqueue(
            &self.id,
            HookCall::PostScan {
                library,
                added,
                updated,
                removed,
                respond,
            },
        )?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `user-link.link-start()` export: begin a per-user
    /// account-link flow, returning the provider's auth URL + state.
    pub async fn call_link_start(&self, user_id: String) -> LinkStartOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance
            .enqueue(&self.id, HookCall::LinkStart { user_id, respond })?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `user-link.link-complete()` export: finish the
    /// link flow from the provider's callback, returning the per-user
    /// secrets the host persists on the plugin's behalf.
    pub async fn call_link_complete(
        &self,
        user_id: String,
        state: String,
        callback_query: String,
    ) -> LinkCompleteOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance.enqueue(
            &self.id,
            HookCall::LinkComplete {
                user_id,
                state,
                callback_query,
                respond,
            },
        )?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    pub async fn call_external_auth_start(
        &self,
        callback_uri: String,
        state: String,
        pkce_challenge: String,
    ) -> ExternalAuthStartOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance.enqueue(
            &self.id,
            HookCall::ExternalAuthStart {
                callback_uri,
                state,
                pkce_challenge,
                respond,
            },
        )?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    pub async fn call_external_auth_complete(
        &self,
        callback_uri: String,
        code: String,
        pkce_verifier: String,
    ) -> ExternalAuthCompleteOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance.enqueue(
            &self.id,
            HookCall::ExternalAuthComplete {
                callback_uri,
                code,
                pkce_verifier,
                respond,
            },
        )?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `embedder.embed()` export. Queued and overlapped
    /// exactly like [`Self::call_search`]. The guest's vectors are checked
    /// against the manifest's `[embedding]` claim (one vector per input
    /// text, each of the claimed dimensionality); a mismatch is a host
    /// error, not a plugin error — the guest broke its declared contract.
    pub async fn call_embed(&self, inputs: Vec<EmbedInput>) -> EmbedOutcome {
        let (respond, answer) = oneshot::channel();
        let inputs_len = inputs.len();
        self.instance
            .enqueue(&self.id, HookCall::Embed { inputs, respond })?;
        let outcome = answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?;
        if let (Ok(Ok(vectors)), Some(claim)) = (&outcome, &self.manifest.embedding) {
            validate_embed_result(claim, inputs_len, vectors)?;
        }
        outcome
    }

    /// Run the plugin's `scrobbler.scrobble()` export with one enriched
    /// scrobble occurrence. Fire-and-report like [`Self::call_notify`]:
    /// the scrobble dispatcher awaits the outcome so a trapping plugin is
    /// recycled, but one scrobbler's error never stops the fan-out.
    pub async fn call_scrobble(&self, event: scrobbler_types::ScrobbleEvent) -> ScrobbleOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance
            .enqueue(&self.id, HookCall::Scrobble { event, respond })?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// Run the plugin's `intro-detector.detect()` export for one edition.
    /// `Ok(None)` from the plugin means "nothing worth marking" — the
    /// intro worker still records the scan so the edition isn't retried
    /// until its inputs change.
    pub async fn call_detect_intro(&self, edition: String) -> IntroDetectOutcome {
        let (respond, answer) = oneshot::channel();
        self.instance
            .enqueue(&self.id, HookCall::IntroDetect { edition, respond })?;
        answer
            .await
            .map_err(|_| HostError::Wasm(format!("{}: instance dropped the call", self.id)))?
    }

    /// True once the instance channel has closed for good (plugin unloaded
    /// or circuit opened and drained). The scheduler uses this to retire
    /// its per-task loops.
    pub(crate) fn instance_closed(&self) -> bool {
        self.instance.calls.is_closed()
    }

    /// Load a plugin from a `.wasm` component on disk. Extracts the embedded
    /// manifest, validates API compatibility, confirms the file is a
    /// well-formed Wasmtime component, and spawns the plugin's instance task.
    ///
    /// Must be called from within a tokio runtime: instantiation itself
    /// happens on the spawned task. Returns the loaded plugin or a
    /// [`HostError`] explaining why it was rejected — the caller logs a
    /// `LoadFailed` lifecycle event in that case.
    pub fn load(
        engine: &Engine,
        linker: &Linker<PluginCtx>,
        path: &Path,
    ) -> Result<Self, HostError> {
        Self::load_with_context(
            engine,
            linker,
            path,
            String::new(),
            None,
            None,
            &EngineConfig::default(),
        )
    }

    /// Like [`Self::load`], with an explicit per-plugin TOML config blob
    /// (rendered back to the guest through `host-config::read`), the
    /// storage handle `host-library` lookups are served from, and the
    /// assets directory `host-assets.read-file` resolves names under
    /// (None disables that import).
    pub fn load_with_context(
        engine: &Engine,
        linker: &Linker<PluginCtx>,
        path: &Path,
        config_toml: String,
        storage: Option<Arc<dyn noflex_storage::Storage>>,
        assets_dir: Option<PathBuf>,
        runtime_config: &EngineConfig,
    ) -> Result<Self, HostError> {
        let bytes = std::fs::read(path)?;

        let binary_sha256 = {
            let mut h = Sha256::new();
            h.update(&bytes);
            hex_encode(&h.finalize())
        };

        let manifest = extract_manifest(&bytes, path)?;
        // Version first: an old plugin should fail with a clear mismatch,
        // not with whichever contract rule its stale manifest trips.
        manifest.check_api_compat(path)?;
        manifest.check_contract(path)?;
        manifest.check_tasks(path)?;
        manifest.check_embedding(path)?;
        manifest.check_events(path)?;
        if manifest.http_egress_hosts.iter().any(|h| h == "*") {
            tracing::warn!(
                plugin = %manifest.plugin_id,
                "plugin requests unrestricted HTTP egress (`*` in http_egress_hosts)"
            );
        }

        let component = Component::from_binary(engine, &bytes)
            .map_err(|e| HostError::Wasm(format!("loading component {}: {e}", path.display())))?;

        let exported_hooks = detect_exported_hooks(&component, &manifest);
        let id = PluginId(manifest.plugin_id.clone());
        validate_declared_hooks(
            &component_export_names(&component),
            &exported_hooks,
            path,
            &id,
        )?;

        let parts = PluginCtxParts {
            plugin_id: id.clone(),
            config_toml: config_toml.clone(),
            declared_secrets: manifest.secrets_read.clone(),
            declared_user_secrets: manifest.user_secrets_read.clone(),
            declared_user_secrets_write: manifest.user_secrets_write.clone(),
            declared_capabilities: manifest.capabilities.clone(),
            declared_tasks: manifest
                .tasks
                .iter()
                .map(|task| task.name.clone())
                .collect(),
            http_egress_hosts: manifest.http_egress_hosts.clone(),
            storage,
            assets_dir,
            inference: runtime_config.inference.clone(),
            limits: PluginLimits::from_mb(
                manifest
                    .memory_limit_mb
                    .unwrap_or(runtime_config.default_memory_limit_mb),
            ),
        };
        let deadlines = Deadlines::build(&manifest, runtime_config);
        let health = Arc::new(RwLock::new(PluginHealth::from_manifest(&manifest)));
        let instance = PluginInstance::spawn(
            engine,
            linker,
            component.clone(),
            parts,
            deadlines,
            Arc::clone(&health),
        );

        Ok(LoadedPlugin {
            id,
            manifest,
            path: path.to_path_buf(),
            binary_sha256,
            exported_hooks,
            component,
            config_toml,
            health,
            instance,
        })
    }

    /// Returns true if this plugin exports the given hook.
    pub fn exports(&self, hook: HookKind) -> bool {
        self.exported_hooks.contains(&hook)
    }
}

/// Per-call wall-clock deadlines, one per hook. Measured from when the
/// instance starts the call, so time spent queued does not count.
#[derive(Debug, Clone)]
struct Deadlines {
    default: Duration,
    per_hook: BTreeMap<HookKind, Duration>,
}

impl Deadlines {
    /// The engine default for every hook, overridden per hook by the
    /// manifest's `[deadlines_secs]`. An override applies to its own hook
    /// only.
    fn build(manifest: &PluginManifest, runtime_config: &EngineConfig) -> Self {
        let per_hook = manifest
            .deadlines_secs
            .iter()
            .filter_map(|(name, seconds)| {
                HookKind::parse(name).map(|hook| (hook, Duration::from_secs(u64::from(*seconds))))
            })
            .collect();
        Self {
            default: Duration::from_secs(u64::from(runtime_config.default_epoch_deadline_secs)),
            per_hook,
        }
    }

    fn for_hook(&self, hook: HookKind) -> Duration {
        self.per_hook.get(&hook).copied().unwrap_or(self.default)
    }
}

/// Check one `embed` result against the manifest's `[embedding]` claim:
/// one vector per input text, every vector exactly the claimed
/// dimensionality. Pure so it is unit-testable without a live instance.
fn validate_embed_result(
    claim: &EmbeddingClaim,
    texts_len: usize,
    vectors: &[Vec<f32>],
) -> Result<(), HostError> {
    if vectors.len() != texts_len {
        return Err(HostError::Wasm(format!(
            "embedder returned {} vectors for {texts_len} texts",
            vectors.len()
        )));
    }
    let want = claim.dimensions as usize;
    if let Some(v) = vectors.iter().find(|v| v.len() != want) {
        return Err(HostError::Wasm(format!(
            "embedder returned a {}-dimensional vector; manifest claims {want}",
            v.len()
        )));
    }
    Ok(())
}

/// Find the `noflex:manifest` custom section in a wasm component binary and
/// parse it. Returns [`HostError::ManifestMissing`] if absent.
///
/// Scans the component binary for a custom section named `noflex:manifest`.
/// The Wasmtime component-section walker handles this without a dedicated
/// parser crate.
pub(crate) fn extract_manifest(bytes: &[u8], path: &Path) -> Result<PluginManifest, HostError> {
    let section = find_custom_section(bytes, "noflex:manifest");
    let payload = section.ok_or_else(|| HostError::ManifestMissing {
        path: path.to_path_buf(),
    })?;
    PluginManifest::from_bytes(payload).map_err(|e| HostError::Manifest {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

/// Scan a WebAssembly Component / Module binary for a custom section
/// with the given name and return its payload bytes.
///
/// Walks the section table. When a component's "core module" subsection
/// is encountered, recurses one level to also scan the embedded module's
/// custom sections — this is where Rust's `#[link_section]` items
/// actually land when a plugin is compiled with wit-bindgen + wasm32-wasip2.
///
/// Component preamble:
///   00 61 73 6d                  -- magic "\0asm"
///   0d 00 01 00                  -- component layer version
/// Sections: <id:u8> <size:LEB128> <payload>.
/// Custom section payload: <name-len:LEB128> <name:utf-8> <data>.
/// Core-module section (id 1 in components): an embedded core wasm module.
fn find_custom_section<'a>(bytes: &'a [u8], target_name: &str) -> Option<&'a [u8]> {
    if bytes.len() < 8 || &bytes[0..4] != b"\0asm" {
        return None;
    }
    // version word at bytes[4..8]: 0x0001_000d for the component layer,
    // 0x0000_0001 for a plain core module. Treat them uniformly — the
    // section layout starts at byte 8 in both.
    let is_component = bytes.len() >= 8 && bytes[6] == 0x01 && bytes[7] == 0x00;
    let mut cursor = 8usize;

    while cursor < bytes.len() {
        let id = bytes[cursor];
        cursor += 1;
        let (size, n) = read_leb128_u32(&bytes[cursor..])?;
        cursor += n;
        let section_end = cursor.checked_add(size as usize)?;
        if section_end > bytes.len() {
            return None;
        }
        if id == 0 {
            if let Some(payload) = match_custom_section(&bytes[cursor..section_end], target_name) {
                return Some(payload);
            }
        } else if is_component && id == 1 {
            // Component "core module" subsection: the nested module bytes.
            // Recurse into it so we also see `#[link_section]`-emitted
            // custom sections that the linker placed there.
            if let Some(payload) = find_custom_section(&bytes[cursor..section_end], target_name) {
                return Some(payload);
            }
        }
        cursor = section_end;
    }
    None
}

/// Given a custom section's payload (everything *after* the section header),
/// return the data slice if its name matches.
fn match_custom_section<'a>(payload: &'a [u8], target_name: &str) -> Option<&'a [u8]> {
    let (name_len, name_n) = read_leb128_u32(payload)?;
    let name_end = name_n.checked_add(name_len as usize)?;
    if name_end > payload.len() {
        return None;
    }
    let name = std::str::from_utf8(&payload[name_n..name_end]).ok()?;
    if name == target_name {
        Some(&payload[name_end..])
    } else {
        None
    }
}

/// LEB128-decode a u32 from the front of `bytes`. Returns the value and the
/// number of bytes consumed, or `None` if malformed / too large.
fn read_leb128_u32(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    for (i, &b) in bytes.iter().enumerate().take(5) {
        let v = (b & 0x7f) as u32;
        result |= v.checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None
}

pub(crate) fn binary_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Returns recognized hooks from the manifest, plus [`HookKind::Background`]
/// whenever `[[tasks]]` are registered (the export is mandatory on every
/// world, so a task-bearing plugin always provides it). Claims only —
/// [`validate_declared_hooks`] cross-checks them against the component's
/// actual exports before the plugin is accepted.
fn detect_exported_hooks(_component: &Component, manifest: &PluginManifest) -> Vec<HookKind> {
    let mut hooks: Vec<HookKind> = manifest
        .hooks
        .iter()
        .filter_map(|h| HookKind::parse(h))
        .collect();
    if !manifest.tasks.is_empty() && !hooks.contains(&HookKind::Background) {
        hooks.push(HookKind::Background);
    }
    hooks
}

/// The component's export names as strings (e.g.
/// `noflex:plugin/metadata@0.6.0`). Gathered once at load time so
/// validation never instantiates.
fn component_export_names(component: &Component) -> Vec<String> {
    component
        .component_type()
        .exports(component.engine())
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Cross-check the manifest's hook claims against the component's actual
/// exports. A plugin that claims a hook it doesn't export would silently
/// never serve that hook (or trap at bind time), so the mismatch is a
/// load-time rejection naming the missing export. A manifest with no
/// recognized hooks at all is rejected outright — there is nothing to
/// dispatch to.
fn validate_declared_hooks(
    export_names: &[String],
    hooks: &[HookKind],
    path: &Path,
    plugin_id: &PluginId,
) -> Result<(), HostError> {
    if hooks.is_empty() {
        return Err(HostError::Manifest {
            path: path.to_path_buf(),
            reason: "manifest declares no recognized hooks".into(),
        });
    }
    for hook in hooks {
        let exported = export_names.iter().any(|name| {
            name.strip_prefix("noflex:plugin/")
                .and_then(|rest| rest.split('@').next())
                == Some(hook.as_str())
        });
        if !exported {
            return Err(HostError::HookNotExported {
                id: plugin_id.0.clone(),
                hook: hook.as_str(),
            });
        }
    }
    Ok(())
}

impl PluginInstance {
    /// Spawn the plugin's instance task: it owns the one `Store<PluginCtx>`,
    /// instantiates the component, and serves hook calls from the channel
    /// inside a single `Store::run_concurrent` scope for the store's whole
    /// lifetime. One scope is what lets N hook calls overlap on the one
    /// guest instance — `call_concurrent` may only run inside it.
    ///
    /// Any trap recycles the instance (fresh store + instantiate,
    /// exponential backoff); [`MAX_CONSECUTIVE_TRAPS`] traps in a row open
    /// the circuit breaker and every queued call fails with the circuit
    /// error until the plugin is reloaded.
    fn spawn(
        engine: &Engine,
        linker: &Linker<PluginCtx>,
        component: Component,
        parts: PluginCtxParts,
        deadlines: Deadlines,
        health: Arc<RwLock<PluginHealth>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(CALL_QUEUE_CAPACITY);
        let engine = engine.clone();
        let linker = linker.clone();
        tokio::spawn(run_instance(
            engine, linker, component, parts, deadlines, rx, health,
        ));
        PluginInstance { calls: tx }
    }
}

/// Why one instance lifetime ended: the call channel closed for good
/// (clean drain on unload) or a trap/event-loop failure forced a recycle.
enum ServeEnd {
    ChannelClosed,
    Trap(String),
}

/// Body of the instance task. Runs until the call channel closes (all
/// [`LoadedPlugin`] handles dropped) and in-flight calls have drained.
///
/// Traps are assumed to poison guest state, so any trap — including an
/// instantiation failure — tears the store down and re-instantiates with
/// exponential backoff instead of letting later calls observe a corrupted
/// instance. The backoff resets whenever an instance served at least one
/// call successfully, so a mostly-healthy plugin isn't ratcheted into the
/// circuit breaker by rare traps spread over its lifetime.
async fn run_instance(
    engine: Engine,
    linker: Linker<PluginCtx>,
    component: Component,
    parts: PluginCtxParts,
    deadlines: Deadlines,
    mut rx: mpsc::Receiver<HookCall>,
    health: Arc<RwLock<PluginHealth>>,
) {
    let plugin_id = parts.plugin_id.clone();
    let mut consecutive_traps = 0u32;
    let mut backoff = RECYCLE_BACKOFF_INITIAL;
    loop {
        let (end, served_ok) =
            serve_instance(&engine, &linker, &component, &parts, &deadlines, &mut rx).await;
        let ServeEnd::Trap(msg) = end else {
            return;
        };
        if served_ok > 0 {
            consecutive_traps = 0;
            backoff = RECYCLE_BACKOFF_INITIAL;
        }
        consecutive_traps += 1;
        let circuit = consecutive_traps >= MAX_CONSECUTIVE_TRAPS;
        {
            let mut h = health.write().expect("health lock poisoned");
            h.instance_restarts += 1;
            h.last_trap = Some(msg.clone());
            h.circuit_open = circuit;
        }
        if circuit {
            tracing::error!(
                plugin = %plugin_id,
                traps = consecutive_traps,
                "plugin instance disabled after repeated traps; reload to recover"
            );
            let reason = format!(
                "{plugin_id}: instance disabled after {consecutive_traps} consecutive traps"
            );
            while let Some(call) = rx.recv().await {
                call.fail(&reason);
            }
            return;
        }
        tracing::warn!(
            plugin = %plugin_id,
            error = %msg,
            attempt = consecutive_traps,
            backoff_ms = backoff.as_millis() as u64,
            "plugin instance trapped; recycling"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECYCLE_BACKOFF_MAX);
    }
}

/// One instance lifetime: build a fresh store, instantiate, and serve the
/// call channel until it closes or a trap ends the instance. Returns the
/// ending reason plus how many calls completed (any result, including a
/// plugin-error — what matters is the store survived to answer), which the
/// supervisor uses to reset its backoff.
async fn serve_instance(
    engine: &Engine,
    linker: &Linker<PluginCtx>,
    component: &Component,
    parts: &PluginCtxParts,
    deadlines: &Deadlines,
    rx: &mut mpsc::Receiver<HookCall>,
) -> (ServeEnd, u64) {
    let plugin_id = parts.plugin_id.clone();
    let mut store: Store<PluginCtx> = Store::new(engine, parts.build());
    store.limiter(|ctx| &mut ctx.limits);
    // The epoch deadline belongs to the store, which outlives any one call,
    // so it is re-armed every tick rather than set once: the callback traps
    // only when the call executing right now is past its own deadline, and
    // otherwise yields to the executor. A call that overruns while parked
    // (awaiting a host import) is caught by `CallEnv::track`'s timer.
    store.epoch_deadline_callback(|mut store| {
        let calls = store.data().calls.clone();
        Ok(if calls.current_overdue(&mut store) {
            UpdateDeadline::Interrupt
        } else {
            UpdateDeadline::Yield(1)
        })
    });
    store.set_epoch_deadline(1);

    // A plugin may export any subset of hooks: bind each world with
    // `.ok()` and let binding success be the source of truth for what
    // the instance can serve. A component binding to nothing exports no
    // recognized hook — an instantiate failure, not a trap-recyclable
    // plugin.
    let bindings = async {
        let instance = linker.instantiate_async(&mut store, component).await?;
        let hooks = BoundHooks {
            metadata: MetadataPlugin::new(&mut store, &instance).ok(),
            image: ImagePlugin::new(&mut store, &instance).ok(),
            notifier: NotifierPlugin::new(&mut store, &instance).ok(),
            post_scan: PostScanPlugin::new(&mut store, &instance).ok(),
            user_link: UserLinkPlugin::new(&mut store, &instance).ok(),
            external_auth: ExternalAuthPlugin::new(&mut store, &instance).ok(),
            embedder: EmbedderPluginBinding::new(&mut store, &instance).ok(),
            scrobbler: ScrobblerPlugin::new(&mut store, &instance).ok(),
            intro_detector: IntroDetectorPlugin::new(&mut store, &instance).ok(),
        };
        Ok::<_, wasmtime::Error>(hooks)
    }
    .await;
    let hooks = match bindings {
        Ok(h) if h.any_bound() => h,
        Ok(_) => {
            return (
                ServeEnd::Trap(format!(
                    "instantiate {plugin_id}: component exports no recognized hook"
                )),
                0,
            );
        }
        Err(e) => return (ServeEnd::Trap(format!("instantiate {plugin_id}: {e}")), 0),
    };

    // Shared with every in-flight call future: the first dispatch-level
    // failure (trap, deadline overrun, event-loop error) wins and ends the
    // instance; every answered call bumps `served_ok`.
    let served_ok = Arc::new(AtomicU64::new(0));
    let trap = Arc::new(std::sync::Mutex::<Option<String>>::new(None));
    let env = CallEnv {
        plugin_id: plugin_id.clone(),
        deadlines: deadlines.clone(),
        calls: store.data().calls.clone(),
        trap: Arc::clone(&trap),
        served_ok: Arc::clone(&served_ok),
    };

    let event_loop_plugin_id = plugin_id.clone();
    let result = store
        .run_concurrent(async move |accessor: &Accessor<PluginCtx>| {
            let env = &env;
            let hooks = &hooks;
            let mut inflight: Inflight<'_> = futures::stream::FuturesUnordered::new();
            let mut open = true;
            while open || !inflight.is_empty() {
                if env.trapped() {
                    break;
                }
                // Stop draining the queue while the in-flight set is full:
                // the bounded channel then fills and `enqueue` answers
                // `Overloaded`, which is the backpressure callers see.
                let accepting = open && inflight.len() < MAX_INFLIGHT_CALLS;
                tokio::select! {
                    msg = rx.recv(), if accepting => match msg {
                        Some(call) if call.is_cancelled() => {}
                        Some(call) => dispatch_call(env, &mut inflight, accessor, hooks, call),
                        None => open = false,
                    },
                    Some(()) = inflight.next(), if !inflight.is_empty() => {}
                }
            }
        })
        .await;

    let trapped = trap.lock().expect("trap flag poisoned").take();
    let served = served_ok.load(Ordering::Relaxed);
    let end = match (trapped, result) {
        (Some(m), _) => ServeEnd::Trap(m),
        (None, Err(e)) => ServeEnd::Trap(format!(
            "{event_loop_plugin_id}: instance event loop failed: {e}"
        )),
        (None, Ok(())) => ServeEnd::ChannelClosed,
    };
    (end, served)
}

/// Record one completed dispatch: any answered call proves the store is
/// healthy; the first dispatch-level error flags the instance for
/// recycling. In-flight calls racing the trap still answer their callers
/// before the scope unwinds.
fn note_dispatch<T>(
    result: &Result<T, HostError>,
    trap: &std::sync::Mutex<Option<String>>,
    served_ok: &AtomicU64,
) {
    match result {
        Ok(_) => {
            served_ok.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => {
            let mut flag = trap.lock().expect("trap flag poisoned");
            if flag.is_none() {
                *flag = Some(e.to_string());
            }
        }
    }
}

/// Answer a caller whose hook the component doesn't export. Same
/// message shape the image arm has always used.
fn missing_export<T>(respond: oneshot::Sender<Result<T, HostError>>, hook: &'static str) {
    let _ = respond.send(Err(HostError::Wasm(format!(
        "plugin does not export the {hook} hook"
    ))));
}

/// In-flight call futures of one instance lifetime; they borrow the scope's
/// accessor, bound hooks, and [`CallEnv`].
type Inflight<'a> =
    futures::stream::FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send + 'a>>>;

/// State shared by every in-flight call of one instance lifetime.
struct CallEnv {
    plugin_id: PluginId,
    deadlines: Deadlines,
    calls: CallTable,
    trap: Arc<std::sync::Mutex<Option<String>>>,
    served_ok: Arc<AtomicU64>,
}

impl CallEnv {
    fn trapped(&self) -> bool {
        self.trap
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// Track the call `start` builds under `hook`'s deadline and answer
    /// `respond` with its outcome. `start` receives the deadline so the call
    /// registers it with the store's [`CallTable`], where the epoch
    /// callback interrupts the call if it is still executing past it; a
    /// call parked past it (awaiting an import) is cut off by the timer
    /// here. Either way the caller sees `DeadlineExceeded` and the instance
    /// recycles: the component model cannot cancel a single task short of
    /// dropping the store, so an overrunning task would otherwise run on.
    fn track<'a, T, F, Fut>(
        &'a self,
        inflight: &mut Inflight<'a>,
        hook: HookKind,
        respond: oneshot::Sender<Result<T, HostError>>,
        start: F,
    ) where
        T: Send + 'a,
        F: FnOnce(Instant) -> Fut,
        Fut: Future<Output = Result<T, HostError>> + Send + 'a,
    {
        let budget = self.deadlines.for_hook(hook);
        let deadline = Instant::now() + budget;
        let call = start(deadline);
        // Answers from `Drop` too: the epoch callback's interrupt trap ends
        // the whole instance scope, dropping this future unanswered.
        let answer = Answer {
            respond: Some(respond),
            plugin_id: self.plugin_id.clone(),
            hook,
            budget,
            deadline,
        };
        inflight.push(Box::pin(async move {
            let outcome =
                tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), call).await;
            let result = match outcome {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(e)) if Instant::now() < deadline => Err(e),
                _ => Err(answer.overrun()),
            };
            note_dispatch(&result, &self.trap, &self.served_ok);
            answer.send(result);
        }));
    }
}

/// The caller's response channel for one tracked call. Exactly one answer
/// is sent: the call's outcome, or — if the instance stops first — the
/// deadline overrun when the call was overdue, else an instance-stopped
/// error.
struct Answer<T> {
    respond: Option<oneshot::Sender<Result<T, HostError>>>,
    plugin_id: PluginId,
    hook: HookKind,
    budget: Duration,
    deadline: Instant,
}

impl<T> Answer<T> {
    fn overrun(&self) -> HostError {
        HostError::DeadlineExceeded {
            id: self.plugin_id.0.clone(),
            hook: self.hook.as_str(),
            seconds: self.budget.as_secs(),
        }
    }

    fn send(mut self, result: Result<T, HostError>) {
        if let Some(respond) = self.respond.take() {
            let _ = respond.send(result);
        }
    }
}

impl<T> Drop for Answer<T> {
    fn drop(&mut self) {
        if let Some(respond) = self.respond.take() {
            let error = if Instant::now() >= self.deadline {
                self.overrun()
            } else {
                HostError::Wasm(format!(
                    "{}: instance stopped before answering",
                    self.plugin_id
                ))
            };
            let _ = respond.send(Err(error));
        }
    }
}

/// Start one export call acting for `user` and await its result. The
/// call is registered in `calls` (principal + deadline) before the guest
/// task first runs and unregistered when this future completes or is
/// dropped.
async fn invoke<P, R>(
    accessor: &Accessor<PluginCtx>,
    calls: &CallTable,
    deadline: Instant,
    func: TypedFunc<P, R>,
    params: P,
    user: Option<noflex_core::UserId>,
    what: &'static str,
) -> Result<R, HostError>
where
    P: ComponentNamedList + Lower + Send + Sync + 'static,
    R: ComponentNamedList + Lift + Send + Sync + 'static,
{
    let (call, _registration) = accessor
        .with(|mut access| {
            let call = func.start_call_concurrent(&mut access, params)?;
            let registration = calls.register(call.task(), user, deadline);
            Ok::<_, wasmtime::Error>((call, registration))
        })
        .map_err(|e| HostError::Wasm(format!("call {what}: {e}")))?;
    func.finish_call_concurrent(accessor, call)
        .await
        .map_err(|e| HostError::Wasm(format!("call {what}: {e}")))
}

/// Parse a WIT user id into the principal a user-bound call acts for. An
/// unparseable id binds no one, so per-user imports fail closed.
fn principal_of(user: &str) -> Option<noflex_core::UserId> {
    user.parse::<uuid::Uuid>()
        .ok()
        .map(noflex_core::UserId::from_uuid)
}

/// Route one queued call to its export and track it in `inflight`.
fn dispatch_call<'a>(
    env: &'a CallEnv,
    inflight: &mut Inflight<'a>,
    accessor: &'a Accessor<PluginCtx>,
    hooks: &'a BoundHooks,
    call: HookCall,
) {
    let calls = &env.calls;
    match call {
        HookCall::Search { query, respond } => match &hooks.metadata {
            Some(metadata) => env.track(
                inflight,
                HookKind::Metadata,
                respond,
                move |deadline| async move {
                    let func = metadata.noflex_plugin_metadata().func_search();
                    invoke(accessor, calls, deadline, func, (query,), None, "search")
                        .await
                        .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "metadata"),
        },
        HookCall::Supports { kind, respond } => match &hooks.metadata {
            Some(metadata) => env.track(
                inflight,
                HookKind::Metadata,
                respond,
                move |deadline| async move {
                    let func = metadata.noflex_plugin_metadata().func_supports();
                    invoke(accessor, calls, deadline, func, (kind,), None, "supports")
                        .await
                        .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "metadata"),
        },
        HookCall::FetchMetadata { id, respond } => match &hooks.metadata {
            Some(metadata) => env.track(
                inflight,
                HookKind::Metadata,
                respond,
                move |deadline| async move {
                    let func = metadata.noflex_plugin_metadata().func_fetch();
                    invoke(accessor, calls, deadline, func, (id,), None, "fetch")
                        .await
                        .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "metadata"),
        },
        HookCall::FetchImage {
            id,
            kind,
            language,
            respond,
        } => match &hooks.image {
            Some(image) => env.track(inflight, HookKind::Image, respond, move |deadline| {
                fetch_image_buffered(
                    accessor,
                    calls,
                    deadline,
                    image,
                    &env.plugin_id,
                    id,
                    kind,
                    language,
                )
            }),
            None => missing_export(respond, "image"),
        },
        HookCall::RunTask {
            user,
            task,
            cursor,
            payload,
            respond,
        } => env.track(inflight, HookKind::Background, respond, move |deadline| {
            run_task_on(
                accessor, calls, deadline, hooks, user, task, cursor, payload,
            )
        }),
        HookCall::Notify { event, respond } => match &hooks.notifier {
            Some(notifier) => env.track(
                inflight,
                HookKind::Notifier,
                respond,
                move |deadline| async move {
                    let func = notifier.noflex_plugin_notifier().func_notify();
                    invoke(accessor, calls, deadline, func, (event,), None, "notify")
                        .await
                        .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "notifier"),
        },
        HookCall::PostScan {
            library,
            added,
            updated,
            removed,
            respond,
        } => match &hooks.post_scan {
            Some(post_scan) => env.track(
                inflight,
                HookKind::PostScan,
                respond,
                move |deadline| async move {
                    let func = post_scan.noflex_plugin_post_scan().func_on_scan_complete();
                    let params = (library, added, updated, removed);
                    invoke(
                        accessor,
                        calls,
                        deadline,
                        func,
                        params,
                        None,
                        "on-scan-complete",
                    )
                    .await
                    .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "post-scan"),
        },
        HookCall::LinkStart { user_id, respond } => match &hooks.user_link {
            Some(user_link) => env.track(
                inflight,
                HookKind::UserLink,
                respond,
                move |deadline| async move {
                    let principal = principal_of(&user_id);
                    let func = user_link.noflex_plugin_user_link().func_link_start();
                    invoke(
                        accessor,
                        calls,
                        deadline,
                        func,
                        (user_id,),
                        principal,
                        "link-start",
                    )
                    .await
                    .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "user-link"),
        },
        HookCall::LinkComplete {
            user_id,
            state,
            callback_query,
            respond,
        } => match &hooks.user_link {
            Some(user_link) => env.track(
                inflight,
                HookKind::UserLink,
                respond,
                move |deadline| async move {
                    let principal = principal_of(&user_id);
                    let func = user_link.noflex_plugin_user_link().func_link_complete();
                    let params = (user_id, state, callback_query);
                    invoke(
                        accessor,
                        calls,
                        deadline,
                        func,
                        params,
                        principal,
                        "link-complete",
                    )
                    .await
                    .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "user-link"),
        },
        HookCall::ExternalAuthStart {
            callback_uri,
            state,
            pkce_challenge,
            respond,
        } => match &hooks.external_auth {
            Some(auth) => env.track(
                inflight,
                HookKind::Auth,
                respond,
                move |deadline| async move {
                    let func = auth.noflex_plugin_external_auth().func_start();
                    let params = (callback_uri, state, pkce_challenge);
                    invoke(
                        accessor,
                        calls,
                        deadline,
                        func,
                        params,
                        None,
                        "external-auth start",
                    )
                    .await
                    .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "external-auth"),
        },
        HookCall::ExternalAuthComplete {
            callback_uri,
            code,
            pkce_verifier,
            respond,
        } => match &hooks.external_auth {
            Some(auth) => env.track(
                inflight,
                HookKind::Auth,
                respond,
                move |deadline| async move {
                    let func = auth.noflex_plugin_external_auth().func_complete();
                    let params = (callback_uri, code, pkce_verifier);
                    invoke(
                        accessor,
                        calls,
                        deadline,
                        func,
                        params,
                        None,
                        "external-auth complete",
                    )
                    .await
                    .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "external-auth"),
        },
        HookCall::Embed { inputs, respond } => match &hooks.embedder {
            Some(embedder) => env.track(
                inflight,
                HookKind::Embedder,
                respond,
                move |deadline| async move {
                    let func = embedder.noflex_plugin_embedder().func_embed();
                    invoke(accessor, calls, deadline, func, (inputs,), None, "embed")
                        .await
                        .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "embedder"),
        },
        HookCall::Scrobble { event, respond } => match &hooks.scrobbler {
            Some(scrobbler) => env.track(
                inflight,
                HookKind::Scrobbler,
                respond,
                move |deadline| async move {
                    let principal = principal_of(&event.user);
                    let func = scrobbler.noflex_plugin_scrobbler().func_scrobble();
                    invoke(
                        accessor,
                        calls,
                        deadline,
                        func,
                        (event,),
                        principal,
                        "scrobble",
                    )
                    .await
                    .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "scrobbler"),
        },
        HookCall::IntroDetect { edition, respond } => match &hooks.intro_detector {
            Some(detector) => env.track(
                inflight,
                HookKind::IntroDetector,
                respond,
                move |deadline| async move {
                    let func = detector.noflex_plugin_intro_detector().func_detect();
                    invoke(accessor, calls, deadline, func, (edition,), None, "detect")
                        .await
                        .map(|(r,)| r)
                },
            ),
            None => missing_export(respond, "intro-detector"),
        },
    }
}

/// Invoke `background.run-task` through whichever world binding the
/// component provided, acting for `user` (the enqueuer of a dynamic
/// payload, or `None`). Every NoFlex world exports `background`, and the
/// per-world generated accessor types are distinct — hence the chain
/// rather than a single stored accessor.
// Mirrors the `run-task` export's arguments plus the call's dispatch context.
#[allow(clippy::too_many_arguments)]
async fn run_task_on(
    accessor: &Accessor<PluginCtx>,
    calls: &CallTable,
    deadline: Instant,
    hooks: &BoundHooks,
    user: Option<noflex_core::UserId>,
    task: String,
    cursor: Option<String>,
    payload: Option<String>,
) -> RunTaskOutcome {
    macro_rules! run_on {
        ($($binding:ident),+ $(,)?) => {
            $(
                if let Some(binding) = &hooks.$binding {
                    let func = binding.noflex_plugin_background().func_run_task();
                    let params = (task, cursor, payload);
                    return invoke(accessor, calls, deadline, func, params, user, "run-task")
                        .await
                        .map(|(r,)| r.map(TaskOutcome::from).map_err(WitPluginError::from));
                }
            )+
        };
    }
    run_on!(
        metadata,
        image,
        notifier,
        post_scan,
        user_link,
        external_auth,
        embedder,
        scrobbler,
        intro_detector,
    );
    // Unreachable: serve_instance rejects components with no bound hook.
    Err(HostError::Wasm(
        "plugin exports no hook interfaces".to_string(),
    ))
}

/// Invoke `image.fetch` and read the returned payload stream to
/// completion. Upper layers (image cache, GraphQL) keep `Vec<u8>` — the
/// stream is the wire shape, not a pipeline rewrite.
// Mirrors the `image.fetch` export's arguments plus the call's dispatch context.
#[allow(clippy::too_many_arguments)]
async fn fetch_image_buffered(
    accessor: &Accessor<PluginCtx>,
    calls: &CallTable,
    deadline: Instant,
    image: &ImagePlugin,
    plugin_id: &PluginId,
    external_id: image_types::ExternalId,
    kind: image_types::ImageKind,
    language: Option<String>,
) -> FetchImageOutcome {
    let func = image.noflex_plugin_image().func_fetch();
    let params = (external_id, kind, language);
    let (result,) = invoke(accessor, calls, deadline, func, params, None, "image.fetch").await?;
    let (meta, stream) = match result {
        Ok(ok) => ok,
        Err(e) => return Ok(Err(e)),
    };
    let bytes = read_stream_to_end(accessor, plugin_id, stream).await?;
    Ok(Ok((meta, bytes)))
}

/// Drain a guest-produced byte stream into one buffer. Completion is
/// signaled by the guest dropping its write end, which drops the
/// collector and closes the channel.
async fn read_stream_to_end(
    accessor: &Accessor<PluginCtx>,
    plugin_id: &PluginId,
    stream: StreamReader<u8>,
) -> Result<Vec<u8>, HostError> {
    let (tx, mut rx) = futures::channel::mpsc::channel::<Vec<u8>>(IMAGE_STREAM_CHUNKS);
    accessor
        .with(|access| stream.pipe(access, ByteCollector { tx }))
        .map_err(|e| HostError::Wasm(format!("pipe image stream: {e}")))?;
    let mut buf = Vec::new();
    while let Some(chunk) = rx.next().await {
        if buf.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            return Err(HostError::ImageTooLarge {
                id: plugin_id.0.clone(),
                limit_bytes: MAX_IMAGE_BYTES,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(dimensions: u32) -> EmbeddingClaim {
        EmbeddingClaim {
            namespace: "fixture-embed".into(),
            model_id: "fixture/embed-v1".into(),
            dimensions,
        }
    }

    #[test]
    fn configured_deadlines_apply_to_their_own_hook_only() {
        let manifest = PluginManifest::from_bytes(
            format!(
                "manifest_version = 1\nplugin_id = \"p\"\ndisplay_name = \"P\"\n\
                 version = \"1\"\nnoflex_api_version = \"{}.{}\"\nhooks = [\"metadata\"]\n\
                 [deadlines_secs]\nmetadata = 3",
                noflex_plugin_wit::API_MAJOR,
                noflex_plugin_wit::API_MINOR,
            )
            .as_bytes(),
        )
        .unwrap();
        let config = EngineConfig {
            default_epoch_deadline_secs: 7,
            ..EngineConfig::default()
        };
        let deadlines = Deadlines::build(&manifest, &config);
        assert_eq!(
            deadlines.for_hook(HookKind::Metadata),
            Duration::from_secs(3)
        );
        assert_eq!(deadlines.for_hook(HookKind::Image), Duration::from_secs(7));
        assert_eq!(
            deadlines.for_hook(HookKind::Background),
            Duration::from_secs(7)
        );
    }

    #[test]
    fn call_queue_rejects_overload() {
        let (calls, _rx) = mpsc::channel(1);
        let instance = PluginInstance { calls };
        let id = PluginId("test".into());
        let call = || {
            let (respond, _answer) = oneshot::channel();
            HookCall::IntroDetect {
                edition: "edition".into(),
                respond,
            }
        };
        instance.enqueue(&id, call()).unwrap();
        assert!(matches!(
            instance.enqueue(&id, call()),
            Err(HostError::Overloaded { .. })
        ));
    }

    #[test]
    fn validate_embed_result_accepts_matching_shape() {
        let vectors = vec![vec![0.0; 8], vec![1.0; 8]];
        validate_embed_result(&claim(8), 2, &vectors).expect("shape matches the claim");
    }

    #[test]
    fn validate_embed_result_rejects_count_mismatch() {
        let vectors = vec![vec![0.0; 8]];
        let err = validate_embed_result(&claim(8), 2, &vectors).expect_err("one vector short");
        assert!(err.to_string().contains("1 vectors for 2 texts"));
    }

    #[test]
    fn validate_embed_result_rejects_dimension_mismatch() {
        let vectors = vec![vec![0.0; 8], vec![0.0; 4]];
        let err = validate_embed_result(&claim(8), 2, &vectors).expect_err("wrong dimensions");
        assert!(err.to_string().contains("4-dimensional"));
    }

    #[test]
    fn leb128_roundtrip() {
        // Encodes 624485 == 0xE5 0x8E 0x26 in LEB128 (classic example).
        let bytes = [0xE5, 0x8E, 0x26];
        let (v, n) = read_leb128_u32(&bytes).expect("decodes");
        assert_eq!(v, 624485);
        assert_eq!(n, 3);
    }

    #[test]
    fn finds_custom_section_in_minimal_module() {
        // Hand-rolled WASM module with one custom section named "demo"
        // containing the bytes "hi".
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\0asm"); // magic
        wasm.extend_from_slice(&[0x01, 0, 0, 0]); // module version (we
        //                                              accept any version
        //                                              header as long as
        //                                              the magic matches)
        // Custom section: id=0, then size, then name-len, name, payload.
        let name = b"demo";
        let payload = b"hi";
        let mut body = Vec::new();
        body.push(name.len() as u8); // LEB128 for small values
        body.extend_from_slice(name);
        body.extend_from_slice(payload);
        wasm.push(0); // section id
        wasm.push(body.len() as u8); // section size (LEB128 for small values)
        wasm.extend_from_slice(&body);

        let found = find_custom_section(&wasm, "demo").expect("section present");
        assert_eq!(found, payload);
    }

    #[test]
    fn validation_accepts_versioned_interface_names() {
        let names = vec![
            "noflex:plugin/metadata@0.6.0".to_string(),
            "noflex:plugin/background@0.6.0".to_string(),
            "component:adapted/plugin".to_string(),
        ];
        let id = PluginId("p".into());
        validate_declared_hooks(
            &names,
            &[HookKind::Metadata, HookKind::Background],
            Path::new("p.wasm"),
            &id,
        )
        .expect("declared hooks are exported");
    }

    #[test]
    fn validation_rejects_a_claimed_but_missing_export() {
        let names = vec!["noflex:plugin/metadata@0.6.0".to_string()];
        let err = validate_declared_hooks(
            &names,
            &[HookKind::Metadata, HookKind::Image],
            Path::new("p.wasm"),
            &PluginId("p".into()),
        )
        .expect_err("image is claimed but not exported");
        assert!(matches!(
            err,
            HostError::HookNotExported { hook: "image", .. }
        ));
    }

    #[test]
    fn validation_rejects_a_hookless_manifest() {
        let err = validate_declared_hooks(
            &["noflex:plugin/metadata@0.6.0".to_string()],
            &[],
            Path::new("p.wasm"),
            &PluginId("p".into()),
        )
        .expect_err("no recognized hooks is a load error");
        assert!(matches!(err, HostError::Manifest { .. }));
    }

    #[test]
    fn detected_hooks_include_background_when_tasks_registered() {
        let manifest = PluginManifest::from_bytes(
            br#"
            manifest_version = 1
            plugin_id = "p"
            display_name = "P"
            version = "0.1.0"
            noflex_api_version = "0.10"
            hooks = ["metadata"]
            capabilities = ["background"]

            [[tasks]]
            name = "sweep"
            interval_ms = 50
            "#,
        )
        .expect("parses");
        // The component argument is only used for its type, so a dummy
        // is fine here — hook detection reads the manifest only.
        let engine = crate::engine::build_engine(&crate::EngineConfig::default()).expect("engine");
        let component = Component::new(&engine, b"\0asm\x0d\x00\x01\x00").expect("empty component");
        let hooks = detect_exported_hooks(&component, &manifest);
        assert!(hooks.contains(&HookKind::Metadata));
        assert!(hooks.contains(&HookKind::Background));
    }

    #[test]
    fn ignores_non_matching_section_names() {
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\0asm");
        wasm.extend_from_slice(&[0x01, 0, 0, 0]);
        let mut body = Vec::new();
        let name: &[u8] = b"other";
        body.push(name.len() as u8);
        body.extend_from_slice(name);
        body.extend_from_slice(b"xx");
        wasm.push(0);
        wasm.push(body.len() as u8);
        wasm.extend_from_slice(&body);

        assert!(find_custom_section(&wasm, "noflex:manifest").is_none());
    }
}
