// SPDX-License-Identifier: AGPL-3.0-only

use async_graphql::{Enum, SimpleObject};
use noflex_core::{LibraryId, TagId, UserId, WorkId, WorkKind};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Opaque pagination cursor. Backends encode whatever they need (last seen ID,
/// last sort-key value, offset, etc.) inside it. Callers should treat as
/// opaque base64 / hex strings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageCursor(pub String);

/// Filter for play-history reads. Every set field narrows the result;
/// `since`/`until` bound `started_at` (inclusive / exclusive).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaySessionQuery {
    pub user: Option<UserId>,
    pub library: Option<LibraryId>,
    pub root_work: Option<WorkId>,
    pub since: Option<OffsetDateTime>,
    pub until: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<PageCursor>,
}

/// A keyset page whose every item carries its own resume cursor — the
/// shape client-facing lists return, since each GraphQL connection edge
/// needs one. Passing `item_cursors[i]` back resumes just after
/// `items[i]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemPage<T> {
    pub items: Vec<T>,
    /// Cursor for each item, in the same order as `items`.
    pub item_cursors: Vec<PageCursor>,
    pub end_cursor: Option<PageCursor>,
    pub has_next: bool,
}

impl<T> ItemPage<T> {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            item_cursors: Vec::new(),
            end_cursor: None,
            has_next: false,
        }
    }

    /// Build a page from rows fetched in page order with one look-ahead
    /// row past `limit`: the extra row only sets `has_next` and is
    /// dropped before `cursor_of` mints each kept row's cursor.
    pub fn from_lookahead(
        rows: Vec<T>,
        limit: u32,
        mut cursor_of: impl FnMut(&T) -> PageCursor,
    ) -> Self {
        match Self::try_from_lookahead(rows, limit, |row| {
            Ok::<_, std::convert::Infallible>(cursor_of(row))
        }) {
            Ok(page) => page,
            Err(never) => match never {},
        }
    }

    /// [`ItemPage::from_lookahead`] with a fallible cursor encoder; the
    /// first encoding error is returned.
    pub fn try_from_lookahead<E>(
        mut rows: Vec<T>,
        limit: u32,
        cursor_of: impl FnMut(&T) -> Result<PageCursor, E>,
    ) -> Result<Self, E> {
        let has_next = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let item_cursors = rows.iter().map(cursor_of).collect::<Result<Vec<_>, E>>()?;
        Ok(Self {
            end_cursor: item_cursors.last().cloned(),
            items: rows,
            item_cursors,
            has_next,
        })
    }

    /// Transform every item, keeping cursors and paging state.
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> ItemPage<U> {
        ItemPage {
            items: self.items.into_iter().map(f).collect(),
            item_cursors: self.item_cursors,
            end_cursor: self.end_cursor,
            has_next: self.has_next,
        }
    }

    /// The cursor for the following page; `None` on the last page.
    pub fn next_cursor(&self) -> Option<PageCursor> {
        self.end_cursor.clone().filter(|_| self.has_next)
    }
}

pub const ITEM_PAGE_DEFAULT: u32 = 50;
pub const ITEM_PAGE_MAX: u32 = 100;

impl<T> Page<T> {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Enum)]
pub enum SortOrder {
    #[default]
    DateAddedDesc,
    DateAddedAsc,
    TitleAsc,
    TitleDesc,
    YearDesc,
    YearAsc,
    DateReleasedDesc,
    Random,
}

/// Per-user watched-state filter. `watched = false` selects works the
/// user has NOT completed: that includes never-played works (no
/// `user_data` row at all) and works carrying only saved progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedFilter {
    pub user_id: UserId,
    pub watched: bool,
}

/// Something a work can be filtered by. Values are portable between servers
/// (slugs, titles, codec names), never server-local ids.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Enum,
)]
pub enum WorkFacet {
    /// Words the title contains, ignoring case.
    Title,
    /// A `genre` tag's slug.
    Genre,
    /// A `mood` tag's slug.
    Mood,
    /// A `studio` tag's slug.
    Studio,
    /// A `parental` tag's slug.
    ContentRating,
    /// A `person` tag's slug, credited as director.
    Director,
    /// A `person` tag's slug, credited as actor.
    Actor,
    /// A `person` tag's slug, credited as writer.
    Writer,
    /// A `person` tag's slug, credited as producer.
    Producer,
    /// A collection's title, ignoring case.
    Collection,
    /// An edition's name, like "Director's Cut".
    Edition,
    /// `YYYY`, `YYYY-YYYY`, `YYYY-` or `-YYYY`, inclusive.
    Years,
    /// A community rating floor; unrated works never match.
    MinRating,
    /// Added to the library within this many days.
    AddedWithinDays,
    /// `unwatched`, `in_progress` or `watched`, for the query's viewer. A series
    /// is watched once every episode is, and in progress once any has been started.
    WatchState,
    /// Favorited by the query's viewer. The value is ignored.
    Favorite,
    /// `unmatched` for works no provider has confirmed, or `matched`.
    MatchStatus,
    /// `8k`, `4k`, `1440p`, `1080p`, `720p` or `sd`.
    Resolution,
    /// `dolby_vision`, `hdr10`, `hlg` or `sdr`. HDR10+ isn't probed, so it reads as `hdr10`.
    DynamicRange,
    /// A codec name as probed, like `h265`.
    VideoCodec,
    /// A codec name as probed, like `eac3`.
    AudioCodec,
    /// `atmos`, `7.1`, `5.1`, `stereo` or `mono`, or a channel count for anything else.
    AudioLayout,
    /// Any spelling of a language code.
    AudioLanguage,
    /// A codec name as probed, or as the sidecar's extension gave it.
    SubtitleCodec,
    /// Any spelling of a language code. Sidecar files count.
    SubtitleLanguage,
}

