// SPDX-License-Identifier: AGPL-3.0-only
//
// Host-side metadata application: choosing provider candidates for a
// work, applying a provider payload (fields, tags, external ids), pulling
// the matching artwork through the winning provider's `image` hook, and
// matching unidentified works automatically. GraphQL's refresh/identify
// mutations and the background metadata worker both drive this, so the
// rules live in one place.
//
// Plugins never see storage: they get external ids and hints in, and
// hand payloads and image bytes back; every write happens here. All
// egress stays inside plugins — the host never downloads a URL itself,
// payload image URLs only tell it which kinds a provider has.

use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use async_graphql::Json;
use noflex_core::{
    ExternalId, ExternalIdKind, ImageKind, MetadataStatus, TagId, TagTarget, WorkId, WorkKind,
    providers::MetadataPayload, slugify,
};
use noflex_metadata::{TagDraft, apply_payload, merge_external_id};
use noflex_storage::{
    Storage, StorageError,
    domain::{Image, ImageSource, ImageTarget, Library, Tag, Work},
};
use time::OffsetDateTime;
use tokio::sync::RwLock;

use crate::{
    HookKind, HostError, LoadedPlugin, PluginId, PluginRegistry, PriorityTable, WorkKindFilter,
    metadata_and_image::noflex::plugin::types as image_types,
    metadata_fetch, metadata_payload_from_wit, plugin_types,
    registry::{from_wit_kind_id, to_wit_kind, to_wit_kind_id},
};

/// Directory under the image cache root holding `<work id>/<kind>.<ext>`.
pub const PROVIDER_ART_DIR: &str = "provider-art";

/// Lowest search confidence an automatic match is accepted at.
pub const AUTO_MATCH_THRESHOLD: f32 = 0.85;
/// How far the best automatic match must lead the runner-up; closer
/// calls (same-titled remakes without a year) are left for a human.
pub const AUTO_MATCH_MARGIN: f32 = 0.05;
/// Languages tried per image kind before settling for what came back.
const MAX_IMAGE_LANGUAGES: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("work not found")]
    WorkNotFound,
    #[error("work has user edits; pass force: true to refresh anyway")]
    UserEdited,
    #[error("work has no external ids; identify it first")]
    NoExternalIds,
    #[error("no metadata provider could fetch this work")]
    NoProvider,
    #[error("provider not loaded: {0}")]
    ProviderNotLoaded(String),
    #[error("provider {0} does not export the metadata hook")]
    NotMetadataProvider(String),
    #[error("provider {provider} rejected the fetch: {error}")]
    Rejected { provider: String, error: String },
    #[error("metadata fetch failed: {0}")]
    Dispatch(#[from] HostError),
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
}

/// What [`MetadataService::auto_identify`] did with one work.
#[derive(Debug, Clone)]
pub enum AutoIdentify {
    /// Matched, fetched and applied from `provider`.
    Matched { provider: PluginId, work: Box<Work> },
    /// Not eligible: confirmed or user-edited, already carries provider
    /// ids, a kind that is never auto-matched, or (for seasons and
    /// episodes) a parent series that is not identified yet.
    Skipped,
    /// Searched, but no candidate produced a confident, unambiguous match
    /// that could be fetched.
    NoMatch,
}

/// Applies provider metadata and artwork to works. Cheap to share behind
/// an `Arc`; holds no per-work state.
pub struct MetadataService {
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    priority: Option<Arc<RwLock<PriorityTable>>>,
    image_dir: Option<PathBuf>,
}

impl MetadataService {
    /// Without a priority table candidates fall back to registry order;
    /// without an image dir no artwork is fetched.
    pub fn new(registry: Arc<PluginRegistry>, storage: Arc<dyn Storage>) -> Self {
        Self {
            registry,
            storage,
            priority: None,
            image_dir: None,
        }
    }

    /// Rank default candidates by `table` (see [`Self::candidates`]).
    pub fn with_priority_table(mut self, table: Arc<RwLock<PriorityTable>>) -> Self {
        self.priority = Some(table);
        self
    }

