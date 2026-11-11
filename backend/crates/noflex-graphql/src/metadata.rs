// SPDX-License-Identifier: AGPL-3.0-only
//
// Metadata search fan-out plus the metadata-management mutations.
// `MetadataSearch` answers "which provider matches this title?"; the
// mutations answer "apply that provider's payload to this work" —
// refresh (candidate walk), identify (explicit rematch), manual field
// edits, and per-field locks. Applying a payload (fields, tags, artwork)
// is `noflex_plugin_host::MetadataService`, shared with the background
// metadata worker; locks always win over provider payloads and refresh
// replaces only `"provider:*"`-sourced tag assignments.

use std::sync::Arc;

use async_graphql::{Context, InputObject, Object};
use noflex_core::{ExternalId, MetadataStatus, WorkField, WorkId};
pub use noflex_core::{model::WorkKind, providers::MetadataMatch};
use noflex_plugin_host::{
    ArtworkCandidate, MetadataError, MetadataService, PluginRegistry, PriorityTable,
};
pub use noflex_plugin_host::{HookKind, WorkKindFilter};
pub use noflex_storage::domain::ActivityKind;
use noflex_storage::{Storage, domain::Work};
use time::OffsetDateTime;

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn registry<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<PluginRegistry>> {
    ctx.data::<Arc<PluginRegistry>>()
        .map_err(|_| async_graphql::Error::new("metadata providers unavailable"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

async fn load_work(storage: &Arc<dyn Storage>, id: WorkId) -> async_graphql::Result<Work> {
    storage
        .get_work(id)
        .await
        .map_err(gql_err)?
        .ok_or_else(|| async_graphql::Error::new("work not found"))
}

/// The wired service, or one built from the schema's registry, storage
/// and priority table (tests, minimal embeddings) — which applies
/// metadata but fetches no artwork, having no image dir.
fn service(ctx: &Context<'_>) -> async_graphql::Result<Arc<MetadataService>> {
    if let Ok(service) = ctx.data::<Arc<MetadataService>>() {
        return Ok(service.clone());
    }
    let mut service = MetadataService::new(registry(ctx)?.clone(), storage(ctx)?.clone());
    if let Ok(table) = ctx.data::<Arc<tokio::sync::RwLock<PriorityTable>>>() {
        service = service.with_priority_table(table.clone());
    }
    Ok(Arc::new(service))
}

fn metadata_err(e: MetadataError) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

pub struct MetadataSearch {
    registry: Arc<PluginRegistry>,
}

impl MetadataSearch {
    pub fn new(registry: Arc<PluginRegistry>) -> Self {
        Self { registry }
    }

    pub async fn search_metadata(
        &self,
        query: String,
        kind: WorkKind,
        year: Option<i32>,
    ) -> async_graphql::Result<Vec<MetadataMatch>> {
        if query.trim().is_empty() {
            return Err(async_graphql::Error::new("query is required"));
        }
        let sq = noflex_core::providers::SearchQuery {
            title: query,
            year,
            kind,
            hints: vec![],
        };
        let matches = noflex_plugin_host::metadata_fanout(self.registry.as_ref(), &sq).await;
        Ok(matches)
    }
}

/// Manual field edits for a work. Every field left `null` keeps its
/// current value — clearing a field is not supported. Each provided
/// field is also locked against future provider refreshes and flips the
/// work's metadata status to `USER_EDITED`.
#[derive(InputObject)]
pub struct WorkFieldsInput {
    pub title: Option<String>,
    pub sort_title: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    /// ISO 8601 date string (e.g. `1999-03-31`).
    pub release_date: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    /// Out of 10. Locked, it stands in for the rating taken from the work's
    /// rating sources until `COMMUNITY_RATING` is unlocked.
    pub community_rating: Option<f32>,
}

#[derive(Default)]
pub struct MetadataQuery;

#[Object]
impl MetadataQuery {
    /// Every `kind` image the loaded image providers offer for a work, to
    /// pick one by hand: the library's preferred providers first, each
    /// provider's own best first. A provider that fails is left out. Each
    /// candidate can be previewed and picked for half an hour. Only the
    /// kinds a `*_IMAGE` work field locks can be picked. Admin only.
    async fn artwork_candidates(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        kind: noflex_core::ImageKind,
    ) -> async_graphql::Result<Vec<ArtworkCandidate>> {
        crate::access::require_admin(ctx)?;
        service(ctx)?
            .artwork_candidates(work_id, kind)
            .await
            .map_err(metadata_err)
    }
}

#[derive(Default)]
pub struct MetadataMutation;

#[Object]
impl MetadataMutation {
    /// Make one of `artworkCandidates`' images the work's artwork of its
    /// kind: the full-size image is fetched through its provider, credited
    /// to it, and locked against refreshes by the kind's `*_IMAGE` field
    /// until that is unlocked. Admin only.
    async fn set_work_artwork(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        candidate_id: async_graphql::ID,
    ) -> async_graphql::Result<Work> {
        crate::access::require_admin(ctx)?;
        service(ctx)?
            .pick_artwork(work_id, &candidate_id)
            .await
            .map_err(metadata_err)
    }

    /// Re-fetch metadata from the first candidate provider that can serve
    /// one of the work's external ids — the library's preferred providers
    /// when set, else the priority ranking — and refresh its provider
    /// artwork. Locked fields and user-assigned tags survive; a work with
    /// user edits needs `force: true`. Admin only.
    async fn refresh_metadata(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        #[graphql(default)] force: bool,
    ) -> async_graphql::Result<Work> {
        crate::access::require_admin(ctx)?;
        load_work(storage(ctx)?, work_id).await?;
        service(ctx)?
            .refresh(work_id, force)
            .await
            .map_err(metadata_err)
    }

    /// Re-match a work against one provider's id explicitly — the fix
    /// for a wrong auto-match — and fetch that provider's artwork. Skips
    /// the user-edits guard (the user just told us the right match);
    /// locked fields still win. Admin only.
    async fn identify_work(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        provider: String,
        external_id: ExternalId,
    ) -> async_graphql::Result<Work> {
        crate::access::require_admin(ctx)?;
        service(ctx)?
            .identify(work_id, &provider, external_id)
            .await
            .map_err(metadata_err)
    }

    /// Manually edit a work's fields. Each provided field is written
    /// and locked against provider refreshes; the work's metadata
    /// status flips to `USER_EDITED`. Admin only.
    async fn update_work_fields(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        fields: WorkFieldsInput,
    ) -> async_graphql::Result<Work> {
        crate::access::require_admin(ctx)?;
        if fields.title.is_none()
            && fields.sort_title.is_none()
            && fields.original_title.is_none()
            && fields.year.is_none()
            && fields.release_date.is_none()
            && fields.overview.is_none()
            && fields.tagline.is_none()
            && fields.community_rating.is_none()
        {
            return Err(async_graphql::Error::new("no fields provided"));
        }
        if let Some(title) = &fields.title
            && title.trim().is_empty()
        {
            return Err(async_graphql::Error::new("title cannot be empty"));
        }
        if let Some(release_date) = &fields.release_date
            && release_date.trim().is_empty()
        {
            return Err(async_graphql::Error::new("release date cannot be empty"));
        }
        if let Some(rating) = fields.community_rating
            && !(0.0..=10.0).contains(&rating)
        {
            return Err(async_graphql::Error::new("rating must be between 0 and 10"));
        }
        let storage = storage(ctx)?;
        let mut work = load_work(storage, work_id).await?;
        let mut edited = Vec::new();
        if let Some(title) = fields.title {
            work.title = title;
            edited.push(WorkField::Title);
        }
        if let Some(sort_title) = fields.sort_title {
            work.sort_title = sort_title;
            edited.push(WorkField::SortTitle);
        }
        if let Some(original_title) = fields.original_title {
            work.original_title = Some(original_title);
            edited.push(WorkField::OriginalTitle);
        }
        if let Some(year) = fields.year {
            work.year = Some(year);
            edited.push(WorkField::Year);
        }
        if let Some(release_date) = fields.release_date {
            work.release_date = Some(release_date);
            edited.push(WorkField::ReleaseDate);
        }
        if let Some(overview) = fields.overview {
            work.overview = Some(overview);
            edited.push(WorkField::Overview);
        }
        if let Some(tagline) = fields.tagline {
            work.tagline = Some(tagline);
            edited.push(WorkField::Tagline);
        }
        if let Some(community_rating) = fields.community_rating {
            work.community_rating = Some(community_rating);
            edited.push(WorkField::CommunityRating);
        }
        work.locked_fields.extend(edited);
        work.locked_fields.sort();
        work.locked_fields.dedup();
        work.metadata_status = MetadataStatus::UserEdited;
        work.updated_at = OffsetDateTime::now_utc();
        storage.upsert_work(&work).await.map_err(gql_err)?;
        // Vectors are computed from these fields. The next sweep redoes them, and
        // search carries on without one in the meantime.
        if let Err(error) = storage.delete_work_embeddings(work.id).await {
            tracing::warn!(work = %work.id, %error, "stale embedding not cleared");
        }
        Ok(work)
    }

    /// Lock or unlock one field against provider refreshes. Values and
    /// metadata status are untouched, but for `COMMUNITY_RATING`: unlocked,
    /// it's taken from the work's ratings again. Admin only.
    async fn set_work_field_lock(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
        field: WorkField,
        locked: bool,
    ) -> async_graphql::Result<Work> {
        crate::access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let mut work = load_work(storage, work_id).await?;
        if locked {
            work.locked_fields.push(field);
        } else {
            work.locked_fields.retain(|f| *f != field);
        }
        work.locked_fields.sort();
        work.locked_fields.dedup();
        work.updated_at = OffsetDateTime::now_utc();
        storage.upsert_work(&work).await.map_err(gql_err)?;
        // Unlocking the rating has the store take it from the ratings again.
        load_work(storage, work_id).await
    }
}
