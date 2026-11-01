// SPDX-License-Identifier: AGPL-3.0-only
//
// Owns one filesystem watcher per library with `watch_enabled`, kept in
// step with library mutations: creating a library starts its watcher,
// changing its paths restarts it, disabling or deleting it stops it.

use std::{collections::HashMap, sync::Arc};

use noflex_core::LibraryId;
use noflex_graphql::LibraryWatchControl;
use noflex_scanner::{FilesystemScanner, Watcher};
use noflex_storage::Storage;
use tokio::sync::Mutex;

use crate::system_status::Recorder;

pub struct LibraryWatchers {
    storage: Arc<dyn Storage>,
    scanner: Arc<FilesystemScanner>,
    recorder: Arc<Recorder>,
    running: Mutex<HashMap<LibraryId, Watcher>>,
}

impl LibraryWatchers {
    pub fn new(
        storage: Arc<dyn Storage>,
        scanner: Arc<FilesystemScanner>,
        recorder: Arc<Recorder>,
    ) -> Self {
        Self {
            storage,
            scanner,
            recorder,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Start watchers for every library that wants one. Failures are
    /// logged and recorded per library; they never abort startup.
    pub async fn start_all(&self) {
        match self.storage.list_libraries().await {
            Ok(libraries) => {
                for library in libraries {
                    self.sync(library.id).await;
                }
            }
            Err(error) => tracing::warn!(%error, "listing libraries for filesystem watch failed"),
        }
    }

    /// Bring `library`'s watcher in line with its stored row: (re)start
    /// it when the library exists with `watch_enabled`, stop it
    /// otherwise. Restarting picks up changed root paths.
    pub async fn sync(&self, library: LibraryId) {
        let mut running = self.running.lock().await;
        if let Some(previous) = running.remove(&library)
            && let Err(error) = previous.shutdown().await
        {
            tracing::warn!(%library, %error, "library watcher stopped with an error");
        }
        let wanted = match self.storage.get_library(library).await {
            Ok(row) => row.is_some_and(|row| row.config.watch_enabled),
            Err(error) => {
                tracing::warn!(%library, %error, "reading library for watcher failed");
                false
            }
        };
        if !wanted {
            self.recorder.watcher_stopped(library);
            return;
        }
        match self.scanner.watch(library).await {
            Ok(watcher) => {
                self.recorder.watcher_started(library, Ok(()));
                running.insert(library, watcher);
            }
            Err(error) => {
                self.recorder
                    .watcher_started(library, Err(error.to_string()));
                tracing::warn!(%library, %error, "library watcher start failed");
            }
        }
    }
}

#[async_trait::async_trait]
impl LibraryWatchControl for LibraryWatchers {
    async fn library_changed(&self, library: LibraryId) {
        self.sync(library).await;
    }
}