    /// `dir` is the image cache root the image route serves from;
    /// provider art lands under [`PROVIDER_ART_DIR`] there.
    pub fn with_image_dir(mut self, dir: PathBuf) -> Self {
        self.image_dir = Some(dir);
        self
    }

    /// The store this service reads and writes.
    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }

    /// Whether any loaded plugin exports `hook`.
    pub fn has_exporters(&self, hook: HookKind) -> bool {
        !self.registry.exporters_of(hook).is_empty()
    }

    /// Plugins to try for `hook` on a `kind` work in `library`, best
    /// first. A non-empty `preferred_providers` is the whole answer: those
    /// ids in that order, keeping only loaded plugins that export `hook`
    /// and (when they export metadata) say they support `kind`. Otherwise
    /// the priority table's ranking for (hook, kind), or every exporter in
    /// registry order when that ranking is empty.
    pub async fn candidates(
        &self,
        hook: HookKind,
        kind: WorkKind,
        library: Option<&Library>,
    ) -> Vec<PluginId> {
        let preferred = library
            .map(|l| l.config.preferred_providers.as_slice())
            .unwrap_or_default();
        if preferred.is_empty() {
            return self.default_candidates(hook, kind).await;
        }
        let mut out = Vec::new();
        for id in preferred {
            let id = PluginId(id.clone());
            if out.contains(&id) {
                continue;
            }
            let Some(plugin) = self.registry.get_plugin(&id) else {
                continue;
            };
            if plugin.exports(hook) && supports(&plugin, kind).await {
                out.push(id);
            }
        }
        out
    }

    async fn default_candidates(&self, hook: HookKind, kind: WorkKind) -> Vec<PluginId> {
        let ranked = match &self.priority {
            Some(table) => table
                .read()
                .await
                .ranked(hook, work_kind_filter(kind))
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        if ranked.is_empty() {
            self.registry.exporters_of(hook)
        } else {
            ranked
        }
    }

    async fn load(&self, id: WorkId) -> Result<(Work, Option<Library>), MetadataError> {
        let work = self
            .storage
            .get_work(id)
            .await?
            .ok_or(MetadataError::WorkNotFound)?;
        let library = self.storage.get_library(work.library_id).await?;
        Ok((work, library))
    }

    /// Re-fetch from the first candidate that can serve one of the work's
    /// external ids, then apply it and its artwork. Locked fields and
    /// user-assigned tags survive.
    ///
    /// # Errors
    /// [`MetadataError::UserEdited`] for a user-edited work unless
    /// `force`; [`MetadataError::NoExternalIds`] when it has none;
    /// [`MetadataError::NoProvider`] when every candidate failed.
    pub async fn refresh(&self, id: WorkId, force: bool) -> Result<Work, MetadataError> {
        let (work, library) = self.load(id).await?;
        if work.metadata_status == MetadataStatus::UserEdited && !force {
            return Err(MetadataError::UserEdited);
        }
        if work.external_ids.is_empty() {
            return Err(MetadataError::NoExternalIds);
        }
        let candidates = self
            .candidates(HookKind::Metadata, work.kind, library.as_ref())
            .await;
        let (provider, payload) = metadata_fetch(&self.registry, &candidates, &work.external_ids)
            .await
            .ok_or(MetadataError::NoProvider)?;
        self.apply(work, &provider, &payload, library.as_ref())
            .await
    }

    /// Fetch `external_id` from `provider` explicitly and apply it — the
    /// fix for a wrong match. Skips the user-edits guard (the caller just
    /// named the right match); locked fields still win.
    ///
    /// # Errors
    /// [`MetadataError::ProviderNotLoaded`] /
    /// [`MetadataError::NotMetadataProvider`] for an unusable provider;
    /// [`MetadataError::Rejected`] when the plugin refuses the id;
    /// [`MetadataError::Dispatch`] when the call itself fails.
    pub async fn identify(
        &self,
        id: WorkId,
        provider: &str,
        external_id: ExternalId,
    ) -> Result<Work, MetadataError> {
        let (mut work, library) = self.load(id).await?;
        let plugin_id = PluginId(provider.to_string());
        let plugin = self
            .registry
            .get_plugin(&plugin_id)
            .ok_or_else(|| MetadataError::ProviderNotLoaded(provider.to_string()))?;
        if !plugin.exports(HookKind::Metadata) {
            return Err(MetadataError::NotMetadataProvider(provider.to_string()));
        }
        let payload = fetch_one(&plugin, &external_id).await?;
        merge_external_id(&mut work, external_id);
        self.apply(work, &plugin_id, &payload, library.as_ref())
            .await
    }

    /// Persist `payload` onto `work` from `provider`: fields (locks win),
    /// provider-sourced tags, external ids, then artwork. Artwork failures
    /// are logged, never returned — the metadata is already saved.
    async fn apply(
        &self,
        mut work: Work,
        provider: &PluginId,
        payload: &MetadataPayload,
        library: Option<&Library>,
    ) -> Result<Work, MetadataError> {
        let report = apply_payload(&mut work, payload);
        persist_tags(self.storage.as_ref(), work.id, &provider.0, &report.tags).await?;
        work.updated_at = OffsetDateTime::now_utc();
        self.storage.upsert_work(&work).await?;
        // Vectors are computed from this metadata, and a work is usually
        // embedded on being added, before it has any. The next sweep redoes it.
        if let Err(error) = self.storage.delete_work_embeddings(work.id).await {
            tracing::warn!(work = %work.id, %error, "metadata: stale embedding not cleared");
        }

        let mut kinds: Vec<(ImageKind, Option<String>)> = Vec::new();
        for image in &payload.images {
            if !kinds.iter().any(|(k, _)| *k == image.kind) {
                kinds.push((image.kind, image.language.clone()));
            }
        }
        let exclusive = !kinds.is_empty();
        if kinds.is_empty() {
            kinds = default_art_kinds(work.kind)
                .iter()
                .map(|k| (*k, None))
                .collect();
        }
        let written = self
            .fetch_artwork(&work, Some(provider), &kinds, library)
            .await;
        if exclusive {
            // The winning provider has no art of these kinds; another
            // provider's leftovers would mix sources within one work.
            self.drop_provider_art_except(&work, &kinds).await;
        }
        tracing::debug!(work = %work.id, provider = %provider.0, written, "metadata applied");
        Ok(work)
    }

    /// Fetch each of `kinds` for `work` through the `image` hook and record
    /// it as the work's `Provider` image, replacing the previous one. Tries
    /// `winner` first (when it exports `image`), then the other image
    /// candidates; within a plugin, each of the work's non-`Custom`
    /// external ids. The language asked for is the library's preferred
    /// languages in order, else the payload's, else none. Returns how many
    /// images were written; every failure is logged and skipped. A no-op
    /// without an image dir or provider ids.
    pub async fn fetch_artwork(
        &self,
        work: &Work,
        winner: Option<&PluginId>,
        kinds: &[(ImageKind, Option<String>)],
        library: Option<&Library>,
    ) -> u32 {
        let Some(root) = &self.image_dir else {
            return 0;
        };
        let ids: Vec<&ExternalId> = provider_ids(work).collect();
        if ids.is_empty() || kinds.is_empty() {
            return 0;
        }
        let mut plugins: Vec<PluginId> = Vec::new();
        if let Some(winner) = winner
            && self
                .registry
                .get_plugin(winner)
                .is_some_and(|p| p.exports(HookKind::Image))
        {
            plugins.push(winner.clone());
        }
        for id in self.candidates(HookKind::Image, work.kind, library).await {
            if !plugins.contains(&id) {
                plugins.push(id);
            }
        }
        let library_languages: Vec<String> = library
            .map(|l| l.config.preferred_languages.clone())
            .unwrap_or_default();

        let mut written = 0;
        for (kind, payload_language) in kinds {
            let languages: Vec<Option<String>> = if !library_languages.is_empty() {
                library_languages
                    .iter()
                    .take(MAX_IMAGE_LANGUAGES)
                    .cloned()
                    .map(Some)
                    .collect()
            } else {
                vec![payload_language.clone()]
            };
            let Some(fetched) = self.fetch_one_kind(&plugins, &ids, *kind, &languages).await else {
                continue;
            };
            match store_art(self.storage.as_ref(), root, work.id, *kind, fetched).await {
                Ok(()) => written += 1,
                Err(error) => {
                    tracing::warn!(work = %work.id, kind = kind.as_str(), %error, "provider art not stored");
                }
            }
        }
        written
    }

    /// The first usable image of `kind` across `plugins` × `ids`. Each
    /// language in `languages` is tried until one comes back in that
    /// language or text-free; otherwise the first answer is kept.
    async fn fetch_one_kind(
        &self,
        plugins: &[PluginId],
        ids: &[&ExternalId],
        kind: ImageKind,
        languages: &[Option<String>],
    ) -> Option<FetchedArt> {
        for plugin_id in plugins {
            let Some(plugin) = self.registry.get_plugin(plugin_id) else {
                continue;
            };
            for id in ids {
                let mut fallback = None;
                for language in languages {
                    match fetch_image(&plugin, id, kind, language.clone()).await {
                        Some(art) if art.matches_language(language.as_deref()) => {
                            return Some(art);
                        }
                        Some(art) => {
                            fallback.get_or_insert(art);
                        }
                        None => break,
                    }
                }
                if fallback.is_some() {
                    return fallback;
                }
            }
        }
        None
    }

    async fn drop_provider_art_except(&self, work: &Work, keep: &[(ImageKind, Option<String>)]) {
        let target = ImageTarget::Work(work.id);
        let rows = match self
            .storage
            .list_images(target, &[ImageSource::Provider])
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(work = %work.id, %error, "provider art listing failed");
                return;
            }
        };
        for row in rows {
            if keep.iter().any(|(k, _)| *k == row.kind) {
                continue;
            }
            if let Err(error) = self
                .storage
                .delete_image(target, row.kind, ImageSource::Provider)
                .await
            {
                tracing::warn!(work = %work.id, %error, "stale provider art not removed");
                continue;
            }
            // Only files this service wrote; a row pointing elsewhere is
            // dropped without touching the file.
            if let Some(root) = &self.image_dir
                && Path::new(&row.path).starts_with(root.join(PROVIDER_ART_DIR))
            {
                let _ = tokio::fs::remove_file(&row.path).await;
            }
        }
    }

    /// Fill `Provider` artwork for an identified work without re-fetching
    /// its metadata: the kinds [`default_art_kinds`] lists for its kind,
    /// from the image candidates in order. Returns images written.
    pub async fn backfill_artwork(&self, work: &Work) -> Result<u32, MetadataError> {
        let library = self.storage.get_library(work.library_id).await?;
        let kinds: Vec<(ImageKind, Option<String>)> = default_art_kinds(work.kind)
            .iter()
            .map(|k| (*k, None))
            .collect();
        Ok(self
            .fetch_artwork(work, None, &kinds, library.as_ref())
            .await)
    }

    /// Match an unidentified work automatically: search the candidates in
    /// order and take the first provider whose best match clears
    /// [`AUTO_MATCH_THRESHOLD`] and leads the runner-up by
    /// [`AUTO_MATCH_MARGIN`], fetch that id, and apply it with artwork.
    /// Only `Unconfirmed` works without provider ids are touched (the
    /// work is re-read just before writing, so a concurrent user edit
    /// wins); seasons and episodes match through their identified series
    /// via search hints. Locked fields stay locked.
    pub async fn auto_identify(&self, id: WorkId) -> Result<AutoIdentify, MetadataError> {
        let (work, library) = self.load(id).await?;
        if !auto_eligible(&work) {
            return Ok(AutoIdentify::Skipped);
        }
        let Some(query) = self.search_query(&work).await? else {
            return Ok(AutoIdentify::Skipped);
        };
        let candidates = self
            .candidates(HookKind::Metadata, work.kind, library.as_ref())
            .await;
        for provider in candidates {
            let Some(plugin) = self.registry.get_plugin(&provider) else {
                continue;
            };
            let matches = match plugin.call_search(query.clone()).await {
                Ok(Ok(matches)) => matches,
                Ok(Err(error)) => {
                    tracing::debug!(work = %work.id, plugin = %provider.0, ?error, "auto-identify: search refused");
                    continue;
                }
                Err(error) => {
                    tracing::debug!(work = %work.id, plugin = %provider.0, %error, "auto-identify: search failed");
                    continue;
                }
            };
            let Some(best) = confident_match(matches) else {
                continue;
            };
            let payload = match fetch_one(&plugin, &best).await {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::debug!(work = %work.id, plugin = %provider.0, %error, "auto-identify: fetch failed");
                    continue;
                }
            };
            // A user may have edited or identified the work meanwhile.
            let (mut fresh, library) = self.load(id).await?;
            if !auto_eligible(&fresh) {
                return Ok(AutoIdentify::Skipped);
            }
            merge_external_id(&mut fresh, best);
            let work = self
                .apply(fresh, &provider, &payload, library.as_ref())
                .await?;
            return Ok(AutoIdentify::Matched {
                provider,
                work: Box::new(work),
            });
        }
        Ok(AutoIdentify::NoMatch)
    }

    /// The search an automatic match runs for `work`, or `None` when a
    /// season/episode's series is not identified (or the chain is broken).
    async fn search_query(
        &self,
        work: &Work,
    ) -> Result<Option<plugin_types::SearchQuery>, MetadataError> {
        let mut hints = Vec::new();
        match work.kind {
            WorkKind::Season | WorkKind::Episode => {
                let mut series = work.parent_id;
                let mut season = work.index_number;
                let mut episode = None;
                if work.kind == WorkKind::Episode {
                    episode = work.index_number;
                    season = work.parent_index;
                    let parent = match work.parent_id {
                        Some(p) => self.storage.get_work(p).await?,
                        None => None,
                    };
                    series = parent.as_ref().and_then(|p| p.parent_id);
                    if season.is_none() {
                        season = parent.and_then(|p| p.index_number);
                    }
                }
                let Some(series) = (match series {
                    Some(s) => self.storage.get_work(s).await?,
                    None => None,
                }) else {
                    return Ok(None);
                };
                let parent_ids: Vec<&ExternalId> = provider_ids(&series).collect();
                let Some(season) = season else {
                    return Ok(None);
                };
                if parent_ids.is_empty() || (work.kind == WorkKind::Episode && episode.is_none()) {
                    return Ok(None);
                }
                for id in parent_ids {
                    hints.push(format!(
                        "parent-external-id:{}:{}",
                        wit_kind_name(id.kind),
                        id.value
                    ));
                }
                hints.push(format!("season:{season}"));
                if let Some(episode) = episode {
                    hints.push(format!("episode:{episode}"));
                }
            }
            _ => {}
        }
        Ok(Some(plugin_types::SearchQuery {
            title: work.title.clone(),
            year: work.year,
            kind: to_wit_kind(work.kind),
            hints,
        }))
    }
}

