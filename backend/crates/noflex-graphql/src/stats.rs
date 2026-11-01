// SPDX-License-Identifier: AGPL-3.0-only
//
// Play history and viewing statistics. The player's progress
// heartbeats (`reportProgress`) are folded into `PlaySession` rows
// here; the admin stats queries then roll those rows up in memory.
// Rolling up in Rust keeps the Storage trait to plain filtered reads,
// so every backend gets identical bucketing and rounding for free.

use std::{collections::HashMap, sync::Arc};

use async_graphql::{
    ComplexObject, Context, Enum, ID, Object, SimpleObject, connection::Connection,
};
use noflex_core::{LibraryId, PlaySessionId, PlayTarget, UserId, WorkId};
use noflex_storage::{
    Storage,
    domain::{Library, PlaySession, User, Work},
    error::StorageResult,
    query::PlaySessionQuery,
};
use time::{Duration, Month, OffsetDateTime, Time, UtcOffset};

use crate::{
    access::{request_user, require_admin},
    paging,
};

/// A heartbeat this long after the previous one opens a new viewing
/// instead of continuing it.
const REOPEN_AFTER: Duration = Duration::minutes(30);
/// The longest gap between two playing heartbeats that counts as
/// watching. The player beats every 15 s; anything longer is a
/// suspended tab or a dropped connection, not viewing time.
const MAX_COUNTED_GAP: Duration = Duration::minutes(1);
/// Parent chains are series → season → episode and artist → album →
/// track; the bound only guards against a corrupt cycle.
const MAX_ANCESTRY: usize = 8;

/// Fold one progress heartbeat into `user`'s play history.
///
/// A heartbeat continues the user's open viewing of the same work when
/// one was seen within [`REOPEN_AFTER`]; otherwise a non-stop heartbeat
/// starts a new viewing and a stop with nothing open is ignored. An
/// edition target is recorded against its work; a target that no longer
/// resolves records nothing.
pub(crate) async fn record_heartbeat(
    storage: &dyn Storage,
    user: UserId,
    target: PlayTarget,
    position_ms: i64,
    state: crate::playback::PlaybackState,
    device: impl AsyncFnOnce() -> Option<String>,
    now: OffsetDateTime,
) -> StorageResult<()> {
    use crate::playback::PlaybackState;

    let work_id = match target {
        PlayTarget::Work(id) => id,
        PlayTarget::Edition(id) => match storage.get_edition(id).await? {
            Some(edition) => edition.work_id,
            None => return Ok(()),
        },
    };
    let position_ms = u64::try_from(position_ms).unwrap_or(0);
    let playing = state == PlaybackState::Playing;
    let ended = state == PlaybackState::Stopped;

    if let Some(mut session) = storage
        .open_play_session(user, work_id, now - REOPEN_AFTER)
        .await?
    {
        if session.playing {
            session.watched_ms += counted_ms(now - session.last_seen_at);
        }
        session.last_seen_at = now;
        session.position_ms = position_ms;
        session.playing = playing;
        session.ended = ended;
        return storage.upsert_play_session(&session).await;
    }
    if ended {
        return Ok(());
    }
    let Some(work) = storage.get_work(work_id).await? else {
        return Ok(());
    };
    let root = root_of(storage, &work).await?;
    storage
        .upsert_play_session(&PlaySession {
            id: PlaySessionId::new(),
            user_id: user,
            work_id,
            root_work_id: root,
            library_id: work.library_id,
            started_at: now,
            last_seen_at: now,
            watched_ms: 0,
            position_ms,
            playing,
            ended: false,
            device: device().await,
        })
        .await
}

fn counted_ms(gap: Duration) -> u64 {
    u64::try_from(
        gap.clamp(Duration::ZERO, MAX_COUNTED_GAP)
            .whole_milliseconds(),
    )
    .unwrap_or(0)
}

async fn root_of(storage: &dyn Storage, work: &Work) -> StorageResult<WorkId> {
    let mut root = work.id;
    let mut parent = work.parent_id;
    for _ in 0..MAX_ANCESTRY {
        let Some(id) = parent else { break };
        let Some(next) = storage.get_work(id).await? else {
            break;
        };
        root = next.id;
        parent = next.parent_id;
    }
    Ok(root)
}

