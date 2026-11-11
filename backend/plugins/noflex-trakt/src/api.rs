// SPDX-License-Identifier: AGPL-3.0-only
//
// Pure mapping core of the Trakt plugin: request bodies, response
// parsing, scrobble-event → `/scrobble/*` JSON, and the poll-link
// cursor codec. Nothing in here touches the WIT bindings (the
// `From<types::...>` conversions live in the wasm-only guest module),
// so `cargo test` exercises this file on the host target.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const API_BASE: &str = "https://api.trakt.tv";
pub const DEVICE_CODE_URL: &str = "https://api.trakt.tv/oauth/device/code";
pub const DEVICE_TOKEN_URL: &str = "https://api.trakt.tv/oauth/device/token";
pub const TOKEN_URL: &str = "https://api.trakt.tv/oauth/token";
pub const REVOKE_URL: &str = "https://api.trakt.tv/oauth/revoke";

/// Longest the poll waits between attempts, however much Trakt asks it to
/// slow down: one wait plus one request has to fit the call's deadline.
pub const MAX_POLL_INTERVAL_SECS: u64 = 15;
/// What RFC 8628 has a client add to its interval on `slow_down`.
const SLOW_DOWN_STEP_SECS: u64 = 5;

/// The OAuth redirect_uri Trakt expects on token refresh for
/// out-of-band (device) apps.
pub const OOB_REDIRECT_URI: &str = "urn:ietf:wg:oauth:2.0:oob";

/// Headers every Trakt call carries; `token` adds the bearer
/// authorization for user-authenticated endpoints (scrobble).
pub fn api_headers(client_id: &str, token: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("trakt-api-version".to_string(), "2".to_string()),
        ("trakt-api-key".to_string(), client_id.to_string()),
    ];
    if let Some(token) = token {
        headers.push(("authorization".to_string(), format!("Bearer {token}")));
    }
    headers
}

// -------- Device authorization flow --------

/// Response of `POST /oauth/device/code` (the fields this plugin uses).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    /// Seconds until the code expires.
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
    /// Seconds Trakt wants between polls.
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_expires_in() -> u64 {
    600
}

fn default_interval() -> u64 {
    5
}

pub fn device_code_body(client_id: &str) -> String {
    json!({ "client_id": client_id }).to_string()
}

pub fn parse_device_code(body: &[u8]) -> Result<DeviceCode, String> {
    serde_json::from_slice(body).map_err(|e| format!("parse device-code response: {e}"))
}

/// Body for `POST /oauth/device/token` (one poll attempt).
pub fn device_token_body(device_code: &str, client_id: &str, client_secret: &str) -> String {
    json!({
        "code": device_code,
        "client_id": client_id,
        "client_secret": client_secret,
    })
    .to_string()
}

/// Body for `POST /oauth/token` with the refresh_token grant.
pub fn refresh_body(refresh_token: &str, client_id: &str, client_secret: &str) -> String {
    json!({
        "refresh_token": refresh_token,
        "client_id": client_id,
        "client_secret": client_secret,
        "grant_type": "refresh_token",
        "redirect_uri": OOB_REDIRECT_URI,
    })
    .to_string()
}

/// The token pair Trakt returns from both the device-token poll and the
/// refresh grant.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
}

pub fn parse_token_pair(body: &[u8]) -> Result<TokenPair, String> {
    serde_json::from_slice(body).map_err(|e| format!("parse token response: {e}"))
}

/// Body for `POST /oauth/revoke`.
pub fn revoke_body(token: &str, client_id: &str, client_secret: &str) -> String {
    json!({
        "token": token,
        "client_id": client_id,
        "client_secret": client_secret,
    })
    .to_string()
}

/// What one `/oauth/device/token` answer means for the poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollAnswer {
    /// 200: the user authorized; the body is the token pair.
    Authorized,
    /// 400: not yet.
    Pending,
    /// 429: not yet, and poll less often.
    SlowDown,
    /// 404 invalid code, 409 already used, 410 expired, 418 denied.
    Ended,
    /// Anything else, taken as transient.
    Unexpected,
}