/// The art kinds asked of providers when no payload says which exist.
pub fn default_art_kinds(kind: WorkKind) -> &'static [ImageKind] {
    match kind {
        WorkKind::Movie | WorkKind::Series => {
            &[ImageKind::Primary, ImageKind::Backdrop, ImageKind::Logo]
        }
        WorkKind::Episode => &[ImageKind::Thumb],
        WorkKind::Artist => &[ImageKind::Primary, ImageKind::Backdrop],
        _ => &[ImageKind::Primary],
    }
}

/// Kinds automatic matching runs for. Tracks and photos are left to
/// their albums or to manual identification.
fn auto_kind(kind: WorkKind) -> bool {
    matches!(
        kind,
        WorkKind::Movie
            | WorkKind::Series
            | WorkKind::Season
            | WorkKind::Episode
            | WorkKind::Artist
            | WorkKind::Album
            | WorkKind::AudioBook
    )
}

/// Unconfirmed, of an auto-matched kind, and without provider ids.
pub fn auto_eligible(work: &Work) -> bool {
    work.metadata_status == MetadataStatus::Unconfirmed
        && auto_kind(work.kind)
        && provider_ids(work).next().is_none()
}

/// The work's external ids a provider could know — every kind but the
/// scanner's `Custom` scan keys.
pub fn provider_ids(work: &Work) -> impl Iterator<Item = &ExternalId> {
    work.external_ids
        .iter()
        .filter(|id| id.kind != ExternalIdKind::Custom)
}