/// How several things combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Enum)]
pub enum MatchMode {
    #[default]
    All,
    Any,
    None,
}

/// One facet's condition: a work matches when `mode` of `values` apply to it.
/// Media facets apply when any file of the work or its descendants has the value.
/// `values` being empty means the rule says nothing, and it's skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkRule {
    pub facet: WorkFacet,
    pub values: Vec<String>,
    pub mode: MatchMode,
}

/// Boolean logic over [`WorkRule`]s: a work matches when `mode` of `rules` and
/// `groups` do. A group with nothing in it says nothing, and it's skipped.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkRules {
    pub mode: MatchMode,
    pub rules: Vec<WorkRule>,
    pub groups: Vec<WorkRules>,
}

/// Which works' values [`crate::Storage::work_facets`] reports.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkFacetsQuery {
    /// Apply library grants and parental restrictions for this user.
    /// Administrators leave this unset.
    pub access_user_id: Option<UserId>,
    pub library: Option<LibraryId>,
    pub category: Option<crate::domain::ContentCategory>,
}

/// A value a facet can take among some works.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SimpleObject)]
pub struct WorkFacetValue {
    /// What to send back in a [`WorkRule`].
    pub value: String,
    /// The display name where the server knows one, otherwise `value`.
    pub label: String,
    /// Top-level works with this value.
    pub work_count: u32,
}

/// The values one facet takes among some works, most common first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SimpleObject)]
pub struct WorkFacetValues {
    pub facet: WorkFacet,
    pub values: Vec<WorkFacetValue>,
    /// More values exist than the most common ones listed here.
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListWorksQuery {
    /// Apply library grants and parental restrictions for this user.
    /// Administrators leave this unset.
    pub access_user_id: Option<UserId>,
    pub library: Option<LibraryId>,
    /// Restrict to this category's top-level works, across libraries.
    pub category: Option<crate::domain::ContentCategory>,
    pub parent: Option<WorkId>,
    /// Restrict to works carrying this tag.
    pub tag: Option<TagId>,
    pub kinds: Vec<WorkKind>,
    pub year_min: Option<i32>,
    pub year_max: Option<i32>,
    pub favorite_for: Option<UserId>,
    pub watched: Option<WatchedFilter>,
    /// Inclusive bounds on `Work.community_rating`. Works without a
    /// rating are excluded whenever a bound is set.
    pub rating_min: Option<f64>,
    pub rating_max: Option<f64>,
    /// Inclusive bounds on `Work.added_at`.
    pub added_after: Option<OffsetDateTime>,
    pub added_before: Option<OffsetDateTime>,
    /// Boolean conditions, combined with everything above by AND.
    pub rules: Option<WorkRules>,
    /// Whose watch state and favorites `rules` mean. A rule that needs one
    /// without it is an [`InvalidInput`](crate::StorageError::InvalidInput).
    pub viewer: Option<UserId>,
    pub recursive: bool,
    pub sort: SortOrder,
    pub limit: u32,
    pub cursor: Option<PageCursor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTagsQuery {
    /// Restrict to these namespaces; empty means every namespace.
    pub namespaces: Vec<String>,
    pub limit: u32,
    pub cursor: Option<PageCursor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TagQuery {
    pub include_tags: Vec<TagId>,
    pub exclude_tags: Vec<TagId>,
    pub kinds: Vec<WorkKind>,
    pub limit: u32,
    pub cursor: Option<PageCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Lexical,
    Vector,
    #[default]
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridQuery {
    /// Apply library grants and parental restrictions for this user.
    /// Administrators leave this unset.
    pub access_user_id: Option<UserId>,
    pub text: String,
    /// Optional caller-provided query embedding. If `None` the server will
    /// embed the text using the configured Embedder before calling the
    /// backend.
    pub query_vector: Option<Vec<f32>>,
    /// The embedding namespace these vectors live in (e.g. "work_text").
    pub vector_namespace: String,
    pub kinds: Vec<WorkKind>,
    pub limit: u32,
    pub mode: SearchMode,
}