impl PollAnswer {
    pub fn from_status(status: u16) -> Self {
        match status {
            200 => Self::Authorized,
            400 => Self::Pending,
            429 => Self::SlowDown,
            404 | 409 | 410 | 418 => Self::Ended,
            _ => Self::Unexpected,
        }
    }
}

// -------- Poll cursor --------

/// State of one in-flight device-link poll. Crosses `background`
/// invocations twice over: as the `host-tasks.enqueue` payload (produced by
/// `link-start`) and as the `more(...)` cursor the scheduler hands back on
/// every re-invocation. JSON by convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCursor {
    pub user: String,
    pub device_code: String,
    /// Seconds to wait before the next poll.
    pub interval_secs: u64,
    /// Seconds of polling the device code has left. Counts only time spent
    /// waiting between polls, so a chain resumed after a restart may poll
    /// past the real expiry, which Trakt then answers with 410.
    pub remaining_secs: u64,
}

impl PollCursor {
    pub fn new(user: String, device: &DeviceCode) -> Self {
        Self {
            user,
            device_code: device.device_code.clone(),
            interval_secs: device.interval.clamp(1, MAX_POLL_INTERVAL_SECS),
            remaining_secs: device.expires_in,
        }
    }

    /// Spend one interval's wait, returning how long to wait; `None` once
    /// the code has expired.
    pub fn next_wait(&mut self) -> Option<u64> {
        if self.remaining_secs == 0 {
            return None;
        }
        let wait = self.interval_secs.min(self.remaining_secs);
        self.remaining_secs -= wait;
        Some(wait)
    }

    /// Trakt asked to poll less often.
    pub fn slow_down(&mut self) {
        self.interval_secs = (self.interval_secs + SLOW_DOWN_STEP_SECS).min(MAX_POLL_INTERVAL_SECS);
    }

    pub fn encode(&self) -> String {
        // serde_json cannot fail on a plain string struct.
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn decode(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|e| format!("parse poll cursor: {e}"))
    }
}

// -------- Scrobble mapping --------

/// Mirrors the WIT `scrobble-action`; local so this module stays
/// host-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Started,
    Paused,
    Stopped,
}

impl Action {
    /// Trakt endpoint suffix under `/scrobble/`.
    pub fn endpoint(self) -> &'static str {
        match self {
            Action::Started => "start",
            Action::Paused => "pause",
            Action::Stopped => "stop",
        }
    }
}

/// The subset of `work-kind` Trakt knows; everything else is `Other` and
/// skipped by the guest. Only movies and episodes scrobble: `Show` names an
/// episode's series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Movie,
    Show,
    Episode,
    Other,
}

/// The external-id kinds Trakt understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Tmdb,
    Tvdb,
    Imdb,
    Other,
}

/// Progress percent in [0, 100]. Position-derived when both position
/// and runtime are known; otherwise `stopped` counts as fully watched (the
/// host sends a stop without a position only for a title marked played)
/// and everything else starts at zero.
pub fn progress_percent(action: Action, position_ms: Option<u64>, runtime_ms: Option<u64>) -> f32 {
    if let (Some(position), Some(runtime)) = (position_ms, runtime_ms)
        && runtime > 0
    {
        return ((position as f32 / runtime as f32) * 100.0).clamp(0.0, 100.0);
    }
    match action {
        Action::Stopped => 100.0,
        Action::Started | Action::Paused => 0.0,
    }
}

/// The record `value` names as a number: bare, or in the `{prefix}/{n}` path
/// form the metadata plugins store (`movie/603`, `tv/1399`, `series/121361`)
/// for one of `prefixes`. A deeper path (`tv/1399/season/1`) names no
/// record Trakt knows by number.
fn record_number(value: &str, prefixes: &[&str]) -> Option<u64> {
    let value = value.trim();
    if let Ok(n) = value.parse() {
        return Some(n);
    }
    let (prefix, n) = value.split_once('/')?;
    prefixes.contains(&prefix).then(|| n.parse().ok()).flatten()
}

