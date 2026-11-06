// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;

use noflex_core::{TagId, UserId, WorkId, WorkKind};
use noflex_storage::{
    domain::{ContentCategory, ContentCategoryCount, Work, WorkFeatures},
    error::StorageResult,
    query::{ItemPage, ListWorksQuery, PageCursor, SortOrder},
};
use turso::{Connection, Value};

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_int_opt, col_real_opt, col_text, dt_to_text,
        text_to_dt, text_to_work_kind, uuid_to_blob,
    },
    error::map_turso,
    repo::{work_filters, works::row_to_work},
};

const SELECT_BASE: &str = "SELECT id, kind, parent_id, library_id, title, sort_title, \
    year, release_date, overview, external_ids_json, index_number, index_end, \
    parent_index, metadata_status, added_at, updated_at, tagline, original_title, \
    community_rating, locked_fields_json, metadata_provider FROM works";

/// Whether a row of `works` is anime: its library says so, or its library
/// defers to per-work tags and it carries the anime tag. The tag values match
/// `ANIME_TAG_SLUG` and `CATEGORY_TAG_NAMESPACE`.
const IS_ANIME: &str = "(library_id IN (SELECT id FROM libraries \
        WHERE config_content_category = 'ANIME') \
    OR (library_id IN (SELECT id FROM libraries WHERE config_content_category = 'AUTO') \
        AND EXISTS (SELECT 1 FROM tag_assignments ta JOIN tags t ON t.id = ta.tag_id \
            WHERE ta.target_kind = 'work' AND ta.target_id = works.id \
                AND t.namespace = 'category' AND t.slug = 'anime')))";

/// Index of the extra `LOWER(title)` column appended for title sorts;
/// `row_to_work` reads fixed indices 0..20, so it rides along unused.
const LOWERED_TITLE_COL: usize = 21;