/// The best match when it clears the threshold and leads every other
/// distinct id by the margin.
fn confident_match(matches: Vec<plugin_types::MetadataMatch>) -> Option<ExternalId> {
    let mut seen = HashSet::new();
    let mut ranked: Vec<plugin_types::MetadataMatch> = matches
        .into_iter()
        .filter(|m| m.confidence.is_finite())
        .filter(|m| {
            seen.insert((
                from_wit_kind_id(m.external_id.kind),
                m.external_id.value.clone(),
            ))
        })
        .collect();
    ranked.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let best = ranked.first()?;
    if best.confidence < AUTO_MATCH_THRESHOLD {
        return None;
    }
    if let Some(second) = ranked.get(1)
        && best.confidence - second.confidence < AUTO_MATCH_MARGIN
    {
        return None;
    }
    Some(ExternalId {
        kind: from_wit_kind_id(best.external_id.kind),
        value: best.external_id.value.clone(),
    })
}

async fn supports(plugin: &LoadedPlugin, kind: WorkKind) -> bool {
    if !plugin.exports(HookKind::Metadata) {
        return true;
    }
    match plugin.call_supports(to_wit_kind(kind)).await {
        Ok(yes) => yes,
        Err(error) => {
            tracing::debug!(plugin = %plugin.id.0, %error, "supports() failed; skipping plugin");
            false
        }
    }
}

