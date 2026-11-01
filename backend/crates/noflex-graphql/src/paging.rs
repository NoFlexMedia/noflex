// SPDX-License-Identifier: AGPL-3.0-only
//
// Connection helpers for resolvers. Keyset lists pass storage cursors
// through untouched (`noflex_storage::connection`). Ranked lists —
// search hits, recommendations, home shelves — have no storage keyset,
// so their cursors encode a rank position here at the GraphQL layer;
// the Storage trait itself only ever sees opaque cursors.

use std::future::Future;

use async_graphql::{OutputType, connection::Connection};
pub(crate) use noflex_storage::connection::{Window, cost, empty, from_item_page};
use noflex_storage::{ItemPage, PageCursor};

/// A [`Window`] over a ranked list: skip `offset` hits, return `first`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RankWindow {
    pub offset: usize,
    pub first: usize,
}

const RANK_PREFIX: &str = "rank:";

fn rank_cursor(rank: usize) -> String {
    format!("{RANK_PREFIX}{rank}")
}

impl RankWindow {
    /// Resolve `first`/`after` for a ranked list no deeper than
    /// `max_depth` hits; `after` must be a cursor this list minted.
    pub fn new(
        first: Option<i32>,
        after: Option<String>,
        default: u32,
        max: u32,
        max_depth: usize,
    ) -> async_graphql::Result<Self> {
        let window = Window::new(first, after, default, max)?;
        let offset = match window.after {
            None => 0,
            Some(PageCursor(cursor)) => cursor
                .strip_prefix(RANK_PREFIX)
                .and_then(|rank| rank.parse::<usize>().ok())
                .map(|rank| rank.saturating_add(1))
                .ok_or_else(|| async_graphql::Error::new("invalid cursor"))?,
        };
        Ok(Self {
            offset: offset.min(max_depth),
            first: (window.first as usize).min(max_depth - offset.min(max_depth)),
        })
    }

    /// Hits to fetch from rank zero: this page plus one look-ahead.
    pub fn fetch(&self) -> usize {
        self.offset + self.first + 1
    }

    /// This window's slice of `ranked` (best first, from rank zero).
    pub fn connection<T: OutputType>(&self, ranked: Vec<T>) -> Connection<String, T> {
        let has_next = self.first > 0 && ranked.len() > self.offset + self.first;
        noflex_storage::connection::from_edges(
            ranked
                .into_iter()
                .enumerate()
                .skip(self.offset)
                .take(self.first)
                .map(|(rank, node)| (rank_cursor(rank), node)),
            self.offset > 0,
            has_next,
        )
    }
}

/// Most storage rows one filtered page may scan before giving up.
const MAX_SCANNED: usize = 2000;

/// A connection over a keyset list whose rows are filtered after the
/// fetch (per-row access checks storage can't express). `fetch(limit,
/// after)` pulls a storage page; `keep` returns the surviving
/// `(cursor, row)` pairs in order. Pages are pulled until `first` rows
/// survive or the list ends, so a page is short only at the end; each
/// edge keeps its storage cursor. Past [`MAX_SCANNED`] rows the page ends
/// early: `hasNextPage` stays true when a row survived, and is false
/// otherwise, since an empty page has no cursor to resume from.
pub(crate) async fn filtered<T, F, Fut, K, KFut>(
    window: Window,
    mut fetch: F,
    mut keep: K,
) -> async_graphql::Result<Connection<String, T>>
where
    T: OutputType,
    F: FnMut(u32, Option<PageCursor>) -> Fut,
    Fut: Future<Output = async_graphql::Result<ItemPage<T>>>,
    K: FnMut(Vec<(PageCursor, T)>) -> KFut,
    KFut: Future<Output = async_graphql::Result<Vec<(PageCursor, T)>>>,
{
    let resumed = window.after.is_some();
    let want = window.first as usize;
    let batch = window.first.saturating_mul(2);
    let mut cursor = window.after;
    let mut edges: Vec<(PageCursor, T)> = Vec::with_capacity(want);
    let mut scanned = 0;
    let has_next = loop {
        let page = fetch(batch, cursor.take()).await?;
        scanned += page.items.len();
        let has_more = page.has_next;
        cursor = page.item_cursors.last().cloned();
        let mut kept = keep(page.item_cursors.into_iter().zip(page.items).collect()).await?;
        if edges.len() + kept.len() > want {
            kept.truncate(want - edges.len());
            edges.extend(kept);
            break true;
        }
        edges.extend(kept);
        if edges.len() == want {
            break has_more;
        }
        if !has_more {
            break false;
        }
        if scanned >= MAX_SCANNED {
            break !edges.is_empty();
        }
    };
    Ok(noflex_storage::connection::from_edges(
        edges.into_iter().map(|(c, node)| (c.0, node)),
        resumed,
        has_next,
    ))
}
