// SPDX-License-Identifier: AGPL-3.0-only
//
// OpenSubtitles.com REST API v1 request building and response parsing.
// Pure (strings and bytes in, plain structs out) so it is testable
// off-target; the guest maps these onto the WIT types.
//
// The API redirects (301) any search whose query string isn't in its
// canonical form, and the host never follows redirects, so every search
// URL is built canonical: parameters sorted by name, values lowercase, ids
// without `tt` or leading zeros, and no `page=1`.

use serde::Deserialize;

/// The API host everyone starts on. A VIP account's login names another.
pub const API_HOST: &str = "api.opensubtitles.com";
/// Hosts the plugin may reach: the API, the VIP API a login can hand out,
/// and where download links point.
pub const HOSTS: [&str; 3] = [
    API_HOST,
    "vip-api.opensubtitles.com",
    "www.opensubtitles.com",
];
/// OpenSubtitles asks every client to name itself and its version.
pub const USER_AGENT: &str = concat!("NoFlex v", env!("CARGO_PKG_VERSION"));
/// Every download asks for SubRip; the API converts what was uploaded.
pub const FORMAT: &str = "srt";

/// ISO 639-2/T, as NoFlex spells languages, beside OpenSubtitles' codes:
/// ISO 639-1, with regions where it tells them apart.
const LANGUAGES: &[(&str, &[&str])] = &[
    ("eng", &["en"]),
    ("fra", &["fr"]),
    ("deu", &["de"]),
    ("spa", &["es"]),
    ("ita", &["it"]),
    ("por", &["pt-br", "pt-pt"]),
    ("nld", &["nl"]),
    ("swe", &["sv"]),
    ("nor", &["no"]),
    ("nob", &["no"]),
    ("dan", &["da"]),
    ("fin", &["fi"]),
    ("isl", &["is"]),
    ("pol", &["pl"]),
    ("ces", &["cs"]),
    ("slk", &["sk"]),
    ("hun", &["hu"]),
    ("ron", &["ro"]),
    ("bul", &["bg"]),
    ("hrv", &["hr"]),
    ("srp", &["sr"]),
    ("slv", &["sl"]),
    ("ell", &["el"]),
    ("tur", &["tr"]),
    ("rus", &["ru"]),
    ("ukr", &["uk"]),
    ("ara", &["ar"]),
    ("heb", &["he"]),
    ("fas", &["fa"]),
    ("hin", &["hi"]),
    ("ben", &["bn"]),
    ("tam", &["ta"]),
    ("tel", &["te"]),
    ("tha", &["th"]),
    ("vie", &["vi"]),
    ("ind", &["id"]),
    ("msa", &["ms"]),
    ("zho", &["zh-cn", "zh-tw"]),
    ("jpn", &["ja"]),
    ("kor", &["ko"]),
];

/// The `languages` parameter for NoFlex's ISO 639-2/T codes: OpenSubtitles'
/// codes, sorted and comma-joined. A code with no known spelling passes
/// through lowercased. `None` for any language.
pub fn languages_param(languages: &[String]) -> Option<String> {
    let mut codes: Vec<String> = languages
        .iter()
        .flat_map(|language| {
            let language = language.to_ascii_lowercase();
            match LANGUAGES.iter().find(|(ours, _)| *ours == language) {
                Some((_, theirs)) => theirs.iter().map(|code| code.to_string()).collect(),
                None => vec![language],
            }
        })
        .collect();
    codes.sort();
    codes.dedup();
    (!codes.is_empty()).then(|| codes.join(","))
}

/// NoFlex's spelling of an OpenSubtitles language (`pt-BR`), and its
/// BCP 47 locale when it names a region.
pub fn language(code: &str) -> (String, Option<String>) {
    let lower = code.to_ascii_lowercase();
    let base = lower.split('-').next().unwrap_or(&lower);
    let ours = LANGUAGES
        .iter()
        .find(|(_, theirs)| theirs.iter().any(|t| *t == lower || *t == base))
        .map_or_else(|| base.to_string(), |(ours, _)| ours.to_string());
    let locale = lower
        .split_once('-')
        .map(|(language, region)| format!("{language}-{}", region.to_ascii_uppercase()));
    (ours, locale)
}

/// The kinds of id NoFlex keeps that OpenSubtitles searches by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Imdb,
    Tmdb,
    Other,
}