async fn fetch_one(
    plugin: &LoadedPlugin,
    id: &ExternalId,
) -> Result<MetadataPayload, MetadataError> {
    let wit_id = plugin_types::ExternalId {
        kind: to_wit_kind_id(id.kind),
        value: id.value.clone(),
    };
    match plugin.call_fetch(wit_id).await {
        Ok(Ok(payload)) => Ok(metadata_payload_from_wit(payload)),
        Ok(Err(error)) => Err(MetadataError::Rejected {
            provider: plugin.id.0.clone(),
            error: format!("{error:?}"),
        }),
        Err(error) => Err(MetadataError::Dispatch(error)),
    }
}

/// One image answer, already checked to be a format the image route can
/// serve and resize.
struct FetchedArt {
    bytes: Vec<u8>,
    ext: &'static str,
    language: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

impl FetchedArt {
    /// In the asked-for language (primary subtags compared), text-free,
    /// or nothing was asked for.
    fn matches_language(&self, asked: Option<&str>) -> bool {
        let primary = |tag: &str| {
            tag.split(['-', '_'])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
        };
        match (asked, self.language.as_deref()) {
            (None, _) | (_, None) => true,
            (Some(asked), Some(got)) => primary(asked) == primary(got),
        }
    }
}

async fn fetch_image(
    plugin: &LoadedPlugin,
    id: &ExternalId,
    kind: ImageKind,
    language: Option<String>,
) -> Option<FetchedArt> {
    let wit_id = image_types::ExternalId {
        kind: image_kind_id(id.kind),
        value: id.value.clone(),
    };
    match plugin
        .call_fetch_image(wit_id, wit_image_kind(kind), language)
        .await
    {
        Ok(Ok((meta, bytes))) => {
            let Some(ext) = sniff_image(&bytes) else {
                tracing::debug!(plugin = %plugin.id.0, kind = kind.as_str(), mime = %meta.mime,
                    "provider image is not a servable format; skipped");
                return None;
            };
            Some(FetchedArt {
                bytes,
                ext,
                language: meta.language.filter(|l| !l.is_empty()),
                width: meta.width,
                height: meta.height,
            })
        }
        Ok(Err(error)) => {
            tracing::debug!(plugin = %plugin.id.0, kind = kind.as_str(), ?error, "image fetch refused");
            None
        }
        Err(error) => {
            tracing::debug!(plugin = %plugin.id.0, kind = kind.as_str(), %error, "image fetch failed");
            None
        }
    }
}

/// File extension for bytes the image route can decode, by magic number
/// rather than the plugin's claimed MIME type.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else {
        None
    }
}

