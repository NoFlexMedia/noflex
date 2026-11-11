// SPDX-License-Identifier: AGPL-3.0-only
//
// Pure core of two-way history and ratings sync: the per-user plan, the
// cursor that carries it across `background` runs, Trakt's `/sync/*`
// payloads in both directions, and the timestamp codec. No WIT bindings,
// so `cargo test` exercises it on the host target; `guest` does the I/O.
//
// A sync imports before it exports. The host leaves out of `user-history`
// whatever this plugin imported, and counts a play an import repeats within
// the hour as the one it has, so a local play Trakt already holds (it was
// scrobbled) never comes back for export, and only what Trakt lacks goes up.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::api::{IdKind, MediaKind, trakt_ids};

/// Items per Trakt page; one page is one `background` run.
pub const PAGE_LIMIT: u32 = 100;
/// How far back an incremental sync looks past the last one, for plays
/// that reached either side late.
pub const OVERLAP_MS: i64 = 24 * 60 * 60 * 1000;
/// How often a sync reads everything again: plays someone adds on Trakt
/// with a date before the last sync are only found by a full read.
pub const FULL_EVERY_MS: i64 = 7 * 24 * 60 * 60 * 1000;

// -------- Timestamps --------

/// Milliseconds since the Unix epoch for an ISO 8601 UTC time as Trakt
/// writes them (`2014-09-01T09:10:11.000Z`, fraction optional).
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut clock = clock.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if date.next().is_some()
        || clock.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let millis: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis)
}

/// The ISO 8601 UTC form Trakt takes, to the millisecond.
pub fn format_time(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

// Howard Hinnant's algorithms, proleptic Gregorian.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

// -------- Last activities --------

/// The `/sync/last_activities` times a sync acts on. Any of them moving
/// since the last sync is news; none moving is an idle account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activities {
    pub movies_watched: Option<String>,
    pub episodes_watched: Option<String>,
    pub movies_rated: Option<String>,
    pub shows_rated: Option<String>,
    pub episodes_rated: Option<String>,
}

pub fn parse_last_activities(body: &[u8]) -> Result<Activities, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("parse last activities: {e}"))?;
    let at = |group: &str, field: &str| value[group][field].as_str().map(str::to_string);
    Ok(Activities {
        movies_watched: at("movies", "watched_at"),
        episodes_watched: at("episodes", "watched_at"),
        movies_rated: at("movies", "rated_at"),
        shows_rated: at("shows", "rated_at"),
        episodes_rated: at("episodes", "rated_at"),
    })
}

// -------- Plan and cursor --------

/// What a user's last finished sync left, kept as their `sync_state`
/// secret so it goes when they unlink.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    pub activities: Activities,
    /// When the last sync started; the next looks from a day before.
    pub synced_at: i64,
    /// When the last full sync started.
    pub full_at: i64,
}

impl SyncState {
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn decode(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }
}

/// One stage of a sync, run a page at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    HistoryMovies,
    HistoryEpisodes,
    RatingsMovies,
    RatingsShows,
    RatingsEpisodes,
    /// Send Trakt what this server has and it lacks.
    Export,
}

impl Step {
    /// The Trakt path a reading step pages through; `None` for `Export`.
    pub fn path(self) -> Option<&'static str> {
        match self {
            Step::HistoryMovies => Some("/sync/history/movies"),
            Step::HistoryEpisodes => Some("/sync/history/episodes"),
            Step::RatingsMovies => Some("/sync/ratings/movies"),
            Step::RatingsShows => Some("/sync/ratings/shows"),
            Step::RatingsEpisodes => Some("/sync/ratings/episodes"),
            Step::Export => None,
        }
    }

    fn is_history(self) -> bool {
        matches!(self, Step::HistoryMovies | Step::HistoryEpisodes)
    }
}

/// One user's sync in progress. Crosses `background` runs as the
/// `more(...)` cursor of their chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCursor {
    pub user: String,
    /// When this sync started: the end of the window it reads, so pages
    /// don't shift under it, and the next one's starting point.
    pub started_at: i64,
    /// Read from here on; `None` reads everything.
    pub since: Option<i64>,
    /// Stored when the sync finishes.
    pub activities: Activities,
    pub full_at: i64,
    /// What's left, the current step first.
    pub steps: Vec<Step>,
    /// The Trakt page the current step reads next.
    pub page: u32,
    /// Where the export reads the user's history from next.
    pub export_cursor: Option<String>,
}