pub(crate) async fn list(conn: &Connection, q: &ListWorksQuery) -> StorageResult<ItemPage<Work>> {
    // Title sorts need the cursor key exactly as SQLite compares it:
    // `LOWER()` folds ASCII only, while Rust's `to_lowercase` folds
    // full Unicode, so the lowered title is selected alongside and
    // read back at LOWERED_TITLE_COL instead of recomputed here.
    let title_sorted = matches!(q.sort, SortOrder::TitleAsc | SortOrder::TitleDesc);
    let mut sql = String::from(SELECT_BASE);
    if title_sorted {
        sql.insert_str(SELECT_BASE.len() - " FROM works".len(), ", LOWER(title)");
    }
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    let matcher = match &q.rules {
        Some(rules) => work_filters::compile(conn, rules, q.viewer).await?,
        None => None,
    };
    if let Some(user) = q.access_user_id {
        push_access_filter(&mut where_clauses, &mut params, user);
    }
    if let Some(lib) = q.library {
        where_clauses.push(format!("library_id = ?{}", params.len() + 1));
        params.push(Value::Blob(uuid_to_blob(lib.into_uuid())));
    }
    if let Some(category) = q.category {
        push_category_filter(&mut where_clauses, &mut params, category);
    }
    if let Some(parent) = q.parent {
        where_clauses.push(format!("parent_id = ?{}", params.len() + 1));
        params.push(Value::Blob(uuid_to_blob(parent.into_uuid())));
    }
    if let Some(tag) = q.tag {
        where_clauses.push(format!(
            "EXISTS (SELECT 1 FROM tag_assignments WHERE tag_id = ?{} \
             AND target_kind = 'work' AND target_id = works.id)",
            params.len() + 1
        ));
        params.push(Value::Blob(uuid_to_blob(tag.into_uuid())));
    }
    push_kind_filter(&mut where_clauses, &mut params, &q.kinds);
    if let Some(ymin) = q.year_min {
        where_clauses.push(format!("year >= ?{}", params.len() + 1));
        params.push(Value::Integer(ymin as i64));
    }
    if let Some(ymax) = q.year_max {
        where_clauses.push(format!("year <= ?{}", params.len() + 1));
        params.push(Value::Integer(ymax as i64));
    }
    if let Some(user) = q.favorite_for {
        where_clauses.push(format!(
            "id IN (SELECT target_id FROM user_data WHERE user_id = ?{} AND target_kind = 'work' AND favorite = 1)",
            params.len() + 1
        ));
        params.push(Value::Blob(uuid_to_blob(user.into_uuid())));
    }
    if let Some(watched) = q.watched {
        // NOT IN covers works with no user_data row and progress-only
        // rows alike: both count as unwatched.
        let op = if watched.watched { "IN" } else { "NOT IN" };
        where_clauses.push(format!(
            "id {op} (SELECT target_id FROM user_data WHERE user_id = ?{} AND target_kind = 'work' AND played = 1)",
            params.len() + 1
        ));
        params.push(Value::Blob(uuid_to_blob(watched.user_id.into_uuid())));
    }
    if let Some(rmin) = q.rating_min {
        // `NULL >= x` is never true, so unrated works drop out on their own.
        where_clauses.push(format!("community_rating >= ?{}", params.len() + 1));
        params.push(Value::Real(rmin));
    }
    if let Some(rmax) = q.rating_max {
        where_clauses.push(format!("community_rating <= ?{}", params.len() + 1));
        params.push(Value::Real(rmax));
    }
    if let Some(after) = q.added_after {
        where_clauses.push(format!("added_at >= ?{}", params.len() + 1));
        params.push(Value::Text(dt_to_text(after)?));
    }
    if let Some(before) = q.added_before {
        where_clauses.push(format!("added_at <= ?{}", params.len() + 1));
        params.push(Value::Text(dt_to_text(before)?));
    }
    if !q.recursive && q.parent.is_none() {
        where_clauses.push("parent_id IS NULL".to_string());
    }
    if let Some(cursor) = &q.cursor
        && let Some(keyset) = parse_cursor(cursor, q.sort)
    {
        push_cursor_clause(&mut where_clauses, &mut params, q.sort, keyset);
    }

    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(order_by_clause(q.sort));

    let limit = q.limit.max(1) as i64;
    // With rules, the page is however far down the listing it takes to fill it.
    if matcher.is_none() {
        sql.push_str(&format!(" LIMIT {}", limit + 1));
    }

    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut items: Vec<Work> = Vec::new();
    let mut lowered_titles: Vec<String> = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        if let Some(matcher) = &matcher {
            if items.len() as i64 > limit {
                break;
            }
            let id = row.get_value(0).map_err(map_turso)?;
            if !matcher.matches(col_blob(&id)?) {
                continue;
            }
        }
        if title_sorted {
            let lowered = row.get_value(LOWERED_TITLE_COL).map_err(map_turso)?;
            lowered_titles.push(col_text(&lowered)?.to_string());
        }
        items.push(row_to_work(&row)?);
    }
    let has_more = items.len() as i64 > limit;
    if has_more {
        // Drop the look-ahead row; cursors key off rows the caller
        // actually receives, not the extra one.
        items.pop();
        lowered_titles.pop();
    }
    let item_cursors = items
        .iter()
        .enumerate()
        .map(|(i, work)| {
            let key: Option<String> = match q.sort {
                SortOrder::DateAddedDesc | SortOrder::DateAddedAsc => {
                    Some(dt_to_text(work.added_at)?)
                }
                SortOrder::TitleAsc | SortOrder::TitleDesc => lowered_titles.get(i).cloned(),
                SortOrder::YearAsc | SortOrder::YearDesc => work.year.map(|y| y.to_string()),
                SortOrder::DateReleasedDesc => work.release_date.clone(),
                SortOrder::Random => None,
            };
            Ok(mint_cursor(q.sort, key.as_deref(), work.id))
        })
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(ItemPage {
        end_cursor: item_cursors.last().cloned(),
        items,
        item_cursors,
        // A random page has no stable order to keyset against: it never
        // honors an incoming cursor, so it never promises a next page.
        has_next: has_more && q.sort != SortOrder::Random,
    })
}