#[derive(Debug, thiserror::Error)]
enum StoreError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
}

/// Write `art` to `<root>/provider-art/<work>/<kind>.<ext>` via a unique
/// temp sibling + rename (so a reader never sees a partial file) and
/// point the work's `Provider` row at it. A previous file with another
/// extension is removed once the row moved.
async fn store_art(
    storage: &dyn Storage,
    root: &Path,
    work: WorkId,
    kind: ImageKind,
    art: FetchedArt,
) -> Result<(), StoreError> {
    let dir = root.join(PROVIDER_ART_DIR).join(work.to_string());
    tokio::fs::create_dir_all(&dir).await?;
    let dest = dir.join(format!("{}.{}", kind.as_str(), art.ext));
    let tmp = dir.join(format!(
        ".{}.{}.{}",
        kind.as_str(),
        uuid::Uuid::now_v7(),
        art.ext
    ));
    if let Err(error) = tokio::fs::write(&tmp, &art.bytes).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error.into());
    }
    if let Err(error) = tokio::fs::rename(&tmp, &dest).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error.into());
    }
    let target = ImageTarget::Work(work);
    let previous = storage
        .image(target, kind, &[ImageSource::Provider])
        .await?;
    let path = dest.to_string_lossy().into_owned();
    storage
        .upsert_image(&Image {
            target,
            kind,
            source: ImageSource::Provider,
            path: path.clone(),
            width: art.width,
            height: art.height,
            language: art.language,
        })
        .await?;
    if let Some(previous) = previous
        && previous.path != path
        && Path::new(&previous.path).starts_with(&dir)
    {
        let _ = tokio::fs::remove_file(&previous.path).await;
    }
    Ok(())
}

