// SPDX-License-Identifier: AGPL-3.0-only
//
// GraphQL Cursor Connections glue for list fields. Lives here, not in
// `noflex-graphql`, because the storage-shape types' complex fields
// (`Tag.works`, `Collection.items`, …) return connections too. Storage
// cursors pass through verbatim as edge cursors, so neither layer ever
// interprets another backend's cursor format.

use async_graphql::{
    OutputType,
    connection::{Connection, Edge},
};

use crate::query::{ItemPage, PageCursor};

/// One forward page request: `first` already defaulted and clamped.
#[derive(Debug, Clone)]
pub struct Window {
    pub first: u32,
    pub after: Option<PageCursor>,
}

impl Window {
    /// Resolve a field's `first`/`after` arguments. `first` defaults to
    /// `default` and is clamped into `1..=max`; a negative `first` is
    /// an error, as the Connections spec requires.
    pub fn new(
        first: Option<i32>,
        after: Option<String>,
        default: u32,
        max: u32,
    ) -> async_graphql::Result<Self> {
        let first = match first {
            None => default,
            Some(n) if n < 0 => {
                return Err(async_graphql::Error::new("`first` must be non-negative"));
            }
            Some(n) => n.unsigned_abs(),
        };
        Ok(Self {
            first: first.clamp(1, max),
            after: after.map(PageCursor),
        })
    }
}

/// Complexity of a connection field: `first` (defaulted and clamped as
/// [`Window::new`] does) times the cost of the selection under it, so
/// `edges { node }` and `nodes` are each priced per returned row.
pub fn cost(first: Option<i32>, default: u32, max: u32, child_complexity: usize) -> usize {
    let first = first.map_or(default, |n| n.max(1).unsigned_abs());
    first.clamp(1, max) as usize * child_complexity
}

/// A connection over a storage keyset page. `hasPreviousPage` is true
/// whenever the request resumed from a cursor — cheap, and exact unless
/// the row behind that cursor has since been deleted.
pub fn from_item_page<T: OutputType>(page: ItemPage<T>, resumed: bool) -> Connection<String, T> {
    from_edges(
        page.item_cursors.into_iter().map(|c| c.0).zip(page.items),
        resumed,
        page.has_next,
    )
}

/// A connection from `(cursor, node)` pairs in page order.
pub fn from_edges<T: OutputType>(
    edges: impl IntoIterator<Item = (String, T)>,
    has_previous: bool,
    has_next: bool,
) -> Connection<String, T> {
    let mut connection = Connection::new(has_previous, has_next);
    connection.edges.extend(
        edges
            .into_iter()
            .map(|(cursor, node)| Edge::new(cursor, node)),
    );
    connection
}

/// An empty, final connection — for a list the caller may not see.
pub fn empty<T: OutputType>() -> Connection<String, T> {
    Connection::new(false, false)
}