/// An IMDb id's number: `tt0078748` → `78748`.
fn imdb_number(value: &str) -> Option<u64> {
    value
        .trim()
        .trim_start_matches("tt")
        .parse()
        .ok()
        .filter(|n| *n > 0)
}

/// A TMDb id's number for a record of `kind`, from the bare number or the
/// TMDb plugin's path (`movie/603`, `tv/1399`). An episode's path names
/// its series, so it gives none.
fn tmdb_number(value: &str, kind: &str) -> Option<u64> {
    let value = value.trim();
    let number = match value.split_once('/') {
        Some((prefix, rest)) if prefix == kind && !rest.contains('/') => rest,
        Some(_) => return None,
        None => value,
    };
    number.parse().ok().filter(|n| *n > 0)
}

/// What the guest knows about the file, in OpenSubtitles' terms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Search<'a> {
    pub episode: bool,
    pub moviehash: Option<&'a str>,
    pub title: &'a str,
    pub year: Option<i32>,
    pub ids: Vec<(IdKind, &'a str)>,
    /// An episode's series ids.
    pub series_ids: Vec<(IdKind, &'a str)>,
    pub season: Option<u32>,
    pub number: Option<u32>,
    /// NoFlex's ISO 639-2/T codes.
    pub languages: &'a [String],
}

impl Search<'_> {
    /// Search URLs on `host`, surest first; the guest stops at the first
    /// that finds anything. By ids (with the hash, so a hash match is
    /// flagged) when there are any, by hash alone otherwise, and by title
    /// last.
    pub fn urls(&self, host: &str) -> Vec<String> {
        let mut base: Vec<(&str, String)> = vec![("ai_translated", "exclude".into())];
        if let Some(languages) = languages_param(self.languages) {
            base.push(("languages", languages));
        }
        if let Some(hash) = self.moviehash {
            base.push(("moviehash", hash.to_ascii_lowercase()));
        }
        let mut by_ids = base.clone();
        if self.episode {
            let parent_imdb = self.series_ids.iter().find_map(|(kind, value)| match kind {
                IdKind::Imdb => imdb_number(value),
                _ => None,
            });
            let parent_tmdb = self.series_ids.iter().find_map(|(kind, value)| match kind {
                IdKind::Tmdb => tmdb_number(value, "tv"),
                _ => None,
            });
            let own_imdb = self.ids.iter().find_map(|(kind, value)| match kind {
                IdKind::Imdb => imdb_number(value),
                _ => None,
            });
            match (parent_imdb, parent_tmdb, self.season, self.number, own_imdb) {
                (imdb, tmdb, Some(season), Some(number), _) if imdb.is_some() || tmdb.is_some() => {
                    if let Some(imdb) = imdb {
                        by_ids.push(("parent_imdb_id", imdb.to_string()));
                    } else if let Some(tmdb) = tmdb {
                        by_ids.push(("parent_tmdb_id", tmdb.to_string()));
                    }
                    by_ids.push(("season_number", season.to_string()));
                    by_ids.push(("episode_number", number.to_string()));
                }
                (.., Some(imdb)) => by_ids.push(("imdb_id", imdb.to_string())),
                _ => {}
            }
        } else {
            let imdb = self.ids.iter().find_map(|(kind, value)| match kind {
                IdKind::Imdb => imdb_number(value),
                _ => None,
            });
            let tmdb = self.ids.iter().find_map(|(kind, value)| match kind {
                IdKind::Tmdb => tmdb_number(value, "movie"),
                _ => None,
            });
            if let Some(imdb) = imdb {
                by_ids.push(("imdb_id", imdb.to_string()));
            } else if let Some(tmdb) = tmdb {
                by_ids.push(("tmdb_id", tmdb.to_string()));
            }
        }

        let mut urls = Vec::new();
        if by_ids.len() > base.len() || self.moviehash.is_some() {
            urls.push(subtitles_url(host, by_ids));
        }
        if !self.title.trim().is_empty() {
            let mut by_title = base;
            by_title.retain(|(key, _)| *key != "moviehash");
            by_title.push(("query", self.title.trim().to_lowercase()));
            if self.episode {
                by_title.push(("type", "episode".into()));
                if let (Some(season), Some(number)) = (self.season, self.number) {
                    by_title.push(("season_number", season.to_string()));
                    by_title.push(("episode_number", number.to_string()));
                }
            } else {
                by_title.push(("type", "movie".into()));
                if let Some(year) = self.year {
                    by_title.push(("year", year.to_string()));
                }
            }
            urls.push(subtitles_url(host, by_title));
        }
        urls
    }
}