/// Replace the provider-sourced assignments in each namespace the
/// payload produced drafts for, leaving user-sourced ones untouched.
/// Namespaces with no drafts are not read as "clear" — a provider that
/// omits genres doesn't strip the ones already there.
async fn persist_tags(
    storage: &dyn Storage,
    work: WorkId,
    provider: &str,
    drafts: &[TagDraft],
) -> Result<(), StorageError> {
    let target = TagTarget::Work(work);
    let namespaces: BTreeSet<&str> = drafts.iter().map(|d| d.namespace).collect();
    if namespaces.is_empty() {
        return Ok(());
    }
    let assignments = storage.tag_assignments_for_target(target).await?;
    for namespace in namespaces {
        for assignment in &assignments {
            if assignment.tag.namespace == namespace && assignment.source.starts_with("provider:") {
                storage.unassign_tag(assignment.tag.id, target).await?;
            }
        }
        for draft in drafts.iter().filter(|d| d.namespace == namespace) {
            let slug = slugify(&draft.display);
            let tag = match storage.tag_by_slug(namespace, &slug).await? {
                Some(tag) => tag,
                None => {
                    let tag = Tag {
                        id: TagId::new(),
                        namespace: namespace.to_string(),
                        parent_id: None,
                        slug,
                        display: draft.display.clone(),
                        metadata: Json(draft.metadata.clone()),
                    };
                    storage.upsert_tag(&tag).await?;
                    tag
                }
            };
            storage
                .assign_tag_with_metadata(
                    tag.id,
                    target,
                    1.0,
                    &format!("provider:{provider}"),
                    &draft.assignment,
                )
                .await?;
        }
    }
    Ok(())
}

fn work_kind_filter(kind: WorkKind) -> WorkKindFilter {
    match kind {
        WorkKind::Movie => WorkKindFilter::Movie,
        WorkKind::Series => WorkKindFilter::Series,
        WorkKind::Season => WorkKindFilter::Season,
        WorkKind::Episode => WorkKindFilter::Episode,
        WorkKind::Album => WorkKindFilter::Album,
        WorkKind::Track => WorkKindFilter::Track,
        WorkKind::Artist => WorkKindFilter::Artist,
        WorkKind::AudioBook => WorkKindFilter::AudioBook,
        WorkKind::Photo => WorkKindFilter::Photo,
    }
}

