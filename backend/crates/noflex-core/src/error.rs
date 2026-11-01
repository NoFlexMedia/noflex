// SPDX-License-Identifier: AGPL-3.0-only

use thiserror::Error;

/// Error type every extension trait returns. Flat and serializable so it can
/// cross a WASM boundary without re-design.
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),

    #[error("bad input: {0}")]
    BadInput(String),

    #[error("not found")]
    NotFound,

    #[error("rate limited; retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u32 },

    #[error("internal provider error: {0}")]
    Internal(String),
}

pub type PluginResult<T> = std::result::Result<T, PluginError>;
