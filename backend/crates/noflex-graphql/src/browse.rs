// SPDX-License-Identifier: AGPL-3.0-only
//
// Catalog browse queries. `list_works` with cursor pagination
// already existed in storage; this module is the resolver surface over
// it (plus the single-item and edition/media-file lookups).

use std::sync::Arc;

use async_graphql::{Context, Enum, ID, InputObject, Object, connection::Connection};
use noflex_core::{EditionId, LibraryId, TagId, UserId, WorkId, WorkKind};
use noflex_storage::{
    ListTagsQuery, ListWorksQuery, MatchMode, SortOrder, Storage, TagSummary, WatchedFilter,
    WorkFacet, WorkFacetValues, WorkFacetsQuery, WorkRule, WorkRules,
    domain::{
        ContentCategory, ContentCategoryCount, Edition, Image, ImageTarget, MediaFile, Tag, Work,
    },
};
use time::OffsetDateTime;

use crate::{access, paging, parse_id};

const MAX_WORKS_BY_ID: usize = 60;

#[derive(InputObject, Default)]
pub struct WorkFilterInput {
    /// Restrict to children of this work (e.g. episodes of a season).
    pub parent_id: Option<WorkId>,
    /// Restrict to works carrying this tag.
    pub tag: Option<TagId>,
    pub kinds: Option<Vec<WorkKind>>,
    pub year_min: Option<i32>,
    pub year_max: Option<i32>,
    /// Only works this user has favorited.
    pub favorite_for: Option<UserId>,
    /// Filter by one user's watched state.
    pub watched: Option<WatchedFilterInput>,
    /// Inclusive bounds on community rating; unrated works are
    /// excluded whenever a bound is set.
    pub rating_min: Option<f64>,
    pub rating_max: Option<f64>,
    /// Inclusive bounds on when the work was added to the library.
    pub added_after: Option<OffsetDateTime>,
    pub added_before: Option<OffsetDateTime>,
    /// Conditions on what a work is and how the caller has watched it. Unlike
    /// the fields above, their values mean the same on every server.
    pub rules: Option<WorkRuleGroupInput>,
    /// Include descendant works (episodes under seasons, …); defaults
    /// to true.
    pub recursive: Option<bool>,
}

/// A condition on one facet: `match` of `values` must apply to the work.
/// `workFacets` lists the values worth offering.
#[derive(InputObject)]
pub struct WorkRuleInput {
    pub facet: WorkFacet,
    pub values: Vec<String>,
    /// Defaults to `ANY`.
    #[graphql(name = "match")]
    pub mode: Option<MatchMode>,
}

/// Conditions combined by `match`, which may themselves be groups.
#[derive(InputObject)]
pub struct WorkRuleGroupInput {
    /// Defaults to `ALL`.
    #[graphql(name = "match")]
    pub mode: Option<MatchMode>,
    pub rules: Option<Vec<WorkRuleInput>>,
    pub groups: Option<Vec<WorkRuleGroupInput>>,
}

impl From<WorkRuleGroupInput> for WorkRules {
    fn from(group: WorkRuleGroupInput) -> Self {
        Self {
            mode: group.mode.unwrap_or_default(),
            rules: group
                .rules
                .unwrap_or_default()
                .into_iter()
                .map(|rule| WorkRule {
                    facet: rule.facet,
                    values: rule.values,
                    mode: rule.mode.unwrap_or(MatchMode::Any),
                })
                .collect(),
            groups: group
                .groups
                .unwrap_or_default()
                .into_iter()
                .map(Self::from)
                .collect(),
        }
    }
}

/// Watched-state filter for one user. `watched` defaults to true;
/// pass `watched: false` for works the user has not completed
/// (including never-played).
#[derive(InputObject)]
pub struct WatchedFilterInput {
    /// The user whose played state is consulted.
    pub user_id: UserId,
    pub watched: Option<bool>,
}

/// Closed set of image-target kinds, mirroring [`ImageTarget`]'s
/// variants — kept separate so the query takes a flat
/// `(kind, id)` pair instead of a tagged input union.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ImageTargetKind {
    Work,
    Edition,
    Tag,
}

#[derive(Default)]
pub struct BrowseQuery;