impl SyncCursor {
    /// What to do for `user` now, given what their last sync left and
    /// Trakt's activity times. Everything on a first sync or once a week;
    /// otherwise only what moved, then the export, which reads only local
    /// changes and so costs no request when there are none.
    pub fn plan(user: String, now: i64, state: Option<&SyncState>, latest: Activities) -> Self {
        let full = state.is_none_or(|state| now - state.full_at >= FULL_EVERY_MS);
        let steps = match state {
            Some(state) if !full => {
                let was = &state.activities;
                let mut steps = Vec::new();
                if latest.movies_watched != was.movies_watched {
                    steps.push(Step::HistoryMovies);
                }
                if latest.episodes_watched != was.episodes_watched {
                    steps.push(Step::HistoryEpisodes);
                }
                if latest.movies_rated != was.movies_rated {
                    steps.push(Step::RatingsMovies);
                }
                if latest.shows_rated != was.shows_rated {
                    steps.push(Step::RatingsShows);
                }
                if latest.episodes_rated != was.episodes_rated {
                    steps.push(Step::RatingsEpisodes);
                }
                steps.push(Step::Export);
                steps
            }
            _ => vec![
                Step::HistoryMovies,
                Step::HistoryEpisodes,
                Step::RatingsMovies,
                Step::RatingsShows,
                Step::RatingsEpisodes,
                Step::Export,
            ],
        };
        Self {
            user,
            started_at: now,
            since: match state {
                Some(state) if !full => Some(state.synced_at - OVERLAP_MS),
                _ => None,
            },
            activities: latest,
            full_at: if full {
                now
            } else {
                state.map_or(now, |state| state.full_at)
            },
            steps,
            page: 1,
            export_cursor: None,
        }
    }

    /// The URL the current reading step fetches next.
    pub fn page_url(&self, base: &str) -> Option<String> {
        let step = *self.steps.first()?;
        let path = step.path()?;
        let mut url = format!("{base}{path}?page={}&limit={PAGE_LIMIT}", self.page);
        // Ratings take no window: there are few, and a re-rating keeps its item.
        if step.is_history() {
            if let Some(since) = self.since {
                url.push_str(&format!("&start_at={}", format_time(since)));
            }
            url.push_str(&format!("&end_at={}", format_time(self.started_at)));
        }
        Some(url)
    }

    /// Move on after a page of `page_count`: the next page, or the next step.
    pub fn advance(&mut self, page_count: u32) {
        if self.page < page_count {
            self.page += 1;
        } else {
            self.next_step();
        }
    }

    pub fn next_step(&mut self) {
        if !self.steps.is_empty() {
            self.steps.remove(0);
        }
        self.page = 1;
        self.export_cursor = None;
    }

    pub fn finished(&self) -> bool {
        self.steps.is_empty()
    }

    /// What to keep once it has finished.
    pub fn state(&self) -> SyncState {
        SyncState {
            activities: self.activities.clone(),
            synced_at: self.started_at,
            full_at: self.full_at,
        }
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn decode(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }
}

// -------- Reading Trakt --------

/// The ids Trakt gives a movie or show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Ids {
    #[serde(default)]
    pub trakt: Option<u64>,
    #[serde(default)]
    pub tmdb: Option<u64>,
    #[serde(default)]
    pub tvdb: Option<u64>,
    #[serde(default)]
    pub imdb: Option<String>,
}

impl Ids {
    /// The external ids to look a record of `kind` up by, as metadata
    /// providers store them here: TMDb and TheTVDB as paths, IMDb bare.
    pub fn lookups(&self, kind: MediaKind) -> Vec<(IdKind, String)> {
        let (tmdb, tvdb) = match kind {
            MediaKind::Movie => ("movie", "movie"),
            MediaKind::Show => ("tv", "series"),
            MediaKind::Episode | MediaKind::Other => return Vec::new(),
        };
        let mut out = Vec::new();
        if let Some(imdb) = self.imdb.as_ref().filter(|imdb| !imdb.is_empty()) {
            out.push((IdKind::Imdb, imdb.clone()));
        }
        if let Some(n) = self.tmdb {
            out.push((IdKind::Tmdb, format!("{tmdb}/{n}")));
        }
        if let Some(n) = self.tvdb {
            out.push((IdKind::Tvdb, format!("{tvdb}/{n}")));
        }
        out
    }
}

