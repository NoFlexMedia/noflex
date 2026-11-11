// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    model::{ExternalId, WorkKind},
};

/// What to find subtitles for. Hash matches often miss, so it carries all a
/// provider might search by: the file's fingerprint, then ids, then title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleQuery {
    /// OpenSubtitles' moviehash of the file; `None` for one under 64 KiB or
    /// one that couldn't be read.
    pub moviehash: Option<String>,
    pub file_size: u64,
    /// The file's name without its folders.
    pub file_name: String,
    pub title: String,
    pub year: Option<i32>,
    pub kind: WorkKind,
    pub external_ids: Vec<ExternalId>,
    /// An episode's series ids, season and episode; empty and `None` otherwise.
    pub series_external_ids: Vec<ExternalId>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// ISO 639-2/T codes, as [`crate::text::canonical_language`] spells them;
    /// empty for any language.
    pub languages: Vec<String>,
}

/// One subtitle a provider offers for a [`SubtitleQuery`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleMatch {
    /// The plugin that found it, tagged by the host.
    pub provider: String,
    /// Opaque; handed back to [`SubtitleProvider::download`].
    pub handle: String,
    /// ISO 639-2/T.
    pub language: String,
    /// BCP 47 where the provider tells regions apart (`pt-BR`).
    pub locale: Option<String>,
    /// `srt`, `vtt`, `ass` or `ssa`.
    pub format: String,
    pub release: Option<String>,
    pub hearing_impaired: bool,
    pub forced: bool,
    /// Found by the file's moviehash, so timed for this exact file.
    pub hash_match: bool,
    pub download_count: Option<u64>,
    /// The provider's own rating; higher is better.
    pub score: Option<f32>,
}

/// A downloaded subtitle: its bytes as the provider sent them.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleFile {
    /// `srt`, `vtt`, `ass` or `ssa`.
    pub format: String,
    pub bytes: Bytes,
}

#[async_trait]
pub trait SubtitleProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn search(&self, query: &SubtitleQuery) -> PluginResult<Vec<SubtitleMatch>>;

    async fn download(&self, handle: &str) -> PluginResult<SubtitleFile>;
}