// ---- Ranges and buckets --------------------------------------------------

/// A reporting window ending now. Windows are aligned to the chart's
/// buckets in the caller's offset, so "last 7 days" is today plus the
/// six days before it rather than a rolling 168 hours.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsRange {
    #[graphql(name = "LAST_24_HOURS")]
    Last24Hours,
    #[graphql(name = "LAST_7_DAYS")]
    Last7Days,
    #[graphql(name = "LAST_30_DAYS")]
    Last30Days,
    #[graphql(name = "LAST_90_DAYS")]
    Last90Days,
    LastYear,
    AllTime,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsBucket {
    Hour,
    Day,
    Week,
    Month,
}

impl StatsRange {
    fn bucket(self) -> StatsBucket {
        match self {
            Self::Last24Hours => StatsBucket::Hour,
            Self::Last7Days | Self::Last30Days => StatsBucket::Day,
            Self::Last90Days => StatsBucket::Week,
            Self::LastYear | Self::AllTime => StatsBucket::Month,
        }
    }

    fn buckets(self) -> Option<i64> {
        match self {
            Self::Last24Hours => Some(24),
            Self::Last7Days => Some(7),
            Self::Last30Days => Some(30),
            Self::Last90Days => Some(13),
            Self::LastYear => Some(12),
            Self::AllTime => None,
        }
    }

    /// First instant of the window, `None` for all time.
    fn start(self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        let bucket = self.bucket();
        self.buckets()
            .map(|n| bucket.step(bucket.floor(now), -(n - 1)))
    }
}

impl StatsBucket {
    fn floor(self, at: OffsetDateTime) -> OffsetDateTime {
        let midnight = at.replace_time(Time::MIDNIGHT);
        match self {
            Self::Hour => {
                at.replace_time(Time::from_hms(at.hour(), 0, 0).unwrap_or(Time::MIDNIGHT))
            }
            Self::Day => midnight,
            Self::Week => {
                midnight - Duration::days(i64::from(at.weekday().number_days_from_monday()))
            }
            Self::Month => midnight.replace_day(1).unwrap_or(midnight),
        }
    }

    fn step(self, at: OffsetDateTime, n: i64) -> OffsetDateTime {
        match self {
            Self::Hour => at + Duration::hours(n),
            Self::Day => at + Duration::days(n),
            Self::Week => at + Duration::weeks(n),
            Self::Month => add_months(at, n),
        }
    }
}

/// `at` moved by `n` calendar months; `at` must be a month start.
fn add_months(at: OffsetDateTime, n: i64) -> OffsetDateTime {
    let index = i64::from(at.year()) * 12 + i64::from(u8::from(at.month())) - 1 + n;
    let year = i32::try_from(index.div_euclid(12)).unwrap_or(at.year());
    let month = u8::try_from(index.rem_euclid(12) + 1)
        .ok()
        .and_then(|m| Month::try_from(m).ok())
        .unwrap_or(at.month());
    at.replace_year(year)
        .and_then(|d| d.replace_month(month))
        .unwrap_or(at)
}