pub(crate) fn push_kind_filter(
    clauses: &mut Vec<String>,
    params: &mut Vec<Value>,
    kinds: &[WorkKind],
) {
    if kinds.is_empty() {
        return;
    }
    let placeholders: Vec<String> = kinds
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", params.len() + 1 + i))
        .collect();
    for k in kinds {
        params.push(Value::Text(
            crate::convert::work_kind_to_text(*k).to_string(),
        ));
    }
    clauses.push(format!("kind IN ({})", placeholders.join(",")));
}

pub(crate) fn push_category_filter(
    clauses: &mut Vec<String>,
    params: &mut Vec<Value>,
    category: ContentCategory,
) {
    clauses.push("parent_id IS NULL".to_string());
    push_kind_filter(clauses, params, category.kinds());
    match category {
        ContentCategory::Anime => clauses.push(IS_ANIME.to_string()),
        ContentCategory::Movies | ContentCategory::Shows => {
            clauses.push(format!("NOT ({IS_ANIME})"));
        }
        ContentCategory::Music | ContentCategory::Audiobooks | ContentCategory::Photos => {}
    }
}

pub(crate) async fn content_category_counts(
    conn: &Connection,
    access_user_id: Option<UserId>,
) -> StorageResult<Vec<ContentCategoryCount>> {
    let mut out = Vec::new();
    for category in ContentCategory::ALL {
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(user) = access_user_id {
            push_access_filter(&mut clauses, &mut params, user);
        }
        push_category_filter(&mut clauses, &mut params, category);
        let sql = format!("SELECT COUNT(*) FROM works WHERE {}", clauses.join(" AND "));
        let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
        let count = match rows.next().await.map_err(map_turso)? {
            Some(row) => col_int(&row.get_value(0).map_err(map_turso)?)?,
            None => 0,
        };
        if count > 0 {
            out.push(ContentCategoryCount {
                category,
                work_count: count as u64,
            });
        }
    }
    Ok(out)
}

pub(crate) async fn category_work_ids(
    conn: &Connection,
    category: ContentCategory,
    access_user_id: Option<UserId>,
) -> StorageResult<Vec<WorkId>> {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    if let Some(user) = access_user_id {
        push_access_filter(&mut clauses, &mut params, user);
    }
    push_category_filter(&mut clauses, &mut params, category);
    let sql = format!("SELECT id FROM works WHERE {}", clauses.join(" AND "));
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(WorkId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(0).map_err(map_turso)?,
        )?)?));
    }
    Ok(out)
}

pub(crate) async fn work_features(
    conn: &Connection,
    kinds: &[WorkKind],
    tag_namespaces: &[&str],
    access_user_id: Option<UserId>,
) -> StorageResult<Vec<WorkFeatures>> {
    let mut clauses = vec!["parent_id IS NULL".to_string()];
    let mut params = Vec::new();
    if let Some(user) = access_user_id {
        push_access_filter(&mut clauses, &mut params, user);
    }
    push_kind_filter(&mut clauses, &mut params, kinds);
    let sql = format!(
        "SELECT id, kind, year, community_rating, added_at, \
            (SELECT MAX(e.runtime_ms) FROM editions e WHERE e.work_id = works.id) \
         FROM works WHERE {}",
        clauses.join(" AND ")
    );
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    let mut out = Vec::new();
    let mut positions: HashMap<WorkId, usize> = HashMap::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let work = WorkId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(0).map_err(map_turso)?,
        )?)?);
        positions.insert(work, out.len());
        out.push(WorkFeatures {
            work,
            kind: text_to_work_kind(col_text(&row.get_value(1).map_err(map_turso)?)?)?,
            year: col_int_opt(&row.get_value(2).map_err(map_turso)?)?.map(|year| year as i32),
            community_rating: col_real_opt(&row.get_value(3).map_err(map_turso)?)?,
            added_at: text_to_dt(col_text(&row.get_value(4).map_err(map_turso)?)?)?,
            runtime_ms: col_int_opt(&row.get_value(5).map_err(map_turso)?)?,
            tags: Vec::new(),
        });
    }
    if tag_namespaces.is_empty() {
        return Ok(out);
    }
    let placeholders = (1..=tag_namespaces.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT ta.target_id, ta.tag_id FROM tag_assignments ta \
         JOIN tags t ON t.id = ta.tag_id \
         WHERE ta.target_kind = 'work' AND t.namespace IN ({placeholders})"
    );
    let params = tag_namespaces
        .iter()
        .map(|namespace| Value::Text((*namespace).to_string()))
        .collect::<Vec<_>>();
    let mut rows = conn.query(&sql, params).await.map_err(map_turso)?;
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let work = WorkId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(0).map_err(map_turso)?,
        )?)?);
        if let Some(position) = positions.get(&work) {
            let tag = TagId::from_uuid(blob_to_uuid(col_blob(
                &row.get_value(1).map_err(map_turso)?,
            )?)?);
            out[*position].tags.push(tag);
        }
    }
    Ok(out)
}