/// `/subtitles` with `params` in the API's canonical order.
fn subtitles_url(host: &str, mut params: Vec<(&str, String)>) -> String {
    params.sort_by(|a, b| a.0.cmp(b.0));
    let query: Vec<String> = params
        .into_iter()
        .map(|(key, value)| format!("{key}={}", encode(&value)))
        .collect();
    format!("https://{host}/api/v1/subtitles?{}", query.join("&"))
}

/// Percent-encoding for a query value, spaces as `+` as the API's own
/// canonical URLs spell them. Commas stay: they separate languages.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b',' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// One subtitle a search found.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// What `/download` takes.
    pub file_id: u64,
    /// OpenSubtitles' code (`en`, `pt-BR`).
    pub language: String,
    pub release: Option<String>,
    pub hearing_impaired: bool,
    pub foreign_parts_only: bool,
    pub moviehash_match: bool,
    pub download_count: Option<u64>,
    /// Out of 10; `None` when nobody rated it.
    pub rating: Option<f32>,
}

#[derive(Deserialize)]
struct SearchPage {
    data: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    attributes: Attributes,
}

#[derive(Deserialize)]
struct Attributes {
    language: Option<String>,
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    hearing_impaired: bool,
    #[serde(default)]
    foreign_parts_only: bool,
    #[serde(default)]
    moviehash_match: bool,
    #[serde(default)]
    download_count: Option<u64>,
    #[serde(default)]
    ratings: Option<f32>,
    #[serde(default)]
    files: Vec<File>,
}

#[derive(Deserialize)]
struct File {
    file_id: u64,
}

/// The subtitles on a search page. One uploaded in several parts (`files`
/// past the first, for a film split over discs) is left out: it can't be
/// laid over a single file.
pub fn parse_search(body: &[u8]) -> Result<Vec<Found>, String> {
    let page: SearchPage =
        serde_json::from_slice(body).map_err(|e| format!("search response: {e}"))?;
    Ok(page
        .data
        .into_iter()
        .filter_map(|item| {
            let a = item.attributes;
            let [file] = a.files.as_slice() else {
                return None;
            };
            Some(Found {
                file_id: file.file_id,
                language: a.language?,
                release: a.release.filter(|r| !r.trim().is_empty()),
                hearing_impaired: a.hearing_impaired,
                foreign_parts_only: a.foreign_parts_only,
                moviehash_match: a.moviehash_match,
                download_count: a.download_count,
                rating: a.ratings.filter(|r| *r > 0.0),
            })
        })
        .collect())
}

/// Body for `POST /download`.
pub fn download_body(file_id: u64) -> String {
    serde_json::json!({ "file_id": file_id, "sub_format": FORMAT }).to_string()
}

/// Where a download is, and how many more the quota allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub remaining: Option<i64>,
}

#[derive(Deserialize)]
struct DownloadAnswer {
    link: Option<String>,
    remaining: Option<i64>,
}

/// The temporary link `POST /download` answers with. It must be `https` on
/// one of [`HOSTS`], as host-net would refuse anything else anyway.
pub fn parse_download(body: &[u8]) -> Result<Link, String> {
    let answer: DownloadAnswer =
        serde_json::from_slice(body).map_err(|e| format!("download response: {e}"))?;
    let url = answer.link.ok_or("download response has no link")?;
    let host = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .ok_or("download link isn't https")?;
    if !HOSTS.contains(&host) {
        return Err(format!("download link points at {host}"));
    }
    Ok(Link {
        url,
        remaining: answer.remaining,
    })
}

#[derive(Deserialize)]
struct QuotaAnswer {
    reset_time_utc: Option<String>,
}

/// Seconds from `now_unix` until the quota a 406 says is used up resets,
/// from its `reset_time_utc`. `None` when the body doesn't say.
pub fn quota_reset_in(body: &[u8], now_unix: u64) -> Option<u32> {
    let answer: QuotaAnswer = serde_json::from_slice(body).ok()?;
    let reset = unix_seconds(&answer.reset_time_utc?)?;
    u32::try_from(reset.saturating_sub(now_unix)).ok()
}