fn offset_from_minutes(minutes: Option<i32>) -> async_graphql::Result<UtcOffset> {
    let minutes = minutes.unwrap_or(0);
    if !(-18 * 60..=18 * 60).contains(&minutes) {
        return Err(async_graphql::Error::new(
            "utcOffsetMinutes must be within ±18 hours",
        ));
    }
    UtcOffset::from_whole_seconds(minutes * 60)
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

// ---- Rollups -------------------------------------------------------------

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
#[graphql(complex)]
pub struct LibraryPlayStats {
    #[graphql(skip)]
    pub library_id: LibraryId,
    pub plays: u64,
    pub watched_ms: u64,
}

#[ComplexObject]
impl LibraryPlayStats {
    async fn library(&self, ctx: &Context<'_>) -> async_graphql::Result<Option<Library>> {
        storage(ctx)?
            .get_library(self.library_id)
            .await
            .map_err(gql_err)
    }
}

/// One user's viewing in a window, with a per-library split.
#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
#[graphql(complex)]
pub struct UserPlayStats {
    #[graphql(skip)]
    pub user_id: UserId,
    pub plays: u64,
    pub watched_ms: u64,
    pub last_played_at: OffsetDateTime,
    /// Busiest library first.
    pub libraries: Vec<LibraryPlayStats>,
}

#[ComplexObject]
impl UserPlayStats {
    async fn user(&self, ctx: &Context<'_>) -> async_graphql::Result<Option<User>> {
        storage(ctx)?.get_user(self.user_id).await.map_err(gql_err)
    }
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
pub struct PlayHistoryPoint {
    /// Bucket start, in the requested offset.
    pub start: OffsetDateTime,
    pub plays: u64,
    pub watched_ms: u64,
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
pub struct PlayHistoryChart {
    pub bucket: StatsBucket,
    /// Every bucket in the window in order, empty ones included.
    pub points: Vec<PlayHistoryPoint>,
    pub total_plays: u64,
    pub total_watched_ms: u64,
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
#[graphql(complex)]
pub struct TopPlayedItem {
    #[graphql(skip)]
    pub work_id: WorkId,
    pub plays: u64,
    pub watched_ms: u64,
    /// Distinct users who played it.
    pub users: u64,
}

#[ComplexObject]
impl TopPlayedItem {
    /// The movie, series or artist; null once deleted or hidden from
    /// the viewer.
    async fn work(&self, ctx: &Context<'_>) -> async_graphql::Result<Option<Work>> {
        let storage = storage(ctx)?;
        let user = request_user(ctx)?;
        let Some(work) = storage.get_work(self.work_id).await.map_err(gql_err)? else {
            return Ok(None);
        };
        noflex_storage::access::visible_work(storage.as_ref(), user, work)
            .await
            .map_err(gql_err)
    }
}

#[derive(SimpleObject, Debug, Clone, PartialEq, Eq)]
#[graphql(complex)]
pub struct TopPlayedLibrary {
    #[graphql(skip)]
    pub library_id: LibraryId,
    pub plays: u64,
    /// Most played first, rolled up to the top of each parent chain.
    pub items: Vec<TopPlayedItem>,
}

#[ComplexObject]
impl TopPlayedLibrary {
    async fn library(&self, ctx: &Context<'_>) -> async_graphql::Result<Option<Library>> {
        storage(ctx)?
            .get_library(self.library_id)
            .await
            .map_err(gql_err)
    }
}

/// Users by watched time, then plays; libraries within each user the
/// same way.
pub fn top_users(sessions: &[PlaySession]) -> Vec<UserPlayStats> {
    let mut users: HashMap<UserId, (UserPlayStats, HashMap<LibraryId, LibraryPlayStats>)> =
        HashMap::new();
    for s in sessions {
        let (user, libraries) = users.entry(s.user_id).or_insert_with(|| {
            (
                UserPlayStats {
                    user_id: s.user_id,
                    plays: 0,
                    watched_ms: 0,
                    last_played_at: s.last_seen_at,
                    libraries: Vec::new(),
                },
                HashMap::new(),
            )
        });
        user.plays += 1;
        user.watched_ms += s.watched_ms;
        user.last_played_at = user.last_played_at.max(s.last_seen_at);
        let library = libraries
            .entry(s.library_id)
            .or_insert_with(|| LibraryPlayStats {
                library_id: s.library_id,
                plays: 0,
                watched_ms: 0,
            });
        library.plays += 1;
        library.watched_ms += s.watched_ms;
    }
    let mut out: Vec<UserPlayStats> = users
        .into_values()
        .map(|(mut user, libraries)| {
            user.libraries = libraries.into_values().collect();
            user.libraries.sort_by(|a, b| {
                (b.watched_ms, b.plays, a.library_id.0).cmp(&(
                    a.watched_ms,
                    a.plays,
                    b.library_id.0,
                ))
            });
            user
        })
        .collect();
    out.sort_by(|a, b| {
        (b.watched_ms, b.plays, a.user_id.0).cmp(&(a.watched_ms, a.plays, b.user_id.0))
    });
    out
}

/// Bucket `sessions` by `started_at` from `start` (a bucket start in
/// the chart's offset) through `now`.
pub fn history_chart(
    sessions: &[PlaySession],
    bucket: StatsBucket,
    start: OffsetDateTime,
    now: OffsetDateTime,
) -> PlayHistoryChart {
    let mut points = Vec::new();
    let mut at = start;
    while at <= now {
        points.push(PlayHistoryPoint {
            start: at,
            plays: 0,
            watched_ms: 0,
        });
        at = bucket.step(at, 1);
    }
    let offset = start.offset();
    for s in sessions {
        let floor = bucket.floor(s.started_at.to_offset(offset));
        if let Ok(i) = points.binary_search_by(|p| p.start.cmp(&floor)) {
            points[i].plays += 1;
            points[i].watched_ms += s.watched_ms;
        }
    }
    PlayHistoryChart {
        bucket,
        total_plays: points.iter().map(|p| p.plays).sum(),
        total_watched_ms: points.iter().map(|p| p.watched_ms).sum(),
        points,
    }
}

/// Per library, the `limit` most played roots by plays, then watched
/// time. Libraries by total plays.
pub fn top_played(sessions: &[PlaySession], limit: usize) -> Vec<TopPlayedLibrary> {
    struct Acc {
        item: TopPlayedItem,
        users: Vec<UserId>,
    }
    let mut libraries: HashMap<LibraryId, HashMap<WorkId, Acc>> = HashMap::new();
    for s in sessions {
        let acc = libraries
            .entry(s.library_id)
            .or_default()
            .entry(s.root_work_id)
            .or_insert_with(|| Acc {
                item: TopPlayedItem {
                    work_id: s.root_work_id,
                    plays: 0,
                    watched_ms: 0,
                    users: 0,
                },
                users: Vec::new(),
            });
        acc.item.plays += 1;
        acc.item.watched_ms += s.watched_ms;
        if !acc.users.contains(&s.user_id) {
            acc.users.push(s.user_id);
        }
    }
    let mut out: Vec<TopPlayedLibrary> = libraries
        .into_iter()
        .map(|(library_id, works)| {
            let mut items: Vec<TopPlayedItem> = works
                .into_values()
                .map(|acc| TopPlayedItem {
                    users: acc.users.len() as u64,
                    ..acc.item
                })
                .collect();
            let plays = items.iter().map(|i| i.plays).sum();
            items.sort_by(|a, b| {
                (b.plays, b.watched_ms, a.work_id.0).cmp(&(a.plays, a.watched_ms, b.work_id.0))
            });
            items.truncate(limit);
            TopPlayedLibrary {
                library_id,
                plays,
                items,
            }
        })
        .collect();
    out.sort_by(|a, b| (b.plays, a.library_id.0).cmp(&(a.plays, b.library_id.0)));
    out
}

// ---- Resolvers -----------------------------------------------------------

#[derive(Default)]
pub struct StatsQuery;

#[Object]
impl StatsQuery {
    /// Users ranked by watched time over `range`. Admin only.
    /// `utcOffsetMinutes` aligns the window to the viewer's day.
    async fn top_users(
        &self,
        ctx: &Context<'_>,
        range: StatsRange,
        utc_offset_minutes: Option<i32>,
    ) -> async_graphql::Result<Vec<UserPlayStats>> {
        require_admin(ctx)?;
        let query = window(range, utc_offset_minutes, None, None)?;
        let sessions = storage(ctx)?
            .play_sessions_matching(&query)
            .await
            .map_err(gql_err)?;
        Ok(top_users(&sessions))
    }

    /// Plays and watched time per bucket over `range`, optionally for one
    /// library and/or one user. Admin only. The bucket size follows the
    /// range: hours for a day, days up to a month, weeks for 90 days,
    /// months beyond. All time starts at the first recorded play.
    async fn play_history_chart(
        &self,
        ctx: &Context<'_>,
        range: StatsRange,
        library_id: Option<ID>,
        user_id: Option<ID>,
        utc_offset_minutes: Option<i32>,
    ) -> async_graphql::Result<PlayHistoryChart> {
        require_admin(ctx)?;
        let offset = offset_from_minutes(utc_offset_minutes)?;
        let now = OffsetDateTime::now_utc().to_offset(offset);
        let query = window(
            range,
            utc_offset_minutes,
            parse_id(library_id, LibraryId::from_uuid)?,
            parse_id(user_id, UserId::from_uuid)?,
        )?;
        let sessions = storage(ctx)?
            .play_sessions_matching(&query)
            .await
            .map_err(gql_err)?;
        let bucket = range.bucket();
        let start = match query.since {
            Some(since) => since.to_offset(offset),
            None => {
                let first = sessions.iter().map(|s| s.started_at).min().unwrap_or(now);
                // At least a year of months, so a young server still
                // draws a meaningful axis.
                bucket
                    .floor(first.to_offset(offset))
                    .min(bucket.step(bucket.floor(now), -11))
            }
        };
        Ok(history_chart(&sessions, bucket, start, now))
    }

    /// The most played movies, series and artists per library over
    /// `range`. Episodes and tracks count toward their series or
    /// artist. Admin only.
    async fn top_played(
        &self,
        ctx: &Context<'_>,
        range: StatsRange,
        library_id: Option<ID>,
        utc_offset_minutes: Option<i32>,
        #[graphql(default = 10)] limit: i32,
    ) -> async_graphql::Result<Vec<TopPlayedLibrary>> {
        require_admin(ctx)?;
        let mut query = window(range, utc_offset_minutes, None, None)?;
        query.library = parse_id(library_id, LibraryId::from_uuid)?;
        let sessions = storage(ctx)?
            .play_sessions_matching(&query)
            .await
            .map_err(gql_err)?;
        let limit = usize::try_from(limit.clamp(1, 100)).unwrap_or(10);
        Ok(top_played(&sessions, limit))
    }

    /// Viewings newest first. Admins may filter by any user; everyone
    /// else sees only their own, and passing another `userId` is an
    /// error.
    #[graphql(complexity = "paging::cost(first, 50, 200, child_complexity)")]
    #[allow(clippy::too_many_arguments)] // one argument per GraphQL filter
    async fn play_sessions(
        &self,
        ctx: &Context<'_>,
        range: Option<StatsRange>,
        user_id: Option<ID>,
        library_id: Option<ID>,
        root_work_id: Option<ID>,
        utc_offset_minutes: Option<i32>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, PlaySession>> {
        let caller = request_user(ctx)?;
        let requested = parse_id(user_id, UserId::from_uuid)?;
        let user = match requested {
            Some(id) if !caller.is_admin && id != caller.user_id => {
                return Err(async_graphql::Error::new("administrator required"));
            }
            Some(id) => Some(id),
            None if caller.is_admin => None,
            None => Some(caller.user_id),
        };
        let mut query = window(
            range.unwrap_or(StatsRange::AllTime),
            utc_offset_minutes,
            parse_id(library_id, LibraryId::from_uuid)?,
            user,
        )?;
        query.root_work = parse_id(root_work_id, WorkId::from_uuid)?;
        let window = paging::Window::new(first, after, 50, 200)?;
        let resumed = window.after.is_some();
        let page = storage(ctx)?
            .list_play_sessions(&query, window.first, window.after)
            .await
            .map_err(gql_err)?;
        Ok(paging::from_item_page(page, resumed))
    }
}

fn window(
    range: StatsRange,
    utc_offset_minutes: Option<i32>,
    library: Option<LibraryId>,
    user: Option<UserId>,
) -> async_graphql::Result<PlaySessionQuery> {
    let offset = offset_from_minutes(utc_offset_minutes)?;
    Ok(PlaySessionQuery {
        user,
        library,
        root_work: None,
        since: range.start(OffsetDateTime::now_utc().to_offset(offset)),
        until: None,
    })
}

fn parse_id<T>(id: Option<ID>, wrap: fn(uuid::Uuid) -> T) -> async_graphql::Result<Option<T>> {
    id.map(|id| {
        id.parse::<uuid::Uuid>()
            .map(wrap)
            .map_err(|e| async_graphql::Error::new(format!("invalid id: {e}")))
    })
    .transpose()
}

fn storage<'a>(ctx: &'a Context<'_>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    fn session(
        user: UserId,
        library: LibraryId,
        root: WorkId,
        at: OffsetDateTime,
        ms: u64,
    ) -> PlaySession {
        PlaySession {
            id: PlaySessionId::new(),
            user_id: user,
            work_id: root,
            root_work_id: root,
            library_id: library,
            started_at: at,
            last_seen_at: at,
            watched_ms: ms,
            position_ms: 0,
            playing: false,
            ended: true,
            device: None,
        }
    }

    #[test]
    fn windows_align_to_buckets() {
        let now = datetime!(2026-11-04 15:42 +1);
        assert_eq!(
            StatsRange::Last24Hours.start(now),
            Some(datetime!(2026-11-03 16:00 +1))
        );
        assert_eq!(
            StatsRange::Last7Days.start(now),
            Some(datetime!(2026-10-29 00:00 +1))
        );
        // 2026-11-04 is a Wednesday; weeks start on Monday.
        assert_eq!(
            StatsRange::Last90Days.start(now),
            Some(datetime!(2026-08-10 00:00 +1))
        );
        assert_eq!(
            StatsRange::LastYear.start(now),
            Some(datetime!(2025-12-01 00:00 +1))
        );
        assert_eq!(StatsRange::AllTime.start(now), None);
        assert_eq!(
            add_months(datetime!(2026-01-01 00:00 UTC), -1),
            datetime!(2025-12-01 00:00 UTC)
        );
    }

    #[test]
    fn counted_gap_is_capped() {
        assert_eq!(counted_ms(Duration::seconds(15)), 15_000);
        assert_eq!(counted_ms(Duration::hours(3)), 60_000);
        assert_eq!(counted_ms(Duration::seconds(-5)), 0);
    }

    #[test]
    fn chart_fills_empty_buckets_in_offset() {
        let (u, l, w) = (UserId::new(), LibraryId::new(), WorkId::new());
        let now = datetime!(2026-11-04 12:00 -5);
        let start = StatsRange::Last7Days.start(now).expect("bounded");
        let sessions = [
            // 02:00 UTC on the 4th is still the 3rd at -05:00.
            session(u, l, w, datetime!(2026-11-04 02:00 UTC), 1_000),
            session(u, l, w, datetime!(2026-11-04 15:00 UTC), 2_000),
        ];
        let chart = history_chart(&sessions, StatsBucket::Day, start, now);
        assert_eq!(chart.points.len(), 7);
        let days: Vec<(u8, u64)> = chart
            .points
            .iter()
            .map(|p| (p.start.day(), p.watched_ms))
            .collect();
        assert_eq!(
            days,
            vec![
                (29, 0),
                (30, 0),
                (31, 0),
                (1, 0),
                (2, 0),
                (3, 1_000),
                (4, 2_000)
            ]
        );
        assert_eq!((chart.total_plays, chart.total_watched_ms), (2, 3_000));
    }

    #[test]
    fn top_users_and_top_played_roll_up() {
        let (alice, bob) = (UserId::new(), UserId::new());
        let (films, tv) = (LibraryId::new(), LibraryId::new());
        let (movie, show) = (WorkId::new(), WorkId::new());
        let at = datetime!(2026-11-01 20:00 UTC);
        let sessions = [
            session(alice, films, movie, at, 7_200_000),
            session(alice, tv, show, at, 1_800_000),
            session(alice, tv, show, at + Duration::hours(1), 1_800_000),
            session(bob, tv, show, at, 600_000),
        ];
        let users = top_users(&sessions);
        assert_eq!(
            users.iter().map(|u| u.user_id).collect::<Vec<_>>(),
            vec![alice, bob]
        );
        assert_eq!((users[0].plays, users[0].watched_ms), (3, 10_800_000));
        assert_eq!(
            users[0]
                .libraries
                .iter()
                .map(|l| (l.library_id, l.plays))
                .collect::<Vec<_>>(),
            vec![(films, 1), (tv, 2)]
        );
        assert_eq!(users[0].last_played_at, at + Duration::hours(1));

        let top = top_played(&sessions, 10);
        assert_eq!(
            top.iter()
                .map(|l| (l.library_id, l.plays))
                .collect::<Vec<_>>(),
            vec![(tv, 3), (films, 1)]
        );
        assert_eq!((top[0].items[0].work_id, top[0].items[0].users), (show, 2));
    }
}