pub(crate) fn push_access_filter(
    clauses: &mut Vec<String>,
    params: &mut Vec<Value>,
    user: noflex_core::UserId,
) {
    let p = params.len() + 1;
    clauses.push(crate::repo::restrictions::accessible_work_predicate(
        "works", p,
    ));
    params.push(Value::Blob(uuid_to_blob(user.into_uuid())));
}

fn order_by_clause(sort: SortOrder) -> &'static str {
    match sort {
        SortOrder::DateAddedDesc => " ORDER BY added_at DESC, id DESC",
        SortOrder::DateAddedAsc => " ORDER BY added_at ASC, id ASC",
        SortOrder::TitleAsc => " ORDER BY LOWER(title) ASC, id ASC",
        SortOrder::TitleDesc => " ORDER BY LOWER(title) DESC, id DESC",
        SortOrder::YearDesc => " ORDER BY year DESC, id DESC",
        SortOrder::YearAsc => " ORDER BY year ASC, id ASC",
        SortOrder::DateReleasedDesc => " ORDER BY release_date DESC, id DESC",
        SortOrder::Random => " ORDER BY RANDOM()",
    }
}

/// One character identifying the sort a cursor was minted for; a
/// cursor whose tag doesn't match the requested sort is ignored.
fn sort_tag(sort: SortOrder) -> Option<&'static str> {
    match sort {
        SortOrder::DateAddedDesc | SortOrder::DateAddedAsc => Some("a"),
        SortOrder::TitleAsc | SortOrder::TitleDesc => Some("t"),
        SortOrder::YearAsc | SortOrder::YearDesc => Some("y"),
        SortOrder::DateReleasedDesc => Some("r"),
        SortOrder::Random => None,
    }
}

/// The sort-key half of a cursor, already decoded. `Null` is only
/// produced/consumed by the nullable sorts (year, release_date).
enum CursorKey {
    Text(String),
    Int(i64),
    Null,
}

struct Keyset {
    key: CursorKey,
    id: [u8; 16],
}

fn mint_cursor(sort: SortOrder, key: Option<&str>, id: WorkId) -> PageCursor {
    let tag = sort_tag(sort).unwrap_or("-");
    let hexkey = match key {
        Some(k) => k.bytes().map(|b| format!("{b:02x}")).collect::<String>(),
        None => "-".to_string(),
    };
    let hexid: String = uuid_to_blob(id.into_uuid())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    PageCursor(format!("{tag}:{hexkey}|{hexid}"))
}

