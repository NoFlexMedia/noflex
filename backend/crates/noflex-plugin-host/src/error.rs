// SPDX-License-Identifier: AGPL-3.0-only

use std::path::PathBuf;

use noflex_core::PluginError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("wasmtime: {0}")]
    Wasm(String),

    #[error("manifest parse error at {path}: {reason}")]
    Manifest { path: PathBuf, reason: String },

    #[error("manifest absent: no `noflex:manifest` custom section in {path:?}")]
    ManifestMissing { path: PathBuf },

    #[error(
        "plugin api version mismatch: expected \"{expected}\", plugin requires \"{plugin_api}\""
    )]
    ApiVersionMismatch {
        expected: String,
        plugin_api: String,
    },

    /// The call was never started: the plugin's queue is full. Not a hook
    /// failure — callers that walk provider candidates must not treat it
    /// as one.
    #[error("plugin {id} call queue is full")]
    Overloaded { id: String },

    /// The call overran its per-hook deadline; the instance is recycled.
    #[error("plugin {id} exceeded its {seconds}s `{hook}` deadline")]
    DeadlineExceeded {
        id: String,
        hook: &'static str,
        seconds: u64,
    },

    #[error("plugin {id} image exceeded the {limit_bytes}-byte limit")]
    ImageTooLarge { id: String, limit_bytes: usize },

    #[error("plugin {id} not found in registry")]
    Unknown { id: String },

    #[error("plugin {id} is unhealthy (too many traps); not dispatching")]
    Unhealthy { id: String },

    #[error("plugin {id} does not export hook `{hook}`")]
    HookNotExported { id: String, hook: &'static str },

    #[error("plugin {id} does not declare the `{capability}` capability")]
    MissingCapability {
        id: String,
        capability: &'static str,
    },

    #[error("plugin {id} has no background task `{task}` (manifest registers: {registered})")]
    UnknownTask {
        id: String,
        task: String,
        registered: String,
    },

    #[error("plugin task payload exceeds the {limit_bytes}-byte limit")]
    TaskPayloadTooLarge { limit_bytes: usize },

    #[error("task queue not configured on this host")]
    TaskQueueUnavailable,

    #[error("storage: {0}")]
    Storage(#[from] noflex_storage::StorageError),

    #[error("plugin error from {id}: {source}")]
    Plugin {
        id: String,
        #[source]
        source: PluginError,
    },
}

impl From<wasmtime::Error> for HostError {
    fn from(e: wasmtime::Error) -> Self {
        HostError::Wasm(e.to_string())
    }
}