/// `2026-11-08T23:59:59.000Z` as seconds since the Unix epoch.
fn unix_seconds(timestamp: &str) -> Option<u64> {
    let (date, time) = timestamp.trim_end_matches('Z').split_once('T')?;
    let mut date = date.split('-').map(|p| p.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let time = time.split('.').next()?;
    let mut time = time.split(':').map(|p| p.parse::<i64>().ok());
    let (hour, minute, second) = (time.next()??, time.next()??, time.next()??);
    // Days from the civil date (Howard Hinnant's algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(days * 86_400 + hour * 3600 + minute * 60 + second).ok()
}

/// What `POST /login` answers: the token, and the API host to use with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub token: String,
    pub host: String,
}

#[derive(Deserialize)]
struct LoginAnswer {
    token: Option<String>,
    base_url: Option<String>,
}

/// Body for `POST /login`.
pub fn login_body(username: &str, password: &str) -> String {
    serde_json::json!({ "username": username, "password": password }).to_string()
}

/// The token a login answers with. A `base_url` that isn't one of
/// [`HOSTS`] is ignored for [`API_HOST`].
pub fn parse_login(body: &[u8]) -> Result<Session, String> {
    let answer: LoginAnswer =
        serde_json::from_slice(body).map_err(|e| format!("login response: {e}"))?;
    let token = answer
        .token
        .filter(|t| !t.is_empty())
        .ok_or("login response has no token")?;
    let host = answer
        .base_url
        .map(|url| {
            url.trim_start_matches("https://")
                .trim_end_matches('/')
                .to_string()
        })
        .filter(|host| HOSTS[..2].contains(&host.as_str()))
        .unwrap_or_else(|| API_HOST.to_string());
    Ok(Session { token, host })
}

