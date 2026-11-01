// SPDX-License-Identifier: AGPL-3.0-only
//
// Filesystem scanner with Plex-style filename + folder resolution.
//
// Two entrypoints:
//
//   - [`FilesystemScanner::scan_library`] — one-shot. Tests use this.
//   - [`FilesystemScanner::watch`] — long-lived `notify`-based watcher that
//     debounces filesystem events and triggers full rescans.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use noflex_core::{DomainEventBus, LibraryId};
use noflex_storage::Storage;

mod artwork;
pub mod ffprobe;
pub mod parser;
pub mod registry;
mod scan;
mod subtitles;
pub mod walker;
mod watcher;

pub use parser::{
    EpisodeClassification, MovieClassification, MovieIdentity, ParsedEpisode, ParsedMovie,
    SeasonFolderInfo, classify_episode, classify_movie, parse_filename_movie, parse_season_folder,
    slugify,
};
pub use registry::{ScanRegistry, ScanStatus, ScanStatusReader};
pub use walker::VIDEO_EXTS;
pub use watcher::Watcher;

#[async_trait]
pub trait Scanner: Send + Sync {
    async fn scan_library(&self, library: LibraryId) -> anyhow::Result<ScanSummary>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub added: u32,
    pub updated: u32,
    pub removed: u32,
    pub errored: u32,
}

pub struct FilesystemScanner {
    storage: Arc<dyn Storage>,
    events: Option<DomainEventBus>,
}

impl FilesystemScanner {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            events: None,
        }
    }

    /// Domain-event sink for scan lifecycle and per-work creates.
    /// Optional so tests and lean embeddings stay silent.
    pub fn with_events(mut self, events: DomainEventBus) -> Self {
        self.events = Some(events);
        self
    }

    pub async fn scan_library(&self, library: LibraryId) -> anyhow::Result<ScanSummary> {
        scan::run(self.storage.as_ref(), library, self.events.as_ref()).await
    }

    /// Start a `notify`-based watcher on every root of the library. The
    /// returned handle stops watching when dropped or when `shutdown()` is
    /// called.
    pub async fn watch(&self, library: LibraryId) -> anyhow::Result<Watcher> {
        let libs = self.storage.list_libraries().await?;
        let lib = libs
            .into_iter()
            .find(|l| l.id == library)
            .ok_or_else(|| anyhow::anyhow!("library {library} not found"))?;
        let roots: Vec<PathBuf> = lib.paths.iter().map(PathBuf::from).collect();
        watcher::start(self.storage.clone(), library, roots, self.events.clone())
    }
}

#[async_trait]
impl Scanner for FilesystemScanner {
    async fn scan_library(&self, library: LibraryId) -> anyhow::Result<ScanSummary> {
        FilesystemScanner::scan_library(self, library).await
    }
}
