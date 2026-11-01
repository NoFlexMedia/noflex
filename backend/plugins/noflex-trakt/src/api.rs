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

// -------- Poll cursor --------

/// State of one in-flight device-link poll. Crosses `background`
/// invocations twice over: as the `host-tasks.enqueue` payload (attempt
/// 0, produced by `link-start`) and as the `more(...)` cursor the
/// scheduler hands back on every re-invocation. JSON by convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCursor {
    pub user: String,
    pub device_code: String,
    pub attempt: u32,
}

impl PollCursor {
    pub fn new(user: String, device_code: String) -> Self {
        Self {
            user,
            device_code,
            attempt: 0,
        }
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

/// The subset of `work-kind` Trakt can scrobble; everything else is
/// `Other` and skipped by the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Movie,
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
/// and runtime are known; otherwise `stopped` counts as fully watched
/// (the host emits session-stop without a position) and everything
/// else starts at zero.
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

/// Trakt `ids` object from the work's external ids. tmdb/tvdb are
/// numeric and dropped when unparseable; imdb stays a string. Matching
/// is ids-only by design — no title fallback.
fn trakt_ids(ids: &[(IdKind, &str)]) -> Map<String, Value> {
    let mut map = Map::new();
    for (kind, value) in ids {
        match kind {
            IdKind::Tmdb => {
                if let Ok(n) = value.parse::<u64>() {
                    map.insert("tmdb".into(), json!(n));
                }
            }
            IdKind::Tvdb => {
                if let Ok(n) = value.parse::<u64>() {
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

/// Body for `/scrobble/{start,pause,stop}`. Movies carry title/year
/// alongside the ids; episodes are keyed by ids alone (Trakt resolves
/// the show from them). Returns `None` when the kind is not
/// scrobbleable or no usable external id remains — the guest treats
/// that as "skip", never as an error.
pub fn scrobble_body(
    kind: MediaKind,
    title: &str,
    year: Option<i32>,
    ids: &[(IdKind, &str)],
    progress: f32,
) -> Option<String> {
    let ids = trakt_ids(ids);
    if ids.is_empty() {
        return None;
    }
    let body = match kind {
        MediaKind::Movie => json!({
            "movie": { "title": title, "year": year, "ids": Value::Object(ids) },
            "progress": progress,
        }),
        MediaKind::Episode => json!({
            "episode": { "ids": Value::Object(ids) },
            "progress": progress,
        }),
        MediaKind::Other => return None,
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

    #[test]
    fn poll_cursor_roundtrip() {
        let cursor = PollCursor::new("user-1".into(), "dcode".into());
        assert_eq!(cursor.attempt, 0);
        let decoded = PollCursor::decode(&cursor.encode()).expect("valid cursor");
        assert_eq!(decoded, cursor);
        let bumped = PollCursor {
            attempt: 7,
            ..cursor
        };
        let decoded = PollCursor::decode(&bumped.encode()).expect("valid cursor");
        assert_eq!(decoded.attempt, 7);
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

    #[test]
    fn movie_body_carries_title_year_ids() {
        let body = scrobble_body(
            MediaKind::Movie,
            "Alien",
            Some(1979),
            &[(IdKind::Tmdb, "348"), (IdKind::Imdb, "tt0078748")],
            0.0,
        )
        .expect("scrobbleable");
        let v: Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(v["movie"]["title"], "Alien");
        assert_eq!(v["movie"]["year"], 1979);
        assert_eq!(v["movie"]["ids"]["tmdb"], 348);
        assert_eq!(v["movie"]["ids"]["imdb"], "tt0078748");
        assert_eq!(v["progress"], 0.0);
    }

    #[test]
    fn episode_body_is_ids_only() {
        let body = scrobble_body(
            MediaKind::Episode,
            "Pilot",
            Some(2008),
            &[(IdKind::Tvdb, "349232")],
            100.0,
        )
        .expect("scrobbleable");
        let v: Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(v["episode"]["ids"]["tvdb"], 349232);
        assert!(v["episode"].get("title").is_none());
        assert_eq!(v["progress"], 100.0);
    }

    #[test]
    fn unparseable_numeric_ids_are_dropped() {
        let body = scrobble_body(
            MediaKind::Movie,
            "Alien",
            None,
            &[(IdKind::Tmdb, "not-a-number"), (IdKind::Imdb, "tt0078748")],
            0.0,
        )
        .expect("imdb survives");
        let v: Value = serde_json::from_str(&body).expect("valid JSON");
        assert!(v["movie"]["ids"].get("tmdb").is_none());
        assert_eq!(v["movie"]["ids"]["imdb"], "tt0078748");
    }

    #[test]
    fn no_usable_ids_or_kind_means_skip() {
        assert!(
            scrobble_body(
                MediaKind::Movie,
                "Alien",
                None,
                &[(IdKind::Other, "x")],
                0.0
            )
            .is_none()
        );
        assert!(
            scrobble_body(MediaKind::Other, "Album", None, &[(IdKind::Tmdb, "1")], 0.0).is_none()
        );
    }

    #[test]
    fn endpoints_match_trakt() {
        assert_eq!(Action::Started.endpoint(), "start");
        assert_eq!(Action::Paused.endpoint(), "pause");
        assert_eq!(Action::Stopped.endpoint(), "stop");
    }
}