/// The headers every API request carries; `token` once logged in.
pub fn headers(api_key: &str, token: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![
        ("api-key".to_string(), api_key.to_string()),
        ("user-agent".to_string(), USER_AGENT.to_string()),
        ("accept".to_string(), "application/json".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
    ];
    if let Some(token) = token {
        headers.push(("authorization".to_string(), format!("Bearer {token}")));
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_MOVIE: &[u8] = include_bytes!("../fixtures/search_movie.json");
    const SEARCH_EPISODE: &[u8] = include_bytes!("../fixtures/search_episode.json");
    const DOWNLOAD: &[u8] = include_bytes!("../fixtures/download.json");
    const QUOTA: &[u8] = include_bytes!("../fixtures/download_quota.json");
    const LOGIN: &[u8] = include_bytes!("../fixtures/login.json");

    fn langs(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn searches_are_built_canonical_surest_first() {
        let languages = langs(&["fra", "eng"]);
        let search = Search {
            moviehash: Some("8E245D9679D31E12"),
            title: "Alien",
            year: Some(1979),
            ids: vec![(IdKind::Tmdb, "movie/348"), (IdKind::Imdb, "tt0078748")],
            languages: &languages,
            ..Search::default()
        };
        assert_eq!(
            search.urls(API_HOST),
            [
                "https://api.opensubtitles.com/api/v1/subtitles?ai_translated=exclude&imdb_id=78748\
                 &languages=en,fr&moviehash=8e245d9679d31e12",
                "https://api.opensubtitles.com/api/v1/subtitles?ai_translated=exclude\
                 &languages=en,fr&query=alien&type=movie&year=1979",
            ]
        );
    }

    #[test]
    fn episodes_search_by_series_season_and_number() {
        let languages = langs(&["por"]);
        let search = Search {
            episode: true,
            title: "A Fractured House",
            ids: vec![(IdKind::Tmdb, "tv/1403/season/2/episode/6")],
            series_ids: vec![(IdKind::Tmdb, "tv/1403")],
            season: Some(2),
            number: Some(6),
            languages: &languages,
            ..Search::default()
        };
        let urls = search.urls("vip-api.opensubtitles.com");
        assert_eq!(
            urls[0],
            "https://vip-api.opensubtitles.com/api/v1/subtitles?ai_translated=exclude\
             &episode_number=6&languages=pt-br,pt-pt&parent_tmdb_id=1403&season_number=2"
        );
        assert_eq!(
            urls[1],
            "https://vip-api.opensubtitles.com/api/v1/subtitles?ai_translated=exclude\
             &episode_number=6&languages=pt-br,pt-pt&query=a+fractured+house&season_number=2\
             &type=episode"
        );
    }

    #[test]
    fn nothing_to_search_by_but_a_title() {
        let search = Search {
            title: "Home Movie: 2009 & After",
            ..Search::default()
        };
        assert_eq!(
            search.urls(API_HOST),
            [
                "https://api.opensubtitles.com/api/v1/subtitles?ai_translated=exclude\
              &query=home+movie%3A+2009+%26+after&type=movie"
            ]
        );
    }

    #[test]
    fn languages_map_both_ways() {
        assert_eq!(
            languages_param(&langs(&["zho", "eng", "xyz"])).as_deref(),
            Some("en,xyz,zh-cn,zh-tw")
        );
        assert_eq!(languages_param(&[]), None);
        assert_eq!(language("pt-BR"), ("por".into(), Some("pt-BR".into())));
        assert_eq!(language("en"), ("eng".into(), None));
        assert_eq!(language("zh-TW"), ("zho".into(), Some("zh-TW".into())));
        assert_eq!(language("tlh"), ("tlh".into(), None));
    }

    #[test]
    fn search_pages_parse_and_multi_part_uploads_are_left_out() {
        let found = parse_search(SEARCH_MOVIE).expect("parse");
        assert_eq!(found.len(), 2, "the two-disc upload is left out");
        assert_eq!(
            found[0],
            Found {
                file_id: 5_274_788,
                language: "en".into(),
                release: Some("Alien.1979.Directors.Cut.1080p.BluRay.x264".into()),
                hearing_impaired: false,
                foreign_parts_only: false,
                moviehash_match: true,
                download_count: Some(48_215),
                rating: Some(8.7),
            }
        );
        assert!(found[1].hearing_impaired);
        assert!(!found[1].moviehash_match);
        assert_eq!(found[1].rating, None, "unrated");

        let episode = parse_search(SEARCH_EPISODE).expect("parse");
        assert_eq!(episode.len(), 1);
        assert_eq!(episode[0].language, "pt-BR");
        assert!(episode[0].foreign_parts_only);
        assert!(parse_search(b"<html>").is_err());
    }

    #[test]
    fn downloads_answer_a_link_on_an_allowed_host() {
        let link = parse_download(DOWNLOAD).expect("link");
        assert!(
            link.url
                .starts_with("https://www.opensubtitles.com/download/")
        );
        assert_eq!(link.remaining, Some(97));
        assert!(parse_download(br#"{"link":"https://evil.example/x.srt","remaining":1}"#).is_err());
        assert!(parse_download(br#"{"link":"http://www.opensubtitles.com/x"}"#).is_err());
        assert_eq!(
            download_body(5_274_788),
            r#"{"file_id":5274788,"sub_format":"srt"}"#
        );
    }

    #[test]
    fn a_used_up_quota_says_when_it_resets() {
        // 2026-11-08T23:59:59Z is 1_794_182_399; an hour and a bit before it:
        assert_eq!(
            unix_seconds("2026-11-08T23:59:59.000Z"),
            Some(1_794_182_399)
        );
        assert_eq!(quota_reset_in(QUOTA, 1_794_177_337), Some(5062));
        assert_eq!(
            quota_reset_in(QUOTA, 1_894_177_337),
            Some(0),
            "already reset"
        );
        assert_eq!(quota_reset_in(b"{}", 0), None);
    }

    #[test]
    fn logins_answer_a_token_and_maybe_the_vip_host() {
        assert_eq!(
            parse_login(LOGIN).expect("login"),
            Session {
                token: "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.fixture.signature".into(),
                host: "vip-api.opensubtitles.com".into(),
            }
        );
        let elsewhere = parse_login(br#"{"token":"t","base_url":"evil.example"}"#).expect("login");
        assert_eq!(elsewhere.host, API_HOST, "only known hosts are used");
        assert!(parse_login(br#"{"status":401}"#).is_err());
    }

    #[test]
    fn requests_name_noflex_and_carry_the_key() {
        let h = headers("k3y", Some("tok"));
        assert!(h.contains(&("api-key".into(), "k3y".into())));
        assert!(h.contains(&("authorization".into(), "Bearer tok".into())));
        assert!(
            h.iter()
                .any(|(name, value)| name == "user-agent" && value.starts_with("NoFlex v"))
        );
    }
}
