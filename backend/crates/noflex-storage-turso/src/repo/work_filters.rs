// SPDX-License-Identifier: AGPL-3.0-only
//
// Browse filters: `WorkRules` evaluated against the works, and the values each
// facet takes.
//
// Each condition collects the ids it matches into a set, and the boolean logic runs
// here over those sets while the listing streams past in sort order. SQL can't carry
// the logic on this engine: `id IN (subquery)` under OR or NOT runs per row, which
// took 65 s over 20,000 titles.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use noflex_core::{UserId, text::canonical_language};
use noflex_storage::{
    MatchMode, StorageError, WorkFacet, WorkFacetValue, WorkFacetValues, WorkFacetsQuery, WorkRule,
    WorkRules, error::StorageResult,
};
use time::{Duration, OffsetDateTime};
use turso::{Connection, Value};

use crate::{
    convert::{col_blob, col_int, col_text, dt_to_text, uuid_to_blob},
    error::map_turso,
    repo::list_works::{push_access_filter, push_category_filter},
};

/// Each costs a scan per page, so this bounds what one request can make the server do.
const MAX_CONDITIONS: usize = 64;
const MAX_VALUES_PER_RULE: usize = 200;
/// Deeper than anyone reasons about, and shallow enough to never exhaust the stack.
const MAX_GROUP_DEPTH: usize = 8;
/// How many of a facet's values to list; cast lists run to many thousands.
const MAX_FACET_VALUES: usize = 500;

/// From a stream `s` up to the work its file belongs to.
const STREAM_LINEAGE: &str = " JOIN media_files m ON m.id = s.media_file_id \
    JOIN editions e ON e.id = m.edition_id JOIN works w ON w.id = e.work_id";

/// Where a file facet's value is stored.
struct MediaColumn {
    /// Everything from the value's table up to the work.
    from: String,
    column: &'static str,
    /// What else a row must satisfy for its value to count.
    guard: &'static str,
}

fn media_column(facet: WorkFacet) -> Option<MediaColumn> {
    let subtype = |table: &str, column, guard| MediaColumn {
        from: format!("{table} x JOIN media_streams s ON s.id = x.stream_id{STREAM_LINEAGE}"),
        column,
        guard,
    };
    let base = |column, guard| MediaColumn {
        from: format!("media_streams s{STREAM_LINEAGE}"),
        column,
        guard,
    };
    Some(match facet {
        WorkFacet::Resolution => subtype("video_streams", "x.resolution", "1"),
        WorkFacet::DynamicRange => subtype("video_streams", "x.dynamic_range", "1"),
        WorkFacet::AudioLayout => subtype("audio_streams", "x.layout", "1"),
        WorkFacet::VideoCodec => base("s.codec", "s.kind = 'video'"),
        WorkFacet::AudioCodec => base("s.codec", "s.kind = 'audio'"),
        WorkFacet::AudioLanguage => base("s.language", "s.kind = 'audio'"),
        WorkFacet::SubtitleCodec => base("s.codec", "s.kind = 'subtitle'"),
        WorkFacet::SubtitleLanguage => base("s.language", "s.kind = 'subtitle'"),
        WorkFacet::Edition => MediaColumn {
            from: "editions e JOIN works w ON w.id = e.work_id".to_string(),
            column: "e.name",
            guard: "e.name <> 'Default'",
        },
        _ => return None,
    })
}

const TAG_FACETS: [(WorkFacet, &str); 4] = [
    (WorkFacet::Genre, "genre"),
    (WorkFacet::Mood, "mood"),
    (WorkFacet::Studio, "studio"),
    (WorkFacet::ContentRating, "parental"),
];

/// How each role reads inside `tag_assignments.metadata`. Serialized by us, so a
/// substring is exact, and it costs far less than expanding the JSON of every credit.
const CREDIT_FACETS: [(WorkFacet, &str); 4] = [
    (WorkFacet::Director, "%\"role\":\"Director\"%"),
    (WorkFacet::Actor, "%\"role\":\"Actor\"%"),
    (WorkFacet::Writer, "%\"role\":\"Writer\"%"),
    (WorkFacet::Producer, "%\"role\":\"Producer\"%"),
];