/// Trakt `ids` object for a record of `kind`. TMDb and TVDB values parse
/// from the path forms that kind's records use; IMDb stays a string.
/// Matching is ids-only by design — no title fallback.
fn trakt_ids(ids: &[(IdKind, &str)], kind: MediaKind) -> Map<String, Value> {
    let (tmdb, tvdb): (&[&str], &[&str]) = match kind {
        MediaKind::Movie => (&["movie"], &["movie"]),
        MediaKind::Show => (&["tv"], &["series"]),
        // An episode's stored TMDb and TVDB values are paths through its
        // series, not the episode's own number.
        MediaKind::Episode | MediaKind::Other => (&[], &[]),
    };
    let mut map = Map::new();
    for (id_kind, value) in ids {
        match id_kind {
            IdKind::Tmdb => {
                if let Some(n) = record_number(value, tmdb) {
                    map.insert("tmdb".into(), json!(n));
                }
            }
            IdKind::Tvdb => {
                if let Some(n) = record_number(value, tvdb) {
                    map.insert("tvdb".into(), json!(n));
                }
            }
            IdKind::Imdb => {
                map.insert("imdb".into(), json!(value));
            }
            IdKind::Other => {}
        }
    }
    map
}

/// One scrobble, in the terms Trakt matches on.
#[derive(Debug, Clone, PartialEq)]
pub struct Scrobble<'a> {
    pub kind: MediaKind,
    pub title: &'a str,
    pub year: Option<i32>,
    pub ids: &'a [(IdKind, &'a str)],
    /// An episode's series ids.
    pub show_ids: &'a [(IdKind, &'a str)],
    pub season: Option<u32>,
    pub number: Option<u32>,
    pub progress: f32,
}