#[Object]
impl BrowseQuery {
    /// The categories holding at least one top-level work the caller can
    /// access, for building navigation.
    async fn content_categories(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<ContentCategoryCount>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let user = access::request_user(ctx)?;
        storage
            .content_category_counts(user.access_user_id())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Works the caller can access, optionally filtered and sorted.
    /// `libraryId` restricts to one library; `category` restricts to one
    /// content category's top-level works across libraries. `first`
    /// defaults to 50, max 200. A `RANDOM` sort is a single page.
    // Each parameter is one GraphQL field argument.
    #[allow(clippy::too_many_arguments)]
    #[graphql(complexity = "paging::cost(first, 50, 200, child_complexity)")]
    async fn works(
        &self,
        ctx: &Context<'_>,
        library_id: Option<LibraryId>,
        category: Option<ContentCategory>,
        filter: Option<WorkFilterInput>,
        sort: Option<SortOrder>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Work>> {
        let window = paging::Window::new(first, after, 50, 200)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let request_user = access::request_user(ctx)?;
        let filter = filter.unwrap_or_default();
        let watched = filter.watched.map(|w| WatchedFilter {
            user_id: w.user_id,
            watched: w.watched.unwrap_or(true),
        });
        if !request_user.is_admin
            && filter
                .favorite_for
                .iter()
                .chain(watched.iter().map(|filter| &filter.user_id))
                .any(|user| *user != request_user.user_id)
        {
            return Err(async_graphql::Error::new("not found"));
        }
        let resumed = window.after.is_some();
        let page = storage
            .list_works(&ListWorksQuery {
                access_user_id: request_user.access_user_id(),
                library: library_id,
                category,
                parent: filter.parent_id,
                tag: filter.tag,
                kinds: filter.kinds.unwrap_or_default(),
                year_min: filter.year_min,
                year_max: filter.year_max,
                favorite_for: filter.favorite_for,
                watched,
                rating_min: filter.rating_min,
                rating_max: filter.rating_max,
                added_after: filter.added_after,
                added_before: filter.added_before,
                rules: filter.rules.map(WorkRules::from),
                viewer: Some(request_user.user_id),
                recursive: filter.recursive.unwrap_or(true),
                sort: sort.unwrap_or_default(),
                limit: window.first,
                cursor: window.after,
            })
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(paging::from_item_page(page, resumed))
    }

    /// The values worth offering as `WorkRuleInput`s among the top-level works
    /// the caller can access, optionally in one library or category. Facets with
    /// nothing to offer are left out. Walks every file in scope, so ask for it
    /// when someone opens the filters rather than with each page.
    async fn work_facets(
        &self,
        ctx: &Context<'_>,
        library_id: Option<LibraryId>,
        category: Option<ContentCategory>,
    ) -> async_graphql::Result<Vec<WorkFacetValues>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let request_user = access::request_user(ctx)?;
        storage
            .work_facets(&WorkFacetsQuery {
                access_user_id: request_user.access_user_id(),
                library: library_id,
                category,
            })
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// A single work by id; null when it doesn't exist.
    async fn work(&self, ctx: &Context<'_>, id: WorkId) -> async_graphql::Result<Option<Work>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let request_user = access::request_user(ctx)?;
        let Some(work) = storage
            .get_work(id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        else {
            return Ok(None);
        };
        access::visible_work(storage, request_user, work).await
    }

    /// The given works the caller can see, in the order asked, for keeping
    /// cards already on screen up to date. At most 60 ids.
    #[graphql(complexity = "ids.len() * child_complexity + 1")]
    async fn works_by_id(
        &self,
        ctx: &Context<'_>,
        ids: Vec<WorkId>,
    ) -> async_graphql::Result<Vec<Work>> {
        if ids.len() > MAX_WORKS_BY_ID {
            return Err(async_graphql::Error::new(format!(
                "at most {MAX_WORKS_BY_ID} ids"
            )));
        }
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let works = storage
            .get_works(&ids)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        crate::visible_works(storage, access::request_user(ctx)?, works, MAX_WORKS_BY_ID).await
    }

    /// Tags across the given namespaces (empty = all), each with its
    /// work count, ordered by display name. `first` defaults to 50, max
    /// 200.
    #[graphql(complexity = "paging::cost(first, 50, 200, child_complexity)")]
    async fn tags(
        &self,
        ctx: &Context<'_>,
        namespaces: Vec<String>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, TagSummary>> {
        let window = paging::Window::new(first, after, 50, 200)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::request_user(ctx)?;
        let resumed = window.after.is_some();
        let page = storage
            .list_tags(&ListTagsQuery {
                namespaces,
                limit: window.first,
                cursor: window.after,
            })
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(paging::from_item_page(page, resumed))
    }

    /// A single tag by id; null when it doesn't exist. Its `works`
    /// field lists the works carrying it (a person's filmography).
    async fn tag(&self, ctx: &Context<'_>, id: TagId) -> async_graphql::Result<Option<Tag>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::request_user(ctx)?;
        storage
            .get_tag(id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Every edition (cut/version) of a work.
    async fn editions(
        &self,
        ctx: &Context<'_>,
        work_id: WorkId,
    ) -> async_graphql::Result<Vec<Edition>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let request_user = access::request_user(ctx)?;
        if !access::can_work(storage, request_user, work_id).await? {
            return Ok(Vec::new());
        }
        storage
            .editions_for_work(work_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Every physical file backing an edition (multi-part media has
    /// more than one).
    async fn media_files(
        &self,
        ctx: &Context<'_>,
        edition_id: EditionId,
    ) -> async_graphql::Result<Vec<MediaFile>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let request_user = access::request_user(ctx)?;
        if !access::can_edition(storage, request_user, edition_id).await? {
            return Ok(Vec::new());
        }
        storage
            .media_files_for_edition(edition_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// The effective image of each kind attached to a work/edition/tag
    /// target, under the owning library's artwork preference. Each
    /// image's `url` is the serving route; append `&w=`/`&h=` to resize.
    async fn images(
        &self,
        ctx: &Context<'_>,
        target_kind: ImageTargetKind,
        target_id: ID,
    ) -> async_graphql::Result<Vec<Image>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let request_user = access::request_user(ctx)?;
        let uuid: uuid::Uuid = parse_id(&target_id)?;
        let target = match target_kind {
            ImageTargetKind::Work => {
                let id = WorkId::from_uuid(uuid);
                if !access::can_work(storage, request_user, id).await? {
                    return Ok(Vec::new());
                }
                ImageTarget::Work(id)
            }
            ImageTargetKind::Edition => {
                let id = EditionId::from_uuid(uuid);
                if !access::can_edition(storage, request_user, id).await? {
                    return Ok(Vec::new());
                }
                ImageTarget::Edition(id)
            }
            ImageTargetKind::Tag => ImageTarget::Tag(TagId::from_uuid(uuid)),
        };
        let order = target
            .source_order(storage.as_ref())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        storage
            .list_images(target, order)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}