/// What a history entry or rating is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Movie(Ids),
    Show(Ids),
    Episode { show: Ids, season: u32, number: u32 },
}

impl Target {
    fn key(&self) -> String {
        match self {
            Target::Movie(ids) => format!("m:{ids:?}"),
            Target::Show(ids) => format!("s:{ids:?}"),
            Target::Episode {
                show,
                season,
                number,
            } => format!("e:{show:?}:{season}:{number}"),
        }
    }
}

#[derive(Deserialize)]
struct RawEpisode {
    season: u32,
    number: u32,
}

#[derive(Deserialize)]
struct RawItem {
    #[serde(default)]
    watched_at: Option<String>,
    #[serde(default)]
    rated_at: Option<String>,
    #[serde(default)]
    rating: Option<u8>,
    #[serde(default)]
    movie: Option<RawMedia>,
    #[serde(default)]
    show: Option<RawMedia>,
    #[serde(default)]
    episode: Option<RawEpisode>,
}

#[derive(Deserialize)]
struct RawMedia {
    #[serde(default)]
    ids: Ids,
}

fn target(item: &RawItem) -> Option<Target> {
    match (&item.movie, &item.show, &item.episode) {
        (Some(movie), _, _) => Some(Target::Movie(movie.ids.clone())),
        (None, Some(show), Some(episode)) => Some(Target::Episode {
            show: show.ids.clone(),
            season: episode.season,
            number: episode.number,
        }),
        (None, Some(show), None) => Some(Target::Show(show.ids.clone())),
        _ => None,
    }
}

/// A title's plays on one page of `/sync/history`: the latest, and how
/// many the page holds, which is never more than Trakt has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub target: Target,
    pub watched_at: i64,
    pub plays: u32,
}

pub fn parse_history(body: &[u8]) -> Result<Vec<Watched>, String> {
    let items: Vec<RawItem> =
        serde_json::from_slice(body).map_err(|e| format!("parse history: {e}"))?;
    let mut out: Vec<Watched> = Vec::new();
    for item in &items {
        let (Some(target), Some(at)) = (
            target(item),
            item.watched_at.as_deref().and_then(parse_time),
        ) else {
            continue;
        };
        let key = target.key();
        match out.iter_mut().find(|watched| watched.target.key() == key) {
            Some(watched) => {
                watched.watched_at = watched.watched_at.max(at);
                watched.plays += 1;
            }
            None => out.push(Watched {
                target,
                watched_at: at,
                plays: 1,
            }),
        }
    }
    Ok(out)
}

/// One rating from `/sync/ratings`, on Trakt's 1–10 scale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rated {
    pub target: Target,
    pub rating: u8,
    pub rated_at: i64,
}

pub fn parse_ratings(body: &[u8]) -> Result<Vec<Rated>, String> {
    let items: Vec<RawItem> =
        serde_json::from_slice(body).map_err(|e| format!("parse ratings: {e}"))?;
    Ok(items
        .iter()
        .filter_map(|item| {
            Some(Rated {
                target: target(item)?,
                rating: item.rating.filter(|rating| (1..=10).contains(rating))?,
                rated_at: item.rated_at.as_deref().and_then(parse_time)?,
            })
        })
        .collect())
}

/// `X-Pagination-Page-Count`, or one page when Trakt sends none.
pub fn page_count(headers: &[(String, String)]) -> u32 {
    header(headers, "x-pagination-page-count")
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(1)
}

/// Seconds `Retry-After` asks for, at least one.
pub fn retry_after(headers: &[(String, String)]) -> u32 {
    header(headers, "retry-after")
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(1)
        .max(1)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

// -------- Writing Trakt --------

/// One title of this server's history, as `user-history` names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub kind: MediaKind,
    pub ids: Vec<(IdKind, String)>,
    pub show_ids: Vec<(IdKind, String)>,
    pub season: Option<u32>,
    pub number: Option<u32>,
    pub played_at: Option<i64>,
    /// On this server's 0–10 scale, with when it was given.
    pub rating: Option<(f32, i64)>,
}

