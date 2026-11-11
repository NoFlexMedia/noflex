// SPDX-License-Identifier: AGPL-3.0-only
//
// noflex-plugin-host is the WASM plugin runtime: engine setup, component
// loading, capability wrappers, audit logging, hot reload, lifecycle events,
// host imports, and provider priority.

mod attribution;
pub mod audit;
mod bindings;
pub mod discovery;
mod dispatch;
pub mod dispatcher;
pub mod embeddings;
mod engine;
pub mod error;
pub mod events;
pub mod inference;
pub mod intro;
mod manifest;
pub mod metadata_service;
pub mod metadata_worker;
mod net;
mod plugin;
pub mod priority;
mod registry;
pub mod remote_embedder;
pub mod resource_limits;
mod scheduler;
pub mod scrobbler;
pub mod secrets;
mod settings;
pub mod watcher;

pub use bindings::{
    embedder,
    embedder::exports::noflex::plugin::embedder::{EmbedDocument, EmbedInput},
    external_auth, metadata_and_image, metadata_only,
    metadata_only::{
        MetadataPlugin, exports::noflex::plugin::metadata as plugin_metadata,
        noflex::plugin::types as plugin_types,
    },
    notifier,
    notifier::NotifierPlugin,
    user_link::{self, UserLinkPlugin},
};
pub use discovery::{PluginDir, SearchPath, Tier};
pub use dispatch::{PluginCtx, PluginCtxParts, build_linker};
pub use embeddings::spawn_embedding_worker;
pub use engine::{EngineConfig, build_engine, spawn_epoch_ticker};
pub use error::HostError;
pub use events::{DiscoveredEvent, LifecycleBus, PluginLifecycleEvent, UnloadReason};
pub use inference::{Inference, InferenceConfig, InferenceDevice};
pub use intro::spawn_intro_detector_worker;
pub use manifest::{
    EmbeddingClaim, HookKind, ManifestParseError, PluginManifest, PriorityKey, SettingKind,
    SettingSpec, TaskRegistration, WorkKindFilter,
};
pub use metadata_service::{
    ArtworkCandidate, AutoIdentify, MetadataError, MetadataService, ServableImage,
};
pub use metadata_worker::{MetadataWorker, SweepReport, spawn_metadata_worker};
pub use plugin::{LoadedPlugin, PluginHealth, PluginId, TaskHealth, TaskOutcome};
pub use priority::{Overrides, PluginOverride, PluginOverrideEntry, PriorityTable};
pub use registry::{
    EmbedOutcome, Plugin, PluginRegistry, PluginTaskHealth, embed_inputs, metadata_fanout,
    metadata_fetch, metadata_payload_from_wit, to_wit_kind_id,
};
pub use remote_embedder::{RemoteEmbedError, RemoteEmbedder};
pub use resource_limits::{PluginLimitState, PluginLimits};
pub use scrobbler::spawn_scrobble_dispatcher;
pub use secrets::SecretStore;
pub use settings::{ConfigStanzas, PluginSetting, SettingSource, SettingsError};
pub use watcher::PluginWatcher;
