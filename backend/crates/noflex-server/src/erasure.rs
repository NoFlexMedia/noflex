// SPDX-License-Identifier: AGPL-3.0-only
//
// Carrying out what `deleteAccount` asked for. An ordinary deletion is over by
// the time it answers, but for a purge of what doesn't cascade from the user
// row: the activity log, and files on disk. The last administrator's erases the
// server, which happens at startup, before anything else reads what it erases.
// Both are recorded in the database first, so whatever a restart cuts short is
// taken up again at the next start.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use noflex_graphql::Erasure;
use noflex_storage::{Storage, domain::AccountPurge};
use tokio::task::JoinHandle;

use crate::{config::Config, optimize_worker::version_file};

/// How long a purge that failed waits for another try, if no deletion wakes
/// the worker first.
const RETRY_EVERY: Duration = Duration::from_secs(60 * 60);

/// Clears what deleted accounts left behind, as `deleteAccount` queues it.
pub struct PurgeWorker {
    pub storage: Arc<dyn Storage>,
    pub erasure: Arc<Erasure>,
}

impl PurgeWorker {
    /// Purge whatever's pending, from before a restart too, then again each
    /// time an account is deleted.
    pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if let Err(error) = self.run_pending().await {
                    tracing::warn!(%error, "account purge failed; retrying later");
                }
                tokio::select! {
                    () = self.erasure.purge_requested() => {}
                    () = tokio::time::sleep(RETRY_EVERY) => {}
                }
            }
        })
    }

    /// Purge every pending account, oldest first. Returns how many were done.
    ///
    /// # Errors
    ///
    /// Storage's, which leave that purge and the ones after it pending.
    pub async fn run_pending(&self) -> anyhow::Result<usize> {
        let pending = self.storage.pending_account_purges().await?;
        for purge in &pending {
            self.purge(purge).await?;
        }
        Ok(pending.len())
    }

    async fn purge(&self, purge: &AccountPurge) -> anyhow::Result<()> {
        let rows = self.storage.forget_user_activity(purge.user_id).await?;
        let mut removed = 0;
        for file in &purge.files {
            // Another account's job sharing the copy keeps it.
            if self.storage.export_output_in_use(file).await? {
                continue;
            }
            match tokio::fs::remove_file(file).await {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                // Kept from blocking the rest; an operator can clear it by hand.
                Err(error) => tracing::warn!(%error, file, "deleted account's file not removed"),
            }
        }
        self.storage.finish_account_purge(purge.user_id).await?;
        tracing::info!(user = %purge.user_id, rows, files = removed, "deleted account purged");
        Ok(())
    }
}

/// What the server made on disk, all of which an erasure removes. Media files
/// and the configuration are nowhere in it.
#[derive(Debug, Clone)]
pub struct ResetPaths {
    /// Emptied of everything: the caches, export copies and recordings.
    pub emptied: Vec<PathBuf>,
    /// Where optimized versions go. An operator may have chosen a folder that
    /// holds their own files, so only what the worker names its files goes.
    pub optimized: Vec<PathBuf>,
    /// The token signing key, so no token from before outlives the erasure.
    pub signing_key: PathBuf,
}

impl ResetPaths {
    pub fn for_config(cfg: &Config) -> Self {
        Self {
            emptied: vec![
                cfg.transcode_dir(),
                cfg.image_dir(),
                cfg.embed_dir(),
                cfg.download_dir(),
                cfg.recordings_dir(),
            ],
            optimized: vec![cfg.optimized_dir()],
            signing_key: cfg.data_dir.join("jwt.key"),
        }
    }
}

/// Carry out the erasure the last administrator asked for, if one is pending:
/// every row the database holds, then what the server made on disk, and the
/// request itself last. Must run before anything else uses `storage`, and
/// before the signing key is loaded. Returns whether there was one.
///
/// # Errors
///
/// Storage's or the filesystem's, leaving the erasure pending for the next
/// start.
pub async fn finish_reset(storage: &dyn Storage, paths: &ResetPaths) -> anyhow::Result<bool> {
    let Some(reset) = storage.pending_server_reset().await? else {
        return Ok(false);
    };
    tracing::warn!("erasing the server back to first-run setup");
    storage
        .erase_all_data()
        .await
        .context("erase the database")?;
    for dir in &paths.emptied {
        empty(dir)
            .await
            .with_context(|| format!("empty {}", dir.display()))?;
    }
    let chosen = reset.folders.iter().map(PathBuf::from);
    for dir in paths.optimized.iter().cloned().chain(chosen) {
        remove_versions(&dir)
            .await
            .with_context(|| format!("clear optimized versions from {}", dir.display()))?;
    }
    match tokio::fs::remove_file(&paths.signing_key).await {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("remove the signing key"),
    }
    storage.finish_server_reset().await?;
    tracing::warn!("server erased; it's waiting for first-run setup");
    Ok(true)
}

/// Remove everything in `dir`, which stays. A symlink goes without what it
/// points to being touched.
async fn empty(dir: &Path) -> std::io::Result<()> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            tokio::fs::remove_dir_all(entry.path()).await?;
        } else {
            tokio::fs::remove_file(entry.path()).await?;
        }
    }
    Ok(())
}

async fn remove_versions(dir: &Path) -> std::io::Result<()> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    while let Some(entry) = entries.next_entry().await? {
        if version_file(&entry.file_name().to_string_lossy()).is_some()
            && entry.file_type().await?.is_file()
        {
            tokio::fs::remove_file(entry.path()).await?;
        }
    }
    Ok(())
}