/// A 0–10 rating on Trakt's whole 1–10 scale.
pub fn trakt_rating(value: f32) -> u8 {
    value.round().clamp(1.0, 10.0) as u8
}

fn as_refs(ids: &[(IdKind, String)]) -> Vec<(IdKind, &str)> {
    ids.iter()
        .map(|(kind, value)| (*kind, value.as_str()))
        .collect()
}

/// The `movies` and `shows` arrays for a `/sync/history` or
/// `/sync/ratings` POST, `fields` giving what each item carries, or `None`
/// when nothing in `items` has it or a usable id.
fn sync_body(
    items: &[Local],
    fields: impl Fn(&Local) -> Option<Map<String, Value>>,
) -> Option<String> {
    let mut movies = Vec::new();
    let mut shows = Vec::new();
    for item in items {
        let Some(fields) = fields(item) else {
            continue;
        };
        match item.kind {
            MediaKind::Movie => {
                let ids = trakt_ids(&as_refs(&item.ids), MediaKind::Movie);
                if ids.is_empty() {
                    continue;
                }
                let mut movie = fields;
                movie.insert("ids".into(), Value::Object(ids));
                movies.push(Value::Object(movie));
            }
            MediaKind::Show => {
                let ids = trakt_ids(&as_refs(&item.ids), MediaKind::Show);
                if ids.is_empty() {
                    continue;
                }
                let mut show = fields;
                show.insert("ids".into(), Value::Object(ids));
                shows.push(Value::Object(show));
            }
            MediaKind::Episode => {
                let ids = trakt_ids(&as_refs(&item.show_ids), MediaKind::Show);
                let (Some(season), Some(number)) = (item.season, item.number) else {
                    continue;
                };
                if ids.is_empty() {
                    continue;
                }
                let mut episode = fields;
                episode.insert("number".into(), json!(number));
                shows.push(json!({
                    "ids": Value::Object(ids),
                    "seasons": [{ "number": season, "episodes": [Value::Object(episode)] }],
                }));
            }
            MediaKind::Other => {}
        }
    }
    if movies.is_empty() && shows.is_empty() {
        return None;
    }
    Some(json!({ "movies": movies, "shows": shows }).to_string())
}

/// `/sync/history` POST body: each played movie and episode with when.
pub fn history_body(items: &[Local]) -> Option<String> {
    sync_body(items, |item| {
        let at = item.played_at?;
        if item.kind == MediaKind::Show {
            return None;
        }
        let mut fields = Map::new();
        fields.insert("watched_at".into(), json!(format_time(at)));
        Some(fields)
    })
}

