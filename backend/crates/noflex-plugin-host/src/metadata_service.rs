// SPDX-License-Identifier: AGPL-3.0-only
//
// Host-side metadata application: choosing provider candidates for a
// work, applying a provider payload (fields, tags, external ids, its
// ratings), pulling the matching artwork through the winning provider's
// `image` hook, asking every ratings plugin for its ratings, and
// matching unidentified works automatically. GraphQL's refresh/identify
// mutations and the background metadata worker both drive this, so the
// rules live in one place.
//
// Plugins never see storage: they get external ids and hints in, and
// hand payloads and image bytes back; every write happens here. All
// egress stays inside plugins — the host never downloads a URL itself,
// payload image URLs only tell it which kinds a provider has.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_graphql::Json;
use noflex_core::{
    ExternalId, ExternalIdKind, ImageKind, MetadataStatus, TagId, TagTarget, WorkField, WorkId,
    WorkKind,
    providers::{MetadataPayload, Rating},
    slugify,
};
use noflex_metadata::{TagDraft, apply_payload, merge_external_id, merge_tag_metadata, person_ids};
use noflex_storage::{
    IndexingProgress, Storage, StorageError,
    domain::{Image, ImageSource, ImageTarget, Library, Tag, Work},
};
use time::OffsetDateTime;
use tokio::sync::RwLock;