/// Body for `/scrobble/{start,pause,stop}`. Movies carry title/year
/// alongside their ids. An episode is named by its show's ids with its
/// season and number when the host knows them, else by its own ids.
/// Returns `None` when the kind is not scrobbleable or no usable id
/// remains — the guest treats that as "skip", never as an error.
pub fn scrobble_body(scrobble: &Scrobble<'_>) -> Option<String> {
    let body = match scrobble.kind {
        MediaKind::Movie => {
            let ids = trakt_ids(scrobble.ids, MediaKind::Movie);
            if ids.is_empty() {
                return None;
            }
            json!({
                "movie": { "title": scrobble.title, "year": scrobble.year, "ids": Value::Object(ids) },
                "progress": scrobble.progress,
            })
        }
        MediaKind::Episode => {
            let show = trakt_ids(scrobble.show_ids, MediaKind::Show);
            match (scrobble.season, scrobble.number) {
                (Some(season), Some(number)) if !show.is_empty() => json!({
                    "show": { "ids": Value::Object(show) },
                    "episode": { "season": season, "number": number },
                    "progress": scrobble.progress,
                }),
                _ => {
                    let ids = trakt_ids(scrobble.ids, MediaKind::Episode);
                    if ids.is_empty() {
                        return None;
                    }
                    json!({
                        "episode": { "ids": Value::Object(ids) },
                        "progress": scrobble.progress,
                    })
                }
            }
        }
        MediaKind::Show | MediaKind::Other => return None,
    };
    Some(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_without_token() {
        let h = api_headers("cid", None);
        assert_eq!(
            h,
            vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("trakt-api-version".to_string(), "2".to_string()),
                ("trakt-api-key".to_string(), "cid".to_string()),
            ]
        );
    }

    #[test]
    fn headers_with_token_add_bearer() {
        let h = api_headers("cid", Some("tok"));
        assert_eq!(h.len(), 4);
        assert_eq!(
            h[3],
            ("authorization".to_string(), "Bearer tok".to_string())
        );
    }

    #[test]
    fn device_code_roundtrip() {
        let body = br#"{
            "device_code": "dcode",
            "user_code": "ABCD-1234",
            "verification_url": "https://trakt.tv/activate",
            "expires_in": 600,
            "interval": 5
        }"#;
        let dc = parse_device_code(body).expect("valid response");
        assert_eq!(dc.device_code, "dcode");
        assert_eq!(dc.user_code, "ABCD-1234");
        assert_eq!(dc.verification_url, "https://trakt.tv/activate");
        assert_eq!((dc.expires_in, dc.interval), (600, 5));
    }

    #[test]
    fn device_code_rejects_garbage() {
        assert!(parse_device_code(b"not json").is_err());
        assert!(parse_device_code(br#"{"user_code":"X"}"#).is_err());
    }

    #[test]
    fn token_pair_roundtrip() {
        let body = br#"{
            "access_token": "acc",
            "token_type": "bearer",
            "expires_in": 7200,
            "refresh_token": "ref",
            "scope": "public",
            "created_at": 1700000000
        }"#;
        let pair = parse_token_pair(body).expect("valid response");
        assert_eq!(
            pair,
            TokenPair {
                access_token: "acc".into(),
                refresh_token: "ref".into(),
            }
        );
    }

    fn device(interval: u64, expires_in: u64) -> DeviceCode {
        DeviceCode {
            device_code: "dcode".into(),
            user_code: "ABCD-1234".into(),
            verification_url: "https://trakt.tv/activate".into(),
            expires_in,
            interval,
        }
    }

    #[test]
    fn poll_cursor_roundtrip() {
        let cursor = PollCursor::new("user-1".into(), &device(5, 600));
        let decoded = PollCursor::decode(&cursor.encode()).expect("valid cursor");
        assert_eq!(decoded, cursor);
    }

    #[test]
    fn polling_waits_trakts_interval_until_the_code_expires() {
        let mut cursor = PollCursor::new("u".into(), &device(5, 12));
        assert_eq!(cursor.next_wait(), Some(5));
        assert_eq!(cursor.next_wait(), Some(5));
        assert_eq!(cursor.next_wait(), Some(2), "the last wait ends at expiry");
        assert_eq!(cursor.next_wait(), None);
    }

    #[test]
    fn slowing_down_lengthens_the_wait_within_the_deadline() {
        let mut cursor = PollCursor::new("u".into(), &device(5, 600));
        cursor.slow_down();
        assert_eq!(cursor.next_wait(), Some(10));
        cursor.slow_down();
        cursor.slow_down();
        assert_eq!(cursor.next_wait(), Some(MAX_POLL_INTERVAL_SECS));
        let eager = PollCursor::new("u".into(), &device(0, 600));
        assert_eq!(eager.interval_secs, 1, "never a busy loop");
    }

    #[test]
    fn poll_answers_follow_trakts_device_token_statuses() {
        assert_eq!(PollAnswer::from_status(200), PollAnswer::Authorized);
        assert_eq!(PollAnswer::from_status(400), PollAnswer::Pending);
        assert_eq!(PollAnswer::from_status(429), PollAnswer::SlowDown);
        for ended in [404, 409, 410, 418] {
            assert_eq!(PollAnswer::from_status(ended), PollAnswer::Ended);
        }
        assert_eq!(PollAnswer::from_status(502), PollAnswer::Unexpected);
    }

    #[test]
    fn poll_cursor_rejects_garbage() {
        assert!(PollCursor::decode("{oops").is_err());
    }

    #[test]
    fn progress_from_position_and_runtime() {
        let p = progress_percent(Action::Stopped, Some(45 * 60_000), Some(90 * 60_000));
        assert!((p - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn progress_clamps_overrun_and_zero_runtime() {
        let p = progress_percent(Action::Stopped, Some(200), Some(100));
        assert_eq!(p, 100.0);
        // A zero runtime falls through to the action default.
        let p = progress_percent(Action::Stopped, Some(200), Some(0));
        assert_eq!(p, 100.0);
        let p = progress_percent(Action::Started, Some(200), Some(0));
        assert_eq!(p, 0.0);
    }

    #[test]
    fn progress_defaults_without_position() {
        assert_eq!(progress_percent(Action::Stopped, None, Some(100)), 100.0);
        assert_eq!(progress_percent(Action::Started, None, None), 0.0);
        assert_eq!(progress_percent(Action::Paused, None, None), 0.0);
    }

    fn scrobble<'a>(
        kind: MediaKind,
        ids: &'a [(IdKind, &'a str)],
        show_ids: &'a [(IdKind, &'a str)],
    ) -> Scrobble<'a> {
        Scrobble {
            kind,
            title: "Alien",
            year: Some(1979),
            ids,
            show_ids,
            season: None,
            number: None,
            progress: 0.0,
        }
    }

    fn body(scrobble: &Scrobble<'_>) -> Value {
        serde_json::from_str(&scrobble_body(scrobble).expect("scrobbleable")).expect("valid JSON")
    }

    #[test]
    fn movie_body_carries_title_year_and_path_style_ids() {
        let v = body(&scrobble(
            MediaKind::Movie,
            &[
                (IdKind::Tmdb, "movie/348"),
                (IdKind::Tvdb, "movie/169"),
                (IdKind::Imdb, "tt0078748"),
            ],
            &[],
        ));
        assert_eq!(v["movie"]["title"], "Alien");
        assert_eq!(v["movie"]["year"], 1979);
        assert_eq!(v["movie"]["ids"]["tmdb"], 348);
        assert_eq!(v["movie"]["ids"]["tvdb"], 169);
        assert_eq!(v["movie"]["ids"]["imdb"], "tt0078748");
        assert_eq!(v["progress"], 0.0);
        let bare = body(&scrobble(MediaKind::Movie, &[(IdKind::Tmdb, "348")], &[]));
        assert_eq!(bare["movie"]["ids"]["tmdb"], 348);
    }

    #[test]
    fn episodes_are_named_by_show_season_and_number() {
        let mut episode = scrobble(
            MediaKind::Episode,
            &[(IdKind::Tmdb, "tv/1399/season/1/episode/2")],
            &[(IdKind::Tmdb, "tv/1399"), (IdKind::Tvdb, "series/121361")],
        );
        episode.season = Some(1);
        episode.number = Some(2);
        episode.progress = 100.0;
        let v = body(&episode);
        assert_eq!(v["show"]["ids"]["tmdb"], 1399);
        assert_eq!(v["show"]["ids"]["tvdb"], 121361);
        assert_eq!(v["episode"]["season"], 1);
        assert_eq!(v["episode"]["number"], 2);
        assert!(v["episode"].get("ids").is_none());
        assert_eq!(v["progress"], 100.0);
    }

    #[test]
    fn an_episode_without_its_show_falls_back_to_its_own_ids() {
        let v = body(&scrobble(
            MediaKind::Episode,
            &[
                (IdKind::Tmdb, "tv/1399/season/1/episode/2"),
                (IdKind::Imdb, "tt1480055"),
            ],
            &[],
        ));
        assert_eq!(v["episode"]["ids"]["imdb"], "tt1480055");
        assert!(
            v["episode"]["ids"].get("tmdb").is_none(),
            "a path through the series is not the episode's TMDb number"
        );
        assert!(v.get("show").is_none());
    }

    #[test]
    fn ids_of_another_record_type_are_dropped() {
        let v = body(&scrobble(
            MediaKind::Movie,
            &[(IdKind::Tmdb, "tv/1399"), (IdKind::Imdb, "tt0078748")],
            &[],
        ));
        assert!(v["movie"]["ids"].get("tmdb").is_none());
        assert_eq!(v["movie"]["ids"]["imdb"], "tt0078748");
    }

    #[test]
    fn no_usable_ids_or_kind_means_skip() {
        assert!(scrobble_body(&scrobble(MediaKind::Movie, &[(IdKind::Other, "x")], &[])).is_none());
        assert!(scrobble_body(&scrobble(MediaKind::Other, &[(IdKind::Tmdb, "1")], &[])).is_none());
        let mut numbered_but_showless = scrobble(
            MediaKind::Episode,
            &[(IdKind::Tvdb, "series/1/season/1/episode/1")],
            &[],
        );
        numbered_but_showless.season = Some(1);
        numbered_but_showless.number = Some(1);
        assert!(scrobble_body(&numbered_but_showless).is_none());
    }

    #[test]
    fn endpoints_match_trakt() {
        assert_eq!(Action::Started.endpoint(), "start");
        assert_eq!(Action::Paused.endpoint(), "pause");
        assert_eq!(Action::Stopped.endpoint(), "stop");
    }
}
