// SPDX-License-Identifier: AGPL-3.0-only
//
// `noflex-storage` defines the backend-agnostic `Storage` trait, the
// storage-shape domain types it returns, and a conformance test suite that
// every backend must pass.
//
// **Zero SQL lives here.** SQL is the concern of backend crates
// (`noflex-storage-turso`, community-shipped
// `noflex-storage-{postgres,mysql,...}`).

pub mod access;
pub mod canonical;
pub mod connection;
pub mod domain;
pub mod error;
pub mod query;
pub mod search;
pub mod store;
pub mod test_suite;

pub use access::{CallerSession, RequestUser};
pub use domain::{
    ActivityEntry, ActivityKind, ArtworkPreference, Chapter, Collection, Edition, EpgProgram,
    ExportFormat, ExportJob, ExportJobStatus, HistoryPage, HistoryRow, Image, ImageSource,
    ImageTarget, Library, LibraryConfig, LibraryKind, LibraryScanStatus, LiveTvChannel,
    LiveTvSource, MediaFile, MediaUrlSigner, Playlist, PluginTaskPayload, PluginTaskState,
    Recording, RecordingSchedule, RecordingScheduleStatus, RecordingStatus, ScanEvent, ScanState,
    ScanSummary, Session, Tag, TagSummary, TrickplayAsset, TrickplayChapterCue, TrickplaySheet,
    User, UserData, Work,
};
pub use error::{StorageError, StorageResult};
pub use query::{
    HybridQuery, ItemPage, ListTagsQuery, ListWorksQuery, MatchMode, Page, PageCursor, SearchMode,
    SortOrder, TagQuery, WatchedFilter, WorkFacet, WorkFacetValue, WorkFacetValues,
    WorkFacetsQuery, WorkRule, WorkRules,
};
pub use search::{HealthReport, SearchHit, VectorHit};
pub use store::{Storage, Transaction};