fn lookup(table: &[(WorkFacet, &'static str)], facet: WorkFacet) -> Option<&'static str> {
    table
        .iter()
        .find_map(|(entry, text)| (*entry == facet).then_some(*text))
}

type WorkKey = Vec<u8>;

/// A rule or group, ready to ask about one work at a time. Sets are shared because
/// a viewer's watch state takes the same three however many values ask about it.
pub(crate) enum Matcher {
    In(Arc<HashSet<WorkKey>>),
    All(Vec<Matcher>),
    Any(Vec<Matcher>),
    None(Vec<Matcher>),
}

impl Matcher {
    fn of(mode: MatchMode, parts: Vec<Matcher>) -> Self {
        match mode {
            MatchMode::All => Self::All(parts),
            MatchMode::Any => Self::Any(parts),
            MatchMode::None => Self::None(parts),
        }
    }

    /// Whether the work with this raw id satisfies the rule.
    pub(crate) fn matches(&self, work: &[u8]) -> bool {
        match self {
            Self::In(set) => set.contains(work),
            Self::All(parts) => parts.iter().all(|part| part.matches(work)),
            Self::Any(parts) => parts.iter().any(|part| part.matches(work)),
            Self::None(parts) => !parts.iter().any(|part| part.matches(work)),
        }
    }
}

/// Every column of every row, as work ids: a condition selects a work and, for what
/// sits on files or episodes, its parent and root too. Parents nest two levels at most,
/// so that's everything above it.
async fn id_set(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> StorageResult<HashSet<WorkKey>> {
    let mut rows = conn.query(sql, params).await.map_err(map_turso)?;
    let columns = rows.column_count();
    let mut set = HashSet::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        for column in 0..columns {
            if let Value::Blob(id) = row.get_value(column).map_err(map_turso)? {
                set.insert(id);
            }
        }
    }
    Ok(set)
}

fn text_values(values: &[String]) -> Vec<Value> {
    values
        .iter()
        .map(|value| Value::Text(value.clone()))
        .collect()
}

/// `?first`, `?first + 1`, ... for `count` parameters.
fn placeholders(first: usize, count: usize) -> String {
    (first..first + count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One work's conditions, with their scans budgeted.
struct Compiler<'a> {
    conn: &'a Connection,
    viewer: Option<UserId>,
    scans: usize,
    started: Option<Arc<HashSet<WorkKey>>>,
    plays: Option<PlaySets>,
}

/// What's been played, and what's unplayed at or beneath a work. Together they tell
/// finished from started, and need a walk over every edition to say so.
#[derive(Clone)]
struct PlaySets {
    played: Arc<HashSet<WorkKey>>,
    unplayed: Arc<HashSet<WorkKey>>,
}

impl Compiler<'_> {
    async fn scan(
        &mut self,
        sql: &str,
        params: Vec<Value>,
    ) -> StorageResult<Arc<HashSet<WorkKey>>> {
        self.scans += 1;
        if self.scans > MAX_CONDITIONS {
            return Err(StorageError::InvalidInput(
                "too many filter conditions".to_string(),
            ));
        }
        Ok(Arc::new(id_set(self.conn, sql, params).await?))
    }

    fn viewer(&self) -> StorageResult<Value> {
        let viewer = self.viewer.ok_or_else(|| {
            StorageError::InvalidInput("this filter needs a signed-in viewer".to_string())
        })?;
        Ok(Value::Blob(uuid_to_blob(viewer.into_uuid())))
    }

    /// The works any of `values` applies to; `None` for a facet read one value at a time.
    async fn set_of(
        &mut self,
        facet: WorkFacet,
        values: &[String],
    ) -> StorageResult<Option<Arc<HashSet<WorkKey>>>> {
        if let Some(namespace) = lookup(&TAG_FACETS, facet) {
            let sql = format!(
                "SELECT ta.target_id FROM tag_assignments ta JOIN tags t ON t.id = ta.tag_id \
                 WHERE ta.target_kind = 'work' AND t.namespace = ?1 AND t.slug IN ({})",
                placeholders(2, values.len())
            );
            let mut params = vec![Value::Text(namespace.to_string())];
            params.extend(text_values(values));
            return Ok(Some(self.scan(&sql, params).await?));
        }
        if let Some(pattern) = lookup(&CREDIT_FACETS, facet) {
            let sql = format!(
                "SELECT ta.target_id FROM tag_assignments ta JOIN tags t ON t.id = ta.tag_id \
                 WHERE ta.target_kind = 'work' AND t.namespace = 'person' \
                    AND ta.metadata LIKE ?1 AND t.slug IN ({})",
                placeholders(2, values.len())
            );
            let mut params = vec![Value::Text(pattern.to_string())];
            params.extend(text_values(values));
            return Ok(Some(self.scan(&sql, params).await?));
        }
        if facet == WorkFacet::Collection {
            let titles = values
                .iter()
                .map(|title| title.to_ascii_lowercase())
                .collect::<Vec<_>>();
            let sql = format!(
                "SELECT ci.work_id FROM collection_items ci JOIN collections c ON c.id = ci.collection_id \
                 WHERE LOWER(c.title) IN ({})",
                placeholders(1, titles.len())
            );
            return Ok(Some(self.scan(&sql, text_values(&titles)).await?));
        }
        if facet == WorkFacet::Favorite {
            let viewer = self.viewer()?;
            return Ok(Some(
                self.scan(
                    "SELECT target_id FROM user_data \
                     WHERE user_id = ?1 AND target_kind = 'work' AND favorite = 1",
                    vec![viewer],
                )
                .await?,
            ));
        }
        let Some(media) = media_column(facet) else {
            return Ok(None);
        };
        let values = if matches!(
            facet,
            WorkFacet::AudioLanguage | WorkFacet::SubtitleLanguage
        ) {
            values.iter().map(|code| canonical_language(code)).collect()
        } else {
            values.to_vec()
        };
        let sql = format!(
            "SELECT e.work_id, w.parent_id, w.root_id FROM {} WHERE {} AND {} IN ({})",
            media.from,
            media.guard,
            media.column,
            placeholders(1, values.len())
        );
        Ok(Some(self.scan(&sql, text_values(&values)).await?))
    }

    /// Works the viewer has played or begun, or that have such a work beneath them.
    async fn started(&mut self) -> StorageResult<Arc<HashSet<WorkKey>>> {
        if let Some(set) = &self.started {
            return Ok(set.clone());
        }
        let viewer = self.viewer()?;
        let set = self
            .scan(
                "SELECT ud.target_id, w.parent_id, w.root_id FROM user_data ud \
                 JOIN works w ON w.id = ud.target_id \
                 WHERE ud.user_id = ?1 AND ud.target_kind = 'work' \
                    AND (ud.played = 1 OR ud.position_ms > 0)",
                vec![viewer],
            )
            .await?;
        self.started = Some(set.clone());
        Ok(set)
    }

    async fn play_sets(&mut self) -> StorageResult<PlaySets> {
        if let Some(sets) = &self.plays {
            return Ok(sets.clone());
        }
        let viewer = self.viewer()?;
        let played = self
            .scan(
                "SELECT target_id FROM user_data \
                 WHERE user_id = ?1 AND target_kind = 'work' AND played = 1",
                vec![viewer],
            )
            .await?;
        // Filtered here: joining `user_data` for every edition ran for minutes.
        let mut rows = self
            .conn
            .query(
                "SELECT e.work_id, w.parent_id, w.root_id FROM editions e \
                 JOIN works w ON w.id = e.work_id",
                (),
            )
            .await
            .map_err(map_turso)?;
        let mut unplayed = HashSet::new();
        while let Some(row) = rows.next().await.map_err(map_turso)? {
            let work = row.get_value(0).map_err(map_turso)?;
            if played.contains(col_blob(&work)?) {
                continue;
            }
            for column in 0..3 {
                if let Value::Blob(id) = row.get_value(column).map_err(map_turso)? {
                    unplayed.insert(id);
                }
            }
        }
        let sets = PlaySets {
            played,
            unplayed: Arc::new(unplayed),
        };
        self.plays = Some(sets.clone());
        Ok(sets)
    }

    /// One value of a facet that isn't a plain set of works.
    async fn value_matcher(&mut self, facet: WorkFacet, value: &str) -> StorageResult<Matcher> {
        let invalid = || StorageError::InvalidInput(format!("invalid {facet:?} filter: {value:?}"));
        let on_works = |condition: &str| format!("SELECT id FROM works WHERE {condition}");
        let set = match facet {
            WorkFacet::Years => {
                let (from, to) = value.split_once('-').unwrap_or((value, value));
                let bound = |text: &str| -> StorageResult<Option<i64>> {
                    (!text.is_empty())
                        .then(|| text.parse().map_err(|_| invalid()))
                        .transpose()
                };
                let (from, to) = (bound(from)?, bound(to)?);
                if from.is_none() && to.is_none() {
                    return Err(invalid());
                }
                let sql = on_works("year >= COALESCE(?1, year) AND year <= COALESCE(?2, year)");
                let param = |bound: Option<i64>| bound.map_or(Value::Null, Value::Integer);
                self.scan(&sql, vec![param(from), param(to)]).await?
            }
            WorkFacet::MinRating => {
                let floor = value.parse().map_err(|_| invalid())?;
                self.scan(
                    &on_works("community_rating >= ?1"),
                    vec![Value::Real(floor)],
                )
                .await?
            }
            WorkFacet::Title => {
                let escaped = value
                    .to_ascii_lowercase()
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                self.scan(
                    &on_works("LOWER(title) LIKE ?1 ESCAPE '\\'"),
                    vec![Value::Text(format!("%{escaped}%"))],
                )
                .await?
            }
            WorkFacet::AddedWithinDays => {
                let days: i64 = value.parse().map_err(|_| invalid())?;
                let since = OffsetDateTime::now_utc()
                    .checked_sub(Duration::days(days))
                    .ok_or_else(invalid)?;
                self.scan(
                    &on_works("added_at >= ?1"),
                    vec![Value::Text(dt_to_text(since)?)],
                )
                .await?
            }
            WorkFacet::MatchStatus => {
                let condition = match value {
                    "unmatched" => "metadata_status = 'unconfirmed'",
                    "matched" => "metadata_status <> 'unconfirmed'",
                    _ => return Err(invalid()),
                };
                self.scan(&on_works(condition), Vec::new()).await?
            }
            WorkFacet::WatchState => {
                let started = self.started().await?;
                if value == "unwatched" {
                    return Ok(Matcher::None(vec![Matcher::In(started)]));
                }
                let PlaySets { played, unplayed } = self.play_sets().await?;
                return match value {
                    "in_progress" => Ok(Matcher::All(vec![
                        Matcher::In(started),
                        Matcher::None(vec![Matcher::In(played)]),
                        Matcher::In(unplayed),
                    ])),
                    "watched" => Ok(Matcher::Any(vec![
                        Matcher::In(played),
                        Matcher::All(vec![
                            Matcher::In(started),
                            Matcher::None(vec![Matcher::In(unplayed)]),
                        ]),
                    ])),
                    _ => Err(invalid()),
                };
            }
            _ => return Err(invalid()),
        };
        Ok(Matcher::In(set))
    }

    async fn rule_matcher(&mut self, rule: &WorkRule) -> StorageResult<Option<Matcher>> {
        if rule.values.is_empty() {
            return Ok(None);
        }
        if rule.values.len() > MAX_VALUES_PER_RULE {
            return Err(StorageError::InvalidInput(
                "too many values in one filter".to_string(),
            ));
        }
        // One scan covers any number of values unless each must apply on its own.
        if rule.mode != MatchMode::All
            && let Some(set) = self.set_of(rule.facet, &rule.values).await?
        {
            return Ok(Some(Matcher::of(rule.mode, vec![Matcher::In(set)])));
        }
        let mut parts = Vec::new();
        for value in &rule.values {
            parts.push(
                match self.set_of(rule.facet, std::slice::from_ref(value)).await? {
                    Some(set) => Matcher::In(set),
                    None => self.value_matcher(rule.facet, value).await?,
                },
            );
        }
        Ok(Some(Matcher::of(rule.mode, parts)))
    }

    /// Boxed: a group's future contains its nested groups'.
    fn group_matcher<'s>(
        &'s mut self,
        group: &'s WorkRules,
        depth: usize,
    ) -> Pin<Box<dyn Future<Output = StorageResult<Option<Matcher>>> + Send + 's>> {
        Box::pin(async move {
            if depth > MAX_GROUP_DEPTH {
                return Err(StorageError::InvalidInput(
                    "filter groups are nested too deeply".to_string(),
                ));
            }
            let mut parts = Vec::new();
            for rule in &group.rules {
                parts.extend(self.rule_matcher(rule).await?);
            }
            for nested in &group.groups {
                parts.extend(self.group_matcher(nested, depth + 1).await?);
            }
            Ok((!parts.is_empty()).then(|| Matcher::of(group.mode, parts)))
        })
    }
}

/// `rules` ready to test works against; `None` when no rule says anything.
///
/// # Errors
/// [`StorageError::InvalidInput`] for a value its facet can't parse, a rule that
/// needs a viewer without one, groups nested too deeply, or too many conditions.
pub(crate) async fn compile(
    conn: &Connection,
    rules: &WorkRules,
    viewer: Option<UserId>,
) -> StorageResult<Option<Matcher>> {
    Compiler {
        conn,
        viewer,
        scans: 0,
        started: None,
        plays: None,
    }
    .group_matcher(rules, 1)
    .await
}

/// A `SELECT id` of the works `q` covers, and its parameters.
fn scope(q: &WorkFacetsQuery) -> (String, Vec<Value>) {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    if let Some(user) = q.access_user_id {
        push_access_filter(&mut clauses, &mut params, user);
    }
    if let Some(library) = q.library {
        clauses.push(format!("library_id = ?{}", params.len() + 1));
        params.push(Value::Blob(uuid_to_blob(library.into_uuid())));
    }
    match q.category {
        Some(category) => push_category_filter(&mut clauses, &mut params, category),
        None => clauses.push("parent_id IS NULL".to_string()),
    }
    (
        format!("SELECT id FROM works WHERE {}", clauses.join(" AND ")),
        params,
    )
}

/// `sql`'s rows of `(value, label, work)`, as how many works in `in_scope` have each
/// value. Counted here because an `IN` on a computed title runs per row.
async fn counted_values(
    conn: &Connection,
    sql: &str,
    in_scope: &HashSet<WorkKey>,
) -> StorageResult<Vec<WorkFacetValue>> {
    let mut rows = conn.query(sql, ()).await.map_err(map_turso)?;
    let mut found: HashMap<String, (String, HashSet<WorkKey>)> = HashMap::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let work = row.get_value(2).map_err(map_turso)?;
        let work = col_blob(&work)?;
        if !in_scope.contains(work) {
            continue;
        }
        let value = row.get_value(0).map_err(map_turso)?;
        let value = col_text(&value)?;
        match found.get_mut(value) {
            Some((_, works)) => {
                works.insert(work.to_vec());
            }
            None => {
                let label = row.get_value(1).map_err(map_turso)?;
                let label = col_text(&label)?.to_string();
                found.insert(value.to_string(), (label, HashSet::from([work.to_vec()])));
            }
        }
    }
    Ok(found
        .into_iter()
        .map(|(value, (label, works))| WorkFacetValue {
            value,
            label,
            work_count: works.len() as u32,
        })
        .collect())
}