/// Cursors are client-supplied and lenient: anything malformed, or
/// tagged for a different sort than the request asks for, is treated
/// as "no cursor" — a client that switches sort mid-pagination gets a
/// fresh first page rather than an error.
fn parse_cursor(c: &PageCursor, sort: SortOrder) -> Option<Keyset> {
    let tag = sort_tag(sort)?;
    let rest = c.0.strip_prefix(tag)?.strip_prefix(':')?;
    // `rsplit_once` — a `|` must never split the key half.
    let (hexkey, hexid) = rest.rsplit_once('|')?;
    let id: [u8; 16] = decode_hex(hexid)?.try_into().ok()?;
    let key = if hexkey == "-" {
        match sort {
            SortOrder::YearAsc | SortOrder::YearDesc | SortOrder::DateReleasedDesc => {
                CursorKey::Null
            }
            _ => return None,
        }
    } else {
        let text = String::from_utf8(decode_hex(hexkey)?).ok()?;
        match sort {
            SortOrder::YearAsc | SortOrder::YearDesc => CursorKey::Int(text.parse().ok()?),
            _ => CursorKey::Text(text),
        }
    };
    Some(Keyset { key, id })
}

fn push_cursor_clause(
    clauses: &mut Vec<String>,
    params: &mut Vec<Value>,
    sort: SortOrder,
    ks: Keyset,
) {
    let id = Value::Blob(ks.id.to_vec());
    // Placeholders are numbered off the live param count, so each arm
    // pushes its values in the same order the clause references them.
    match (sort, ks.key) {
        (SortOrder::DateAddedDesc, CursorKey::Text(key)) => {
            clauses.push(format!(
                "(added_at, id) < (?{}, ?{})",
                params.len() + 1,
                params.len() + 2
            ));
            params.extend([Value::Text(key), id]);
        }
        (SortOrder::DateAddedAsc, CursorKey::Text(key)) => {
            clauses.push(format!(
                "(added_at, id) > (?{}, ?{})",
                params.len() + 1,
                params.len() + 2
            ));
            params.extend([Value::Text(key), id]);
        }
        (SortOrder::TitleAsc, CursorKey::Text(key)) => {
            clauses.push(format!(
                "(LOWER(title), id) > (?{}, ?{})",
                params.len() + 1,
                params.len() + 2
            ));
            params.extend([Value::Text(key), id]);
        }
        (SortOrder::TitleDesc, CursorKey::Text(key)) => {
            clauses.push(format!(
                "(LOWER(title), id) < (?{}, ?{})",
                params.len() + 1,
                params.len() + 2
            ));
            params.extend([Value::Text(key), id]);
        }
        (SortOrder::YearAsc, CursorKey::Int(year)) => {
            clauses.push(format!(
                "(year, id) > (?{}, ?{})",
                params.len() + 1,
                params.len() + 2
            ));
            params.extend([Value::Integer(year), id]);
        }
        (SortOrder::YearAsc, CursorKey::Null) => {
            // ASC puts NULL years first; a NULL key therefore still
            // admits every non-NULL row after the remaining NULLs.
            clauses.push(format!(
                "((year IS NULL AND id > ?{}) OR year IS NOT NULL)",
                params.len() + 1
            ));
            params.push(id);
        }
        (SortOrder::YearDesc, CursorKey::Int(year)) => {
            // DESC puts NULL years last, so they always stay eligible
            // while the key is non-NULL.
            clauses.push(format!(
                "((year, id) < (?{}, ?{}) OR year IS NULL)",
                params.len() + 1,
                params.len() + 2
            ));
            params.extend([Value::Integer(year), id]);
        }
        (SortOrder::YearDesc, CursorKey::Null) => {
            clauses.push(format!("year IS NULL AND id < ?{}", params.len() + 1));
            params.push(id);
        }
        (SortOrder::DateReleasedDesc, CursorKey::Text(date)) => {
            clauses.push(format!(
                "((release_date, id) < (?{}, ?{}) OR release_date IS NULL)",
                params.len() + 1,
                params.len() + 2
            ));
            params.extend([Value::Text(date), id]);
        }
        (SortOrder::DateReleasedDesc, CursorKey::Null) => {
            clauses.push(format!(
                "release_date IS NULL AND id < ?{}",
                params.len() + 1
            ));
            params.push(id);
        }
        // parse_cursor keeps sort and key shape in agreement; a
        // mismatch here means no clause, same as no cursor.
        _ => {}
    }
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    // Only even-length ASCII can be sliced pairwise below; anything
    // else is just "no cursor".
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}