use crate::{
    HookKind, HostError, LoadedPlugin, PluginId, PluginRegistry, PriorityTable, WorkKindFilter,
    bindings::ratings::noflex::plugin::types as ratings_types,
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
/// How long an offered artwork candidate can be previewed and picked.
const OFFER_TTL: Duration = Duration::from_secs(30 * 60);
/// Most candidates remembered at once; the oldest go first.
const MAX_OFFERS: usize = 4096;
/// People whose photos one payload fetches at most, the first billed.
const MAX_PHOTOS: usize = 20;

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
    #[error("{0} artwork can't be picked")]
    UnpickableKind(&'static str),
    #[error("that artwork is no longer on offer; list the candidates again")]
    UnknownCandidate,
    #[error("the provider's image isn't in a format NoFlex can show")]
    UnservableImage,
    #[error("this server keeps no provider artwork")]
    NoImageDir,
    #[error("artwork not stored: {0}")]
    Store(String),
}

/// One image a provider offers for a work, registered under `id` so a
/// client can preview and pick it without ever naming the provider's
/// handle: only what was offered can be fetched.
#[derive(Debug, Clone, PartialEq, async_graphql::SimpleObject)]
#[graphql(complex)]
pub struct ArtworkCandidate {
    /// What `setWorkArtwork` takes. It expires half an hour after it was
    /// offered.
    pub id: async_graphql::ID,
    /// The plugin offering it, as `providerAttributions` names it.
    pub provider: String,
    pub kind: ImageKind,
    /// ISO 639-1 code of the text on it; null when text-free.
    pub language: Option<String>,
    /// The full-size image's, when the provider says.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The provider's own rating, higher is better; comparable only between
    /// one provider's candidates.
    pub score: Option<f32>,
}

#[async_graphql::ComplexObject]
impl ArtworkCandidate {
    /// A small rendition for a picker, fetched through its provider,
    /// relative to this server. Carries a `token` so `<img src>` works.
    async fn preview_url(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<String> {
        let user = noflex_storage::access::request_user(ctx)?;
        let path = format!("/artwork-candidates/{}/preview", self.id.as_str());
        let Some(signer) = ctx.data_opt::<Arc<dyn noflex_storage::MediaUrlSigner>>() else {
            return Ok(path);
        };
        let token = signer
            .sign_artwork_candidate_token(
                user.user_id,
                noflex_storage::access::caller_session(ctx),
                self.id.as_str(),
            )
            .map_err(async_graphql::Error::new)?;
        Ok(format!(
            "{path}?token={}",
            noflex_storage::domain::percent_encode(&token)
        ))
    }
}

/// Image bytes in a format the image routes serve.
#[derive(Debug, Clone)]
pub struct ServableImage {
    pub bytes: Vec<u8>,
    /// `image/jpeg`, `image/png`, `image/webp` or `image/gif`.
    pub mime: &'static str,
}

/// A candidate between being offered and being picked.
#[derive(Debug, Clone)]
struct Offer {
    work: WorkId,
    kind: ImageKind,
    provider: PluginId,
    handle: String,
    expires: Instant,
}

/// What came of asking one ratings plugin about one work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingsFetch {
    /// Its answer, empty or not, is the work's ratings from it now.
    Stored,
    /// It can't answer anything for now (rate-limited, unavailable, its
    /// queue full); asking about another work won't go better.
    Unavailable,
    /// It failed for this work alone, or its answer couldn't be stored.
    Failed,
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
    indexing: Arc<IndexingProgress>,
    /// Artwork candidates offered by [`Self::artwork_candidates`], by id.
    offers: Mutex<HashMap<String, Offer>>,
    /// One payload's people at a time fetch their photos, so identifying a
    /// whole library doesn't queue thousands of image calls at once.
    photos: Arc<tokio::sync::Semaphore>,
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
            indexing: Arc::default(),
            offers: Mutex::default(),
            photos: Arc::new(tokio::sync::Semaphore::new(1)),
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

    /// What the worker is getting through, for `Work.indexing`.
    pub fn indexing(&self) -> &Arc<IndexingProgress> {
        &self.indexing
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

    /// Fetch photos for the first [`MAX_PHOTOS`] of `people` from
    /// `provider`, the plugin that credited them, in the background: the
    /// caller doesn't wait, and nothing it does fails because of it. A
    /// no-op without an image dir, or when `provider` exports no `image`.
    fn fetch_photos_later(&self, provider: &PluginId, mut people: Vec<Credited>) {
        let Some(root) = self.image_dir.clone() else {
            return;
        };
        people.truncate(MAX_PHOTOS);
        if people.is_empty() {
            return;
        }
        let registry = Arc::clone(&self.registry);
        let storage = Arc::clone(&self.storage);
        let permits = Arc::clone(&self.photos);
        let provider = provider.clone();
        tokio::spawn(async move {
            let Ok(_permit) = permits.acquire_owned().await else {
                return;
            };
            let Some(plugin) = registry.get_plugin(&provider) else {
                return;
            };
            if plugin.exports(HookKind::Image) {
                fetch_photos(&plugin, storage.as_ref(), &root, &people).await;
            }
        });
    }

    /// Persist `payload` onto `work` from `provider`: fields (locks win),
    /// provider-sourced tags, external ids and `provider`'s ratings, then
    /// every ratings plugin's ([`Self::fetch_ratings`]), then artwork, with
    /// the credited people's photos following in the background. Ratings and
    /// artwork failures are logged, never returned — the metadata is
    /// already saved. Returns the work as stored.
    async fn apply(
        &self,
        mut work: Work,
        provider: &PluginId,
        payload: &MetadataPayload,
        library: Option<&Library>,
    ) -> Result<Work, MetadataError> {
        let report = apply_payload(&mut work, payload);
        work.metadata_provider = Some(provider.0.clone());
        let people =
            persist_tags(self.storage.as_ref(), work.id, &provider.0, &report.tags).await?;
        self.fetch_photos_later(provider, people);
        work.updated_at = OffsetDateTime::now_utc();
        self.storage.upsert_work(&work).await?;
        // Vectors are computed from this metadata, and a work is usually
        // embedded on being added, before it has any. The next sweep redoes it.
        if let Err(error) = self.storage.delete_work_embeddings(work.id).await {
            tracing::warn!(work = %work.id, %error, "metadata: stale embedding not cleared");
        }
        let ratings = usable_ratings(&provider.0, &payload.ratings);
        if let Err(error) = self
            .storage
            .replace_work_ratings(work.id, &provider.0, &ratings)
            .await
        {
            tracing::warn!(work = %work.id, plugin = %provider.0, %error, "metadata: ratings not stored");
        }
        self.fetch_ratings(&work).await;

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
        // The store derived its rating from what was stored above.
        Ok(self.storage.get_work(work.id).await?.unwrap_or(work))
    }

    /// Ask every loaded ratings plugin about `work` at once, through its
    /// provider ids, and store each answer as that plugin's ratings of it.
    /// One plugin failing changes nothing of its own or anyone else's; every
    /// failure is logged. A no-op without provider ids.
    pub async fn fetch_ratings(&self, work: &Work) {
        let plugins = self.ratings_plugins();
        futures::future::join_all(plugins.iter().map(|plugin| self.rate(plugin, work))).await;
    }

    /// Every loaded plugin exporting `ratings`, by id.
    pub fn ratings_plugins(&self) -> Vec<Arc<LoadedPlugin>> {
        let mut ids = self.registry.exporters_of(HookKind::Ratings);
        ids.sort();
        ids.iter()
            .filter_map(|id| self.registry.get_plugin(id))
            .collect()
    }

    /// Ask `plugin` alone about `work` and store its answer, as
    /// [`Self::fetch_ratings`] does for each.
    pub async fn rate(&self, plugin: &LoadedPlugin, work: &Work) -> RatingsFetch {
        let ids: Vec<ratings_types::ExternalId> = provider_ids(work)
            .map(|id| ratings_types::ExternalId {
                kind: ratings_kind_id(id.kind),
                value: id.value.clone(),
            })
            .collect();
        if ids.is_empty() {
            return RatingsFetch::Failed;
        }
        let answer = match plugin.call_ratings(ids, ratings_work_kind(work.kind)).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(
                error @ (ratings_types::PluginError::RateLimited(_)
                | ratings_types::PluginError::Unavailable(_)),
            )) => {
                tracing::debug!(work = %work.id, plugin = %plugin.id.0, ?error, "ratings: plugin can't answer");
                return RatingsFetch::Unavailable;
            }
            Ok(Err(error)) => {
                tracing::debug!(work = %work.id, plugin = %plugin.id.0, ?error, "ratings: refused");
                return RatingsFetch::Failed;
            }
            Err(error @ HostError::Overloaded { .. }) => {
                tracing::debug!(work = %work.id, plugin = %plugin.id.0, %error, "ratings: plugin busy");
                return RatingsFetch::Unavailable;
            }
            Err(error) => {
                tracing::warn!(work = %work.id, plugin = %plugin.id.0, %error, "ratings: dispatch failed");
                return RatingsFetch::Failed;
            }
        };
        let answer: Vec<Rating> = answer
            .into_iter()
            .map(|r| Rating {
                source: r.source,
                value: r.value,
                scale: r.scale,
                votes: r.votes,
            })
            .collect();
        let ratings = usable_ratings(&plugin.id.0, &answer);
        match self
            .storage
            .replace_work_ratings(work.id, &plugin.id.0, &ratings)
            .await
        {
            Ok(()) => RatingsFetch::Stored,
            Err(error) => {
                tracing::warn!(work = %work.id, plugin = %plugin.id.0, %error, "ratings: not stored");
                RatingsFetch::Failed
            }
        }
    }

    /// Fetch each of `kinds` for `work` through the `image` hook and record
    /// it as the work's `Provider` image, replacing the previous one. Tries
    /// `winner` first (when it exports `image`), then the other image
    /// candidates; within a plugin, each of the work's non-`Custom`
    /// external ids. The language asked for is the library's preferred
    /// languages in order, else the payload's, else none. A kind an
    /// administrator picked by hand is left alone. Returns how many images
    /// were written; every failure is logged and skipped. A no-op without
    /// an image dir or provider ids.
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
        for (kind, payload_language) in kinds.iter().filter(|(kind, _)| !picked(work, *kind)) {
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
            match store_art(
                self.storage.as_ref(),
                root,
                ImageTarget::Work(work.id),
                *kind,
                fetched,
            )
            .await
            {
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
            if keep.iter().any(|(k, _)| *k == row.kind) || picked(work, row.kind) {
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

    /// Every `kind` image the loaded image providers offer for `id`, for an
    /// administrator to pick from. Each provider is asked through the first
    /// of the work's provider ids it answers for, all at once; one that
    /// fails is left out. Providers are ordered by the library's preferred
    /// providers, then by image priority, and each one's images keep its
    /// own order. Every candidate is remembered under its `id` for
    /// [`OFFER_TTL`], which is all [`Self::candidate_preview`] and
    /// [`Self::pick_artwork`] accept.
    ///
    /// # Errors
    /// [`MetadataError::WorkNotFound`]; [`MetadataError::UnpickableKind`]
    /// for a kind [`WorkField::image`] has no lock for.
    pub async fn artwork_candidates(
        &self,
        id: WorkId,
        kind: ImageKind,
    ) -> Result<Vec<ArtworkCandidate>, MetadataError> {
        if WorkField::image(kind).is_none() {
            return Err(MetadataError::UnpickableKind(kind.as_str()));
        }
        let (work, library) = self.load(id).await?;
        let ids: Vec<&ExternalId> = provider_ids(&work).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let plugins = self.image_providers(work.kind, library.as_ref()).await;
        let wit_kind = wit_image_kind(kind);
        let lists = plugins.iter().map(|plugin| {
            let ids = &ids;
            async move {
                for external_id in ids {
                    let wit_id = image_types::ExternalId {
                        kind: image_kind_id(external_id.kind),
                        value: external_id.value.clone(),
                    };
                    match plugin.call_list_candidates(wit_id, wit_kind).await {
                        Ok(Ok(list)) => return list,
                        Ok(Err(error)) => {
                            tracing::debug!(plugin = %plugin.id.0, ?error, "artwork candidates refused");
                        }
                        Err(error) => {
                            tracing::debug!(plugin = %plugin.id.0, %error, "artwork candidates failed");
                            return Vec::new();
                        }
                    }
                }
                Vec::new()
            }
        });
        let lists = futures::future::join_all(lists).await;

        let expires = Instant::now() + OFFER_TTL;
        let mut out = Vec::new();
        let mut offers = Vec::new();
        for (plugin, list) in plugins.iter().zip(lists) {
            let mut seen = HashSet::new();
            for candidate in list {
                // A provider may answer for several of the work's ids with one record.
                if candidate.kind != wit_kind || !seen.insert(candidate.handle.clone()) {
                    continue;
                }
                let id = uuid::Uuid::now_v7().simple().to_string();
                offers.push((
                    id.clone(),
                    Offer {
                        work: work.id,
                        kind,
                        provider: plugin.id.clone(),
                        handle: candidate.handle,
                        expires,
                    },
                ));
                out.push(ArtworkCandidate {
                    id: id.into(),
                    provider: plugin.id.0.clone(),
                    kind,
                    language: candidate.language.filter(|l| !l.is_empty()),
                    width: candidate.width,
                    height: candidate.height,
                    score: candidate.score.filter(|s| s.is_finite()),
                });
            }
        }
        self.remember(offers);
        Ok(out)
    }

    /// The small rendition of the offered candidate `id`, from its provider.
    ///
    /// # Errors
    /// [`MetadataError::UnknownCandidate`] for an id not on offer;
    /// [`MetadataError::ProviderNotLoaded`], [`MetadataError::Rejected`],
    /// [`MetadataError::Dispatch`] or [`MetadataError::UnservableImage`]
    /// when the provider can't deliver it.
    pub async fn candidate_preview(&self, id: &str) -> Result<ServableImage, MetadataError> {
        let offer = self.offer(id).ok_or(MetadataError::UnknownCandidate)?;
        let art = self.fetch_offer(&offer, true).await?;
        Ok(ServableImage {
            mime: mime_of(art.ext),
            bytes: art.bytes,
        })
    }

    /// Make the offered candidate `candidate` `work`'s provider image of its
    /// kind: the full-size image is fetched through its provider, stored and
    /// credited to it, and the kind's [`WorkField::image`] lock keeps
    /// refreshes from replacing it until it is unlocked.
    ///
    /// # Errors
    /// [`MetadataError::UnknownCandidate`] for an id not on offer for this
    /// work; [`MetadataError::NoImageDir`] without an image dir; the
    /// provider's failures as for [`Self::candidate_preview`].
    pub async fn pick_artwork(&self, work: WorkId, candidate: &str) -> Result<Work, MetadataError> {
        let offer = self
            .offer(candidate)
            .filter(|offer| offer.work == work)
            .ok_or(MetadataError::UnknownCandidate)?;
        let root = self.image_dir.as_ref().ok_or(MetadataError::NoImageDir)?;
        let art = self.fetch_offer(&offer, false).await?;
        store_art(
            self.storage.as_ref(),
            root,
            ImageTarget::Work(work),
            offer.kind,
            art,
        )
        .await
        .map_err(|error| MetadataError::Store(error.to_string()))?;
        let (mut work, _) = self.load(work).await?;
        if let Some(field) = WorkField::image(offer.kind)
            && !work.locked_fields.contains(&field)
        {
            work.locked_fields.push(field);
            work.locked_fields.sort();
            work.updated_at = OffsetDateTime::now_utc();
            self.storage.upsert_work(&work).await?;
        }
        Ok(work)
    }

    /// Every loaded image provider, the library's preferred ones first,
    /// then by image priority, then by id.
    async fn image_providers(
        &self,
        kind: WorkKind,
        library: Option<&Library>,
    ) -> Vec<Arc<LoadedPlugin>> {
        let preferred = library
            .map(|l| l.config.preferred_providers.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|id| PluginId(id.clone()));
        let ranked = self.default_candidates(HookKind::Image, kind).await;
        let mut rest = self.registry.exporters_of(HookKind::Image);
        rest.sort();
        let mut out: Vec<Arc<LoadedPlugin>> = Vec::new();
        for id in preferred.chain(ranked).chain(rest) {
            if out.iter().any(|p| p.id == id) {
                continue;
            }
            if let Some(plugin) = self.registry.get_plugin(&id)
                && plugin.exports(HookKind::Image)
            {
                out.push(plugin);
            }
        }
        out
    }

    fn remember(&self, offered: Vec<(String, Offer)>) {
        let now = Instant::now();
        let mut offers = self.offers.lock().expect("artwork offers poisoned");
        offers.retain(|_, offer| offer.expires > now);
        offers.extend(offered);
        if offers.len() > MAX_OFFERS {
            let mut by_age: Vec<(Instant, String)> = offers
                .iter()
                .map(|(id, offer)| (offer.expires, id.clone()))
                .collect();
            by_age.sort();
            for (_, id) in by_age.into_iter().take(offers.len() - MAX_OFFERS) {
                offers.remove(&id);
            }
        }
    }

    fn offer(&self, id: &str) -> Option<Offer> {
        let offers = self.offers.lock().expect("artwork offers poisoned");
        offers
            .get(id)
            .filter(|offer| offer.expires > Instant::now())
            .cloned()
    }

    async fn fetch_offer(&self, offer: &Offer, preview: bool) -> Result<FetchedArt, MetadataError> {
        let plugin = self
            .registry
            .get_plugin(&offer.provider)
            .ok_or_else(|| MetadataError::ProviderNotLoaded(offer.provider.0.clone()))?;
        let (meta, bytes) = match plugin
            .call_fetch_candidate(offer.handle.clone(), preview)
            .await
        {
            Ok(Ok(fetched)) => fetched,
            Ok(Err(error)) => {
                return Err(MetadataError::Rejected {
                    provider: offer.provider.0.clone(),
                    error: format!("{error:?}"),
                });
            }
            Err(error) => return Err(MetadataError::Dispatch(error)),
        };
        let ext = sniff_image(&bytes).ok_or(MetadataError::UnservableImage)?;
        Ok(FetchedArt {
            provider: offer.provider.0.clone(),
            bytes,
            ext,
            language: meta.language.filter(|l| !l.is_empty()),
            width: meta.width,
            height: meta.height,
        })
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
        if !work.awaits_identification() {
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
            if !fresh.awaits_identification() {
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

/// `ratings` as `provider` may store them: the first of each source, and
/// only those [`Rating::is_valid`] passes. The rest are logged and dropped.
pub(crate) fn usable_ratings(provider: &str, ratings: &[Rating]) -> Vec<Rating> {
    let mut out: Vec<Rating> = Vec::with_capacity(ratings.len());
    for rating in ratings {
        if !rating.is_valid() {
            tracing::warn!(plugin = %provider, source = %rating.source, value = rating.value,
                scale = rating.scale, "rating dropped: not a slug, or not within its scale");
        } else if !out.iter().any(|kept| kept.source == rating.source) {
            out.push(rating.clone());
        }
    }
    out
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
    provider: String,
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
                provider: plugin.id.0.clone(),
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

/// Whether an administrator picked `work`'s `kind` image, which refreshes
/// leave alone.
fn picked(work: &Work, kind: ImageKind) -> bool {
    WorkField::image(kind).is_some_and(|field| work.locked_fields.contains(&field))
}

/// The content type of an extension [`sniff_image`] answers.
fn mime_of(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

/// File extension for bytes the image route can decode, by magic number
/// rather than the plugin's claimed MIME type.
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
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

/// Write `art` to `<root>/provider-art/<target id>/<kind>-<unique>.<ext>`
/// via a temp sibling + rename (so a reader never sees a partial file) and
/// point the target's `Provider` row at it. Every write gets a new name, so
/// the image's URL changes with it; the previous file is removed once the
/// row moved.
async fn store_art(
    storage: &dyn Storage,
    root: &Path,
    target: ImageTarget,
    kind: ImageKind,
    art: FetchedArt,
) -> Result<(), StoreError> {
    let dir = root.join(PROVIDER_ART_DIR).join(target.id().to_string());
    tokio::fs::create_dir_all(&dir).await?;
    let unique = uuid::Uuid::now_v7().simple();
    let dest = dir.join(format!("{}-{unique}.{}", kind.as_str(), art.ext));
    let tmp = dir.join(format!(".{}-{unique}.{}", kind.as_str(), art.ext));
    if let Err(error) = tokio::fs::write(&tmp, &art.bytes).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error.into());
    }
    if let Err(error) = tokio::fs::rename(&tmp, &dest).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error.into());
    }
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
            provider: Some(art.provider),
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
/// omits genres doesn't strip the ones already there. A tag that exists
/// takes in the draft's metadata ([`merge_tag_metadata`]). Returns the
/// people `provider` gave ids for, in billing order.
async fn persist_tags(
    storage: &dyn Storage,
    work: WorkId,
    provider: &str,
    drafts: &[TagDraft],
) -> Result<Vec<Credited>, StorageError> {
    let target = TagTarget::Work(work);
    let namespaces: BTreeSet<&str> = drafts.iter().map(|d| d.namespace).collect();
    let mut people = Vec::new();
    if namespaces.is_empty() {
        return Ok(people);
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
                Some(mut tag) => {
                    if let Some(merged) = merge_tag_metadata(&tag.metadata.0, &draft.metadata) {
                        tag.metadata = Json(merged);
                        storage.upsert_tag(&tag).await?;
                    }
                    tag
                }
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
            let ids = person_ids(&draft.metadata);
            if namespace == "person" && !ids.is_empty() {
                people.push(Credited { tag: tag.id, ids });
            }
        }
    }
    Ok(people)
}

/// A person a payload credited, with the ids its provider knows them by.
struct Credited {
    tag: TagId,
    ids: Vec<ExternalId>,
}

/// Store a photo for each of `people` without a provider one, through
/// `plugin`'s image hook. A person it has no photo of is skipped; failures
/// are logged and never returned.
async fn fetch_photos(
    plugin: &LoadedPlugin,
    storage: &dyn Storage,
    root: &Path,
    people: &[Credited],
) {
    for person in people {
        let target = ImageTarget::Tag(person.tag);
        match storage
            .image(target, ImageKind::Primary, &[ImageSource::Provider])
            .await
        {
            Ok(None) => {}
            Ok(Some(_)) => continue,
            Err(error) => {
                tracing::warn!(tag = %person.tag, %error, "person photo lookup failed");
                continue;
            }
        }
        for id in &person.ids {
            let Some(art) = fetch_image(plugin, id, ImageKind::Primary, None).await else {
                continue;
            };
            if let Err(error) = store_art(storage, root, target, ImageKind::Primary, art).await {
                tracing::warn!(tag = %person.tag, %error, "person photo not stored");
            }
            break;
        }
    }
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

fn ratings_kind_id(kind: ExternalIdKind) -> ratings_types::ExternalIdKind {
    use ratings_types::ExternalIdKind as K;
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

fn ratings_work_kind(kind: WorkKind) -> ratings_types::WorkKind {
    use ratings_types::WorkKind as K;
    match kind {
        WorkKind::Movie => K::Movie,
        WorkKind::Series => K::Series,
        WorkKind::Season => K::Season,
        WorkKind::Episode => K::Episode,
        WorkKind::Album => K::Album,
        WorkKind::Track => K::Track,
        WorkKind::Artist => K::Artist,
        WorkKind::AudioBook => K::AudioBook,
        WorkKind::Photo => K::Photo,
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
            provider: "p".into(),
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
