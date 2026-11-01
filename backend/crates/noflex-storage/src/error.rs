// SPDX-License-Identifier: AGPL-3.0-only

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("not found")]
    NotFound,

    #[error("conflict: {0}")]
    Conflict(String),

    /// Caller supplied something unparseable (e.g. a malformed cursor).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("constraint violation: {0}")]
    Constraint(String),

    /// Transient — caller should retry with backoff.
    #[error("transient backend error: {0}")]
    Transient(String),

    /// Catch-all. Backends wrap their engine-specific errors here.
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub type StorageResult<T> = std::result::Result<T, StorageError>;