/// See [`noflex_storage::Storage::work_facets`].
pub(crate) async fn facets(
    conn: &Connection,
    q: &WorkFacetsQuery,
) -> StorageResult<Vec<WorkFacetValues>> {
    let (scope, params) = scope(q);
    let in_scope = id_set(conn, &scope, params).await?;
    let mut out = Vec::new();
    let mut push = |facet, mut values: Vec<WorkFacetValue>| {
        values.sort_by(|a, b| {
            b.work_count
                .cmp(&a.work_count)
                .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
        });
        let truncated = values.len() > MAX_FACET_VALUES;
        values.truncate(MAX_FACET_VALUES);
        if !values.is_empty() {
            out.push(WorkFacetValues {
                facet,
                values,
                truncated,
            });
        }
    };

    for (facet, namespace) in TAG_FACETS {
        let sql = format!(
            "SELECT t.slug, t.display, ta.target_id FROM tag_assignments ta \
             JOIN tags t ON t.id = ta.tag_id \
             WHERE ta.target_kind = 'work' AND t.namespace = '{namespace}'"
        );
        push(facet, counted_values(conn, &sql, &in_scope).await?);
    }
    for (facet, pattern) in CREDIT_FACETS {
        let sql = format!(
            "SELECT t.slug, t.display, ta.target_id FROM tag_assignments ta \
             JOIN tags t ON t.id = ta.tag_id \
             WHERE ta.target_kind = 'work' AND t.namespace = 'person' AND ta.metadata LIKE '{pattern}'"
        );
        push(facet, counted_values(conn, &sql, &in_scope).await?);
    }
    let sql = "SELECT c.title, c.title, ci.work_id FROM collection_items ci \
        JOIN collections c ON c.id = ci.collection_id";
    push(
        WorkFacet::Collection,
        counted_values(conn, sql, &in_scope).await?,
    );

    let mut rows = conn
        .query("SELECT id, year FROM works WHERE year IS NOT NULL", ())
        .await
        .map_err(map_turso)?;
    let mut decades: HashMap<i64, u32> = HashMap::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let work = row.get_value(0).map_err(map_turso)?;
        if in_scope.contains(col_blob(&work)?) {
            let year = col_int(&row.get_value(1).map_err(map_turso)?)?;
            *decades.entry(year / 10 * 10).or_default() += 1;
        }
    }
    push(
        WorkFacet::Years,
        decades
            .into_iter()
            .map(|(decade, work_count)| WorkFacetValue {
                value: format!("{decade}-{}", decade + 9),
                label: format!("{decade}s"),
                work_count,
            })
            .collect(),
    );

    let edition = media_column(WorkFacet::Edition);
    if let Some(edition) = edition {
        let sql = format!(
            "SELECT e.name, e.name, w.root_id FROM {} WHERE {}",
            edition.from, edition.guard
        );
        push(
            WorkFacet::Edition,
            counted_values(conn, &sql, &in_scope).await?,
        );
    }

    // One pass answers every stream facet, where a scan apiece took most of a minute
    // in a debug build over 50,000 files.
    let mut streams: BTreeMap<(WorkFacet, String), HashSet<WorkKey>> = BTreeMap::new();
    let mut rows = conn
        .query(
            &format!(
                "SELECT s.kind, s.codec, s.language, v.resolution, v.dynamic_range, a.layout, w.root_id \
                 FROM media_streams s LEFT JOIN video_streams v ON v.stream_id = s.id \
                 LEFT JOIN audio_streams a ON a.stream_id = s.id{STREAM_LINEAGE}"
            ),
            (),
        )
        .await
        .map_err(map_turso)?;
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let root = row.get_value(6).map_err(map_turso)?;
        let root = col_blob(&root)?;
        if !in_scope.contains(root) {
            continue;
        }
        let kind = row.get_value(0).map_err(map_turso)?;
        let (codec, language) = match col_text(&kind)? {
            "video" => (WorkFacet::VideoCodec, None),
            "audio" => (WorkFacet::AudioCodec, Some(WorkFacet::AudioLanguage)),
            _ => (WorkFacet::SubtitleCodec, Some(WorkFacet::SubtitleLanguage)),
        };
        for (column, facet) in [
            (1, Some(codec)),
            (2, language),
            (3, Some(WorkFacet::Resolution)),
            (4, Some(WorkFacet::DynamicRange)),
            (5, Some(WorkFacet::AudioLayout)),
        ] {
            if let (Some(facet), Value::Text(value)) =
                (facet, row.get_value(column).map_err(map_turso)?)
            {
                streams
                    .entry((facet, value))
                    .or_default()
                    .insert(root.to_vec());
            }
        }
    }
    drop(rows);
    let mut by_facet: BTreeMap<WorkFacet, Vec<WorkFacetValue>> = BTreeMap::new();
    for ((facet, value), works) in streams {
        by_facet.entry(facet).or_default().push(WorkFacetValue {
            label: value.clone(),
            value,
            work_count: works.len() as u32,
        });
    }
    for (facet, values) in by_facet {
        push(facet, values);
    }
    Ok(out)
}