/// `external-id-kind` as spelled in the WIT, for search hints.
fn wit_kind_name(kind: ExternalIdKind) -> &'static str {
    match kind {
        ExternalIdKind::Tmdb => "tmdb",
        ExternalIdKind::Tvdb => "tvdb",
        ExternalIdKind::Imdb => "imdb",
        ExternalIdKind::MusicBrainzReleaseGroup => "music-brainz-release-group",
        ExternalIdKind::MusicBrainzArtist => "music-brainz-artist",
        ExternalIdKind::MusicBrainzRecording => "music-brainz-recording",
        ExternalIdKind::AniDb => "ani-db",
        ExternalIdKind::Custom => "custom",
    }
}

fn image_kind_id(kind: ExternalIdKind) -> image_types::ExternalIdKind {
    use image_types::ExternalIdKind as K;
    match kind {
        ExternalIdKind::Tmdb => K::Tmdb,
        ExternalIdKind::Tvdb => K::Tvdb,
        ExternalIdKind::Imdb => K::Imdb,
        ExternalIdKind::MusicBrainzReleaseGroup => K::MusicBrainzReleaseGroup,
        ExternalIdKind::MusicBrainzArtist => K::MusicBrainzArtist,
        ExternalIdKind::MusicBrainzRecording => K::MusicBrainzRecording,
        ExternalIdKind::AniDb => K::AniDb,
        ExternalIdKind::Custom => K::Custom,
    }
}

fn wit_image_kind(kind: ImageKind) -> image_types::ImageKind {
    use image_types::ImageKind as K;
    match kind {
        ImageKind::Primary => K::Primary,
        ImageKind::Backdrop => K::Backdrop,
        ImageKind::Banner => K::Banner,
        ImageKind::Thumb => K::Thumb,
        ImageKind::Logo => K::Logo,
        ImageKind::Disc => K::Disc,
        ImageKind::Box => K::Box,
        ImageKind::Menu => K::Menu,
        ImageKind::Chapter => K::Chapter,
        ImageKind::Art => K::Art,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(value: &str, confidence: f32) -> plugin_types::MetadataMatch {
        plugin_types::MetadataMatch {
            external_id: plugin_types::ExternalId {
                kind: plugin_types::ExternalIdKind::Tmdb,
                value: value.into(),
            },
            title: value.into(),
            year: None,
            confidence,
        }
    }

    #[test]
    fn confident_match_needs_threshold_and_margin() {
        assert_eq!(
            confident_match(vec![m("a", 0.9), m("b", 0.7)]).map(|i| i.value),
            Some("a".into())
        );
        assert!(
            confident_match(vec![m("a", 0.8)]).is_none(),
            "below threshold"
        );
        assert!(
            confident_match(vec![m("a", 0.95), m("b", 0.93)]).is_none(),
            "too close to call"
        );
        assert_eq!(
            confident_match(vec![m("a", 0.95), m("a", 0.94)]).map(|i| i.value),
            Some("a".into()),
            "duplicates of one id are not a rival"
        );
        assert!(confident_match(vec![]).is_none());
        assert!(confident_match(vec![m("a", f32::NAN)]).is_none());
    }

    #[test]
    fn sniffing_accepts_only_decodable_formats() {
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_image(b"GIF89a"), Some("gif"));
        assert_eq!(sniff_image(b"<svg xmlns="), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn language_match_compares_primary_subtags() {
        let art = |language: Option<&str>| FetchedArt {
            bytes: vec![],
            ext: "jpg",
            language: language.map(str::to_string),
            width: None,
            height: None,
        };
        assert!(art(Some("en")).matches_language(Some("en-US")));
        assert!(art(None).matches_language(Some("de")), "text-free fits any");
        assert!(art(Some("fr")).matches_language(None));
        assert!(!art(Some("fr")).matches_language(Some("de")));
    }
}
