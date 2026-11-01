// SPDX-License-Identifier: AGPL-3.0-only
//
// Map turso engine errors onto the canonical `StorageError` enum.

use noflex_storage::error::StorageError;

pub(crate) fn map_turso(err: turso::Error) -> StorageError {
    match err {
        turso::Error::Constraint(msg) => StorageError::Constraint(msg),
        turso::Error::Busy(msg) | turso::Error::BusySnapshot(msg) => StorageError::Transient(msg),
        turso::Error::QueryReturnedNoRows => StorageError::NotFound,
        other => StorageError::Backend(Box::new(other)),
    }
}

pub(crate) fn backend<E>(err: E) -> StorageError
where
    E: std::error::Error + Send + Sync + 'static,
{
    StorageError::Backend(Box::new(err))
}

pub(crate) fn backend_msg(msg: impl Into<String>) -> StorageError {
    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    struct Msg(String);
    StorageError::Backend(Box::new(Msg(msg.into())))
}