/// `/sync/ratings` POST body: each rated movie, show and episode.
pub fn ratings_body(items: &[Local]) -> Option<String> {
    sync_body(items, |item| {
        let (value, at) = item.rating?;
        let mut fields = Map::new();
        fields.insert("rating".into(), json!(trakt_rating(value)));
        fields.insert("rated_at".into(), json!(format_time(at)));
        Some(fields)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip_through_trakts_format() {
        let at = parse_time("2014-09-01T09:10:11.000Z").expect("valid");
        assert_eq!(at, 1_409_562_611_000);
        assert_eq!(format_time(at), "2014-09-01T09:10:11.000Z");
        assert_eq!(parse_time("2024-02-29T23:59:59Z"), Some(1_709_251_199_000));
        assert_eq!(format_time(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            parse_time("2014-09-01T09:10:11.5Z"),
            Some(1_409_562_611_500)
        );
        for bad in [
            "2014-09-01 09:10:11Z",
            "2014-13-01T00:00:00Z",
            "2014-09-01T09:10:11+02:00",
            "",
        ] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    // Trakt's example response for `GET /sync/last_activities`.
    const LAST_ACTIVITIES: &[u8] = br#"{
        "all": "2014-11-20T07:01:32.000Z",
        "movies": {
            "watched_at": "2014-11-19T21:42:41.000Z",
            "collected_at": "2014-11-20T06:51:30.000Z",
            "rated_at": "2014-11-19T18:32:29.000Z",
            "watchlisted_at": "2014-11-19T21:42:41.000Z",
            "favorited_at": "2014-11-19T21:42:41.000Z",
            "commented_at": "2014-11-20T06:51:30.000Z",
            "paused_at": "2014-11-20T06:51:30.000Z",
            "hidden_at": "2016-08-20T06:51:30.000Z"
        },
        "episodes": {
            "watched_at": "2014-11-20T06:51:30.000Z",
            "collected_at": "2014-11-19T22:02:41.000Z",
            "rated_at": "2014-11-20T06:51:30.000Z",
            "watchlisted_at": "2014-11-20T06:51:30.000Z",
            "commented_at": "2014-11-20T06:51:30.000Z",
            "paused_at": "2014-11-20T06:51:30.000Z"
        },
        "shows": {
            "rated_at": "2014-11-19T19:50:58.000Z",
            "watchlisted_at": "2014-11-20T06:51:30.000Z",
            "favorited_at": "2014-11-20T06:51:30.000Z",
            "commented_at": "2014-11-20T06:51:30.000Z",
            "hidden_at": "2016-08-20T06:51:30.000Z",
            "dropped_at": "2025-03-13T01:23:45.000Z"
        },
        "seasons": { "rated_at": "2014-11-19T19:54:24.000Z" },
        "account": { "settings_at": "2020-03-04T03:38:09.000Z" }
    }"#;

    // Trakt's example response for `GET /sync/history`, with a second play
    // of the first movie.
    const HISTORY: &[u8] = br#"[
        {
            "id": 1982346,
            "watched_at": "2014-03-31T09:28:53.000Z",
            "action": "scrobble",
            "type": "movie",
            "movie": {
                "title": "The Dark Knight",
                "year": 2008,
                "ids": { "trakt": 4, "slug": "the-dark-knight-2008", "imdb": "tt0468569", "tmdb": 155 }
            }
        },
        {
            "id": 1982347,
            "watched_at": "2014-02-27T09:28:53.000Z",
            "action": "checkin",
            "type": "episode",
            "episode": {
                "season": 2,
                "number": 1,
                "title": "Pawnee Zoo",
                "ids": { "trakt": 251, "tvdb": 797571, "imdb": null, "tmdb": 397629 }
            },
            "show": {
                "title": "Parks and Recreation",
                "year": 2009,
                "ids": { "trakt": 4, "slug": "parks-and-recreation", "tvdb": 84912, "imdb": "tt1266020", "tmdb": 8592 }
            }
        },
        {
            "id": 1982349,
            "watched_at": "2013-06-15T05:54:27.000Z",
            "action": "watch",
            "type": "movie",
            "movie": {
                "title": "The Dark Knight",
                "year": 2008,
                "ids": { "trakt": 4, "slug": "the-dark-knight-2008", "imdb": "tt0468569", "tmdb": 155 }
            }
        }
    ]"#;

    // Trakt's example responses for `GET /sync/ratings/{movies,shows,episodes}`.
    const RATINGS: &[u8] = br#"[
        {
            "rated_at": "2014-09-01T09:10:11.000Z",
            "rating": 10,
            "type": "movie",
            "movie": {
                "title": "TRON: Legacy",
                "year": 2010,
                "ids": { "trakt": 1, "slug": "tron-legacy-2010", "imdb": "tt1104001", "tmdb": 20526 }
            }
        },
        {
            "rated_at": "2014-09-01T09:10:11.000Z",
            "rating": 10,
            "type": "show",
            "show": {
                "title": "Breaking Bad",
                "year": 2008,
                "ids": { "trakt": 1, "slug": "breaking-bad", "tvdb": 81189, "imdb": "tt0903747", "tmdb": 1396 }
            }
        },
        {
            "rated_at": "2014-09-01T09:10:11.000Z",
            "rating": 10,
            "type": "episode",
            "episode": {
                "season": 4,
                "number": 1,
                "title": "Box Cutter",
                "ids": { "trakt": 49, "tvdb": 2639411, "imdb": "tt1683084", "tmdb": 62118 }
            },
            "show": {
                "title": "Breaking Bad",
                "year": 2008,
                "ids": { "trakt": 1, "slug": "breaking-bad", "tvdb": 81189, "imdb": "tt0903747", "tmdb": 1396 }
            }
        }
    ]"#;

    #[test]
    fn last_activities_keep_what_a_sync_acts_on() {
        let activities = parse_last_activities(LAST_ACTIVITIES).expect("valid");
        assert_eq!(
            activities.movies_watched.as_deref(),
            Some("2014-11-19T21:42:41.000Z")
        );
        assert_eq!(
            activities.shows_rated.as_deref(),
            Some("2014-11-19T19:50:58.000Z")
        );
        assert_eq!(
            activities.episodes_rated.as_deref(),
            Some("2014-11-20T06:51:30.000Z")
        );
    }

    #[test]
    fn history_folds_a_titles_plays_on_a_page() {
        let watched = parse_history(HISTORY).expect("valid");
        assert_eq!(watched.len(), 2);
        let knight = &watched[0];
        assert_eq!(knight.plays, 2);
        assert_eq!(
            knight.watched_at,
            parse_time("2014-03-31T09:28:53.000Z").unwrap()
        );
        let Target::Movie(ids) = &knight.target else {
            panic!("a movie");
        };
        assert_eq!(
            ids.lookups(MediaKind::Movie),
            [
                (IdKind::Imdb, "tt0468569".to_string()),
                (IdKind::Tmdb, "movie/155".to_string()),
            ]
        );
        let Target::Episode {
            show,
            season,
            number,
        } = &watched[1].target
        else {
            panic!("an episode");
        };
        assert_eq!((*season, *number), (2, 1));
        assert_eq!(
            show.lookups(MediaKind::Show),
            [
                (IdKind::Imdb, "tt1266020".to_string()),
                (IdKind::Tmdb, "tv/8592".to_string()),
                (IdKind::Tvdb, "series/84912".to_string()),
            ]
        );
    }

    #[test]
    fn ratings_name_movies_shows_and_episodes() {
        let rated = parse_ratings(RATINGS).expect("valid");
        assert_eq!(rated.len(), 3);
        assert!(matches!(rated[0].target, Target::Movie(_)));
        assert!(matches!(rated[1].target, Target::Show(_)));
        assert!(matches!(
            rated[2].target,
            Target::Episode {
                season: 4,
                number: 1,
                ..
            }
        ));
        assert!(rated.iter().all(|r| r.rating == 10));
        assert!(parse_ratings(br#"[{"rating": 11, "rated_at": "2014-09-01T09:10:11.000Z", "movie": {"ids": {}}}]"#)
            .expect("valid")
            .is_empty());
    }

    #[test]
    fn a_first_sync_reads_everything_and_an_idle_one_only_exports() {
        let now = 1_700_000_000_000;
        let activities = parse_last_activities(LAST_ACTIVITIES).unwrap();
        let first = SyncCursor::plan("u".into(), now, None, activities.clone());
        assert_eq!(first.steps.len(), 6);
        assert_eq!(first.since, None);
        assert_eq!(first.full_at, now);

        let state = SyncState {
            activities: activities.clone(),
            synced_at: now - 900_000,
            full_at: now - 86_400_000,
        };
        let idle = SyncCursor::plan("u".into(), now, Some(&state), activities.clone());
        assert_eq!(idle.steps, [Step::Export]);
        assert_eq!(idle.since, Some(state.synced_at - OVERLAP_MS));
        assert_eq!(idle.full_at, state.full_at, "an incremental sync keeps it");

        let mut moved = activities.clone();
        moved.episodes_watched = Some("2024-01-01T00:00:00.000Z".into());
        let busy = SyncCursor::plan("u".into(), now, Some(&state), moved);
        assert_eq!(busy.steps, [Step::HistoryEpisodes, Step::Export]);

        let stale = SyncState {
            full_at: now - FULL_EVERY_MS,
            ..state
        };
        assert_eq!(
            SyncCursor::plan("u".into(), now, Some(&stale), activities).since,
            None,
            "a week on, it reads everything again"
        );
    }

    #[test]
    fn history_pages_are_windowed_and_ratings_are_not() {
        let mut cursor = SyncCursor::plan(
            "u".into(),
            1_409_562_611_000,
            Some(&SyncState {
                activities: Activities::default(),
                synced_at: 1_409_562_611_000 + OVERLAP_MS - 1000,
                full_at: 1_409_562_611_000,
            }),
            Activities {
                movies_watched: Some("x".into()),
                movies_rated: Some("y".into()),
                ..Activities::default()
            },
        );
        assert_eq!(
            cursor.page_url("https://api.trakt.tv").as_deref(),
            Some(
                "https://api.trakt.tv/sync/history/movies?page=1&limit=100\
                 &start_at=2014-09-01T09:10:10.000Z&end_at=2014-09-01T09:10:11.000Z"
            )
        );
        cursor.advance(2);
        assert_eq!(cursor.page, 2);
        cursor.advance(2);
        assert_eq!(cursor.steps[0], Step::RatingsMovies);
        assert_eq!(
            cursor.page_url("https://api.trakt.tv").as_deref(),
            Some("https://api.trakt.tv/sync/ratings/movies?page=1&limit=100")
        );
        cursor.next_step();
        assert_eq!(
            cursor.page_url("https://api.trakt.tv"),
            None,
            "export reads no page"
        );
        cursor.next_step();
        assert!(cursor.finished());
        assert_eq!(SyncCursor::decode(&cursor.encode()), Some(cursor.clone()));
        assert_eq!(cursor.state().synced_at, 1_409_562_611_000);
    }

    #[test]
    fn pagination_and_retry_headers_are_read_case_insensitively() {
        let headers = vec![
            ("X-Pagination-Page-Count".to_string(), "7".to_string()),
            ("retry-after".to_string(), "12".to_string()),
        ];
        assert_eq!(page_count(&headers), 7);
        assert_eq!(retry_after(&headers), 12);
        assert_eq!(page_count(&[]), 1);
        assert_eq!(retry_after(&[]), 1);
    }

    fn local(kind: MediaKind) -> Local {
        Local {
            kind,
            ids: Vec::new(),
            show_ids: Vec::new(),
            season: None,
            number: None,
            played_at: None,
            rating: None,
        }
    }

    #[test]
    fn history_export_names_movies_by_id_and_episodes_by_show() {
        let at = parse_time("2014-09-01T09:10:11.000Z").unwrap();
        let movie = Local {
            ids: vec![
                (IdKind::Tmdb, "movie/272".into()),
                (IdKind::Imdb, "tt0372784".into()),
            ],
            played_at: Some(at),
            ..local(MediaKind::Movie)
        };
        let episode = Local {
            show_ids: vec![(IdKind::Tvdb, "series/80337".into())],
            season: Some(1),
            number: Some(2),
            played_at: Some(at),
            ..local(MediaKind::Episode)
        };
        let unplayed = Local {
            ids: vec![(IdKind::Tmdb, "movie/1".into())],
            ..local(MediaKind::Movie)
        };
        let body: Value =
            serde_json::from_str(&history_body(&[movie, episode, unplayed]).expect("body"))
                .unwrap();
        assert_eq!(
            body,
            json!({
                "movies": [{
                    "ids": { "tmdb": 272, "imdb": "tt0372784" },
                    "watched_at": "2014-09-01T09:10:11.000Z"
                }],
                "shows": [{
                    "ids": { "tvdb": 80337 },
                    "seasons": [{ "number": 1, "episodes": [{
                        "number": 2,
                        "watched_at": "2014-09-01T09:10:11.000Z"
                    }] }]
                }]
            })
        );
        assert_eq!(history_body(&[local(MediaKind::Movie)]), None);
    }

    #[test]
    fn ratings_export_rounds_onto_trakts_scale() {
        let at = parse_time("2014-09-01T09:10:11.000Z").unwrap();
        let show = Local {
            ids: vec![(IdKind::Tmdb, "tv/1396".into())],
            rating: Some((7.6, at)),
            ..local(MediaKind::Show)
        };
        let body: Value = serde_json::from_str(&ratings_body(&[show]).expect("body")).unwrap();
        assert_eq!(
            body["shows"][0],
            json!({ "ids": { "tmdb": 1396 }, "rating": 8, "rated_at": "2014-09-01T09:10:11.000Z" })
        );
        assert_eq!(trakt_rating(0.2), 1);
        assert_eq!(trakt_rating(10.0), 10);
    }
}
