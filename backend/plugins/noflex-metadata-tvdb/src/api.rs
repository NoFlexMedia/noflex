// SPDX-License-Identifier: AGPL-3.0-only
//
// TheTVDB v4 request building and response parsing. Pure (bytes in, plain
// structs out) so it is testable off-target against fixtures written from the
// published OpenAPI document; the guest module maps these onto the WIT types.
//
// External-id convention for `ExternalIdKind::Tvdb` values, mirroring how a
// work is reached rather than TVDB's own season and episode numbers, so a
// child's id follows from its series and numbers without a request:
//   movie    `movie/{id}`
//   series   `series/{id}`
//   season   `series/{id}/season/{n}`
//   episode  `series/{id}/season/{n}/episode/{m}`
// Seasons and episodes count in the series' default order.

use std::collections::HashMap;

use serde::Deserialize;

/// TVDB v4 API root.
pub const API_BASE: &str = "https://api4.thetvdb.com/v4";
/// The artwork CDN; older records give paths relative to it.
const ART_BASE: &str = "https://artworks.thetvdb.com";
/// Sent on every request so TVDB can attribute traffic.
pub const USER_AGENT: &str = "NoFlex/0.0.1 (https://github.com/noflex-media/noflex)";

/// Cast members kept per payload; TVDB lists whole ensembles.
const MAX_CAST: usize = 15;
/// Search results requested and turned into matches.
const MAX_MATCHES: usize = 10;

// TVDB's numbering of artwork types, from its `/artwork/types`. A season keeps
// its posters under another number than a series, and a movie's differ again.
const ART_SERIES_POSTER: u32 = 2;
const ART_SERIES_BACKGROUND: u32 = 3;
const ART_SEASON_POSTER: u32 = 7;
const ART_MOVIE_POSTER: u32 = 14;
const ART_MOVIE_BACKGROUND: u32 = 15;
const ART_SERIES_CLEARLOGO: u32 = 23;
const ART_MOVIE_CLEARLOGO: u32 = 25;
/// What an episode's own `image` field is, for the entry made from it.
const ART_EPISODE_IMAGE: u32 = 11;

/// One TVDB record, addressed by the path convention in this module's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TvdbId {
    Movie(u64),
    Series(u64),
    Season {
        series: u64,
        season: u32,
    },
    Episode {
        series: u64,
        season: u32,
        episode: u32,
    },
}

impl TvdbId {
    /// Parses the path convention above; anything else is an error.
    pub fn parse(value: &str) -> Result<Self, String> {
        let bad = || format!("not a TVDB id: {value:?}");
        let parts: Vec<&str> = value.trim().split('/').collect();
        let num = |s: &str| s.parse::<u64>().map_err(|_| bad());
        let small = |s: &str| s.parse::<u32>().map_err(|_| bad());
        match parts.as_slice() {
            ["movie", id] => Ok(Self::Movie(num(id)?)),
            ["series", id] => Ok(Self::Series(num(id)?)),
            ["series", id, "season", n] => Ok(Self::Season {
                series: num(id)?,
                season: small(n)?,
            }),
            ["series", id, "season", n, "episode", m] => Ok(Self::Episode {
                series: num(id)?,
                season: small(n)?,
                episode: small(m)?,
            }),
            _ => Err(bad()),
        }
    }

    /// The external-id value.
    pub fn path(&self) -> String {
        match self {
            Self::Movie(id) => format!("movie/{id}"),
            Self::Series(id) => format!("series/{id}"),
            Self::Season { series, season } => format!("series/{series}/season/{season}"),
            Self::Episode {
                series,
                season,
                episode,
            } => format!("series/{series}/season/{season}/episode/{episode}"),
        }
    }
}

/// The two record types TVDB searches by title here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Movie,
    Series,
}

impl SearchKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }
}

/// External-id kinds a payload can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Tvdb,
    Imdb,
    Tmdb,
}

/// The image kinds this plugin serves, named after the WIT `image-kind` they map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtKind {
    Primary,
    Backdrop,
    Logo,
    Thumb,
}

/// One image a payload advertises.
#[derive(Debug, Clone, PartialEq)]
pub struct Art {
    pub kind: ArtKind,
    pub url: String,
    pub language: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A credited person; `role` is `Director`, `Writer`, `Producer` or `Actor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub role: Option<String>,
    pub character: Option<String>,
    pub photo_url: Option<String>,
}

/// A parsed record in WIT-payload shape; empty fields mean TVDB had none. TVDB's
/// `score` hints at popularity rather than rating a title, so no rating comes
/// from here, and nothing fills keywords.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Payload {
    pub ids: Vec<(IdKind, String)>,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub runtime_ms: Option<i64>,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    pub people: Vec<Person>,
    pub images: Vec<Art>,
    pub rating: Option<String>,
    pub original_language: Option<String>,
    pub origin_countries: Vec<String>,
    pub budget_usd: Option<u64>,
}

/// One search result with this plugin's confidence (see [`confidence`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub id: TvdbId,
    pub title: String,
    pub year: Option<i32>,
    pub confidence: f32,
}

/// A season of a series, in the series' default order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonRef {
    pub id: u64,
    pub number: u32,
}

/// One downloadable image chosen for an `image::fetch` request.
#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    pub url: String,
    pub language: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// The `/login` body. TVDB wants `pin` left out unless the key is user-supported.
pub fn login_body(key: &str, pin: Option<&str>) -> Vec<u8> {
    let mut body = serde_json::Map::new();
    body.insert("apikey".into(), key.into());
    if let Some(pin) = pin {
        body.insert("pin".into(), pin.into());
    }
    serde_json::Value::Object(body).to_string().into_bytes()
}

/// Title search, narrowed to `year` when given.
pub fn search_url(kind: SearchKind, title: &str, year: Option<i32>) -> String {
    let mut url = format!(
        "{API_BASE}/search?query={}&type={}&limit={MAX_MATCHES}",
        percent_encode(title.trim()),
        kind.as_str(),
    );
    if let Some(year) = year {
        url.push_str(&format!("&year={year}"));
    }
    url
}

/// Resolves an IMDb or other remote id to TVDB records.
pub fn remote_id_url(id: &str) -> String {
    format!("{API_BASE}/search/remoteid/{}", percent_encode(id.trim()))
}

pub fn series_url(id: u64) -> String {
    format!("{API_BASE}/series/{id}/extended?meta=translations")
}

pub fn movie_url(id: u64) -> String {
    format!("{API_BASE}/movies/{id}/extended?meta=translations")
}

/// `id` is TVDB's own number for the season, from [`parse_seasons`].
pub fn season_url(id: u64) -> String {
    format!("{API_BASE}/seasons/{id}/extended")
}

/// Finds an episode's own number from its place in the series' default order.
pub fn episode_lookup_url(series: u64, season: u32, episode: u32) -> String {
    format!(
        "{API_BASE}/series/{series}/episodes/default?page=0&season={season}&episodeNumber={episode}"
    )
}

/// `id` is TVDB's own number for the episode, from [`parse_episode_lookup`].
pub fn episode_url(id: u64) -> String {
    format!("{API_BASE}/episodes/{id}/extended?meta=translations")
}

/// The number TVDB files a TMDb id under: `tv/1399` and `movie/603` are the
/// TMDb plugin's values, and TMDb's numbers alone are what TVDB stored.
/// `None` for the season and episode forms.
pub fn tmdb_record_number(value: &str) -> Option<&str> {
    match value.trim().split('/').collect::<Vec<_>>().as_slice() {
        ["tv" | "movie", id] if id.bytes().all(|b| b.is_ascii_digit()) => Some(id),
        _ => None,
    }
}

/// Content type for a downloaded image: the response header when it names an
/// image type, else a guess from the file extension.
pub fn image_mime(content_type: Option<&str>, file_path: &str) -> String {
    if let Some(ct) = content_type {
        let ct = ct.split(';').next().unwrap_or("").trim();
        if ct.starts_with("image/") {
            return ct.to_ascii_lowercase();
        }
    }
    let ext = file_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "image/jpeg",
    }
    .to_string()
}

fn percent_encode(input: &str) -> String {
    let mut output = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

/// ISO 639-2 codes as TVDB spells them, and the ISO 639-1 codes the host uses.
const LANGUAGES: &[(&str, &str)] = &[
    ("ara", "ar"),
    ("ces", "cs"),
    ("dan", "da"),
    ("deu", "de"),
    ("ell", "el"),
    ("eng", "en"),
    ("fin", "fi"),
    ("fra", "fr"),
    ("heb", "he"),
    ("hin", "hi"),
    ("hun", "hu"),
    ("ind", "id"),
    ("ita", "it"),
    ("jpn", "ja"),
    ("kor", "ko"),
    ("nld", "nl"),
    ("nor", "no"),
    ("pol", "pl"),
    ("por", "pt"),
    ("ron", "ro"),
    ("rus", "ru"),
    ("spa", "es"),
    ("swe", "sv"),
    ("tha", "th"),
    ("tur", "tr"),
    ("ukr", "uk"),
    ("vie", "vi"),
    ("zho", "zh"),
];

/// ISO 3166-1 alpha-3 codes as TVDB spells them, and the alpha-2 codes the host uses.
const COUNTRIES: &[(&str, &str)] = &[
    ("arg", "AR"),
    ("aus", "AU"),
    ("aut", "AT"),
    ("bel", "BE"),
    ("bra", "BR"),
    ("can", "CA"),
    ("che", "CH"),
    ("chn", "CN"),
    ("deu", "DE"),
    ("dnk", "DK"),
    ("esp", "ES"),
    ("fin", "FI"),
    ("fra", "FR"),
    ("gbr", "GB"),
    ("hkg", "HK"),
    ("ind", "IN"),
    ("irl", "IE"),
    ("ita", "IT"),
    ("jpn", "JP"),
    ("kor", "KR"),
    ("mex", "MX"),
    ("nld", "NL"),
    ("nor", "NO"),
    ("nzl", "NZ"),
    ("pol", "PL"),
    ("rus", "RU"),
    ("swe", "SE"),
    ("tur", "TR"),
    ("twn", "TW"),
    ("usa", "US"),
];

fn two_letter_language(tvdb: &str) -> Option<String> {
    LANGUAGES
        .iter()
        .find(|(three, _)| three.eq_ignore_ascii_case(tvdb))
        .map(|(_, two)| (*two).to_string())
}

/// TVDB's spelling of the host's BCP-47 `tag` (`en-US` -> `eng`).
fn tvdb_language(tag: &str) -> Option<&'static str> {
    let primary = tag.split(['-', '_']).next()?.trim();
    LANGUAGES
        .iter()
        .find(|(_, two)| two.eq_ignore_ascii_case(primary))
        .map(|(three, _)| *three)
}

fn two_letter_country(tvdb: &str) -> Option<String> {
    COUNTRIES
        .iter()
        .find(|(three, _)| three.eq_ignore_ascii_case(tvdb))
        .map(|(_, two)| (*two).to_string())
}

// ---- response shapes ------------------------------------------------------

/// TVDB answers `null` rather than an empty value for much of a record, which
/// `#[serde(default)]` alone refuses.
fn nullable<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct LoginData {
    token: String,
}

#[derive(Debug, Default, Deserialize)]
struct Named {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct Translation {
    language: Option<String>,
    name: Option<String>,
    overview: Option<String>,
    tagline: Option<String>,
    #[serde(default, rename = "isAlias", deserialize_with = "nullable")]
    is_alias: bool,
}

#[derive(Debug, Default, Deserialize)]
struct Translations {
    #[serde(default, rename = "nameTranslations", deserialize_with = "nullable")]
    names: Vec<Translation>,
    #[serde(
        default,
        rename = "overviewTranslations",
        deserialize_with = "nullable"
    )]
    overviews: Vec<Translation>,
}

#[derive(Debug, Default, Deserialize)]
struct RemoteId {
    #[serde(default, deserialize_with = "nullable")]
    id: String,
    #[serde(default, rename = "sourceName", deserialize_with = "nullable")]
    source: String,
}

#[derive(Debug, Default, Deserialize)]
struct CharacterRecord {
    #[serde(default, rename = "personName", deserialize_with = "nullable")]
    person: String,
    name: Option<String>,
    #[serde(default, rename = "peopleType", deserialize_with = "nullable")]
    people_type: String,
    #[serde(rename = "personImgURL")]
    photo: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    sort: i64,
}

#[derive(Debug, Default, Deserialize)]
struct ContentRating {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    country: String,
}

/// One entry of a record's artwork listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtEntry {
    #[serde(default, deserialize_with = "nullable")]
    image: String,
    #[serde(default, rename = "type", deserialize_with = "nullable")]
    kind: u32,
    language: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    #[serde(default, deserialize_with = "nullable")]
    score: f32,
    /// Made from the record's own `image`, whose language isn't known.
    #[serde(skip)]
    last_resort: bool,
}

#[derive(Debug, Default, Deserialize)]
struct SeasonType {
    id: Option<u64>,
    #[serde(rename = "type")]
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SeasonBase {
    id: u64,
    number: Option<u32>,
    name: Option<String>,
    image: Option<String>,
    year: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "nullable")]
    season_type: SeasonType,
}

#[derive(Debug, Default, Deserialize)]
struct CompanyRecord {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(rename = "primaryCompanyType")]
    company_type: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SeriesRecord {
    id: u64,
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    overview: Option<String>,
    #[serde(rename = "firstAired")]
    first_aired: Option<String>,
    year: Option<String>,
    #[serde(rename = "averageRuntime")]
    average_runtime: Option<i64>,
    #[serde(rename = "originalCountry")]
    country: Option<String>,
    #[serde(rename = "originalLanguage")]
    language: Option<String>,
    #[serde(rename = "originalNetwork")]
    network: Option<Named>,
    #[serde(default, deserialize_with = "nullable")]
    companies: Vec<CompanyRecord>,
    #[serde(default, deserialize_with = "nullable")]
    genres: Vec<Named>,
    #[serde(default, deserialize_with = "nullable")]
    characters: Vec<CharacterRecord>,
    #[serde(default, rename = "contentRatings", deserialize_with = "nullable")]
    ratings: Vec<ContentRating>,
    #[serde(default, rename = "remoteIds", deserialize_with = "nullable")]
    remote_ids: Vec<RemoteId>,
    #[serde(default, deserialize_with = "nullable")]
    artworks: Vec<ArtEntry>,
    image: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    translations: Translations,
    #[serde(default, deserialize_with = "nullable")]
    seasons: Vec<SeasonBase>,
    #[serde(rename = "defaultSeasonType")]
    default_season_type: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct MovieCompanies {
    #[serde(default, deserialize_with = "nullable")]
    studio: Vec<Named>,
    #[serde(default, deserialize_with = "nullable")]
    production: Vec<Named>,
}

#[derive(Debug, Default, Deserialize)]
struct Release {
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MovieRecord {
    id: u64,
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    overview: Option<String>,
    year: Option<String>,
    runtime: Option<i64>,
    budget: Option<String>,
    first_release: Option<Release>,
    #[serde(rename = "originalCountry")]
    country: Option<String>,
    #[serde(rename = "originalLanguage")]
    language: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    studios: Vec<Named>,
    #[serde(default, deserialize_with = "nullable")]
    companies: MovieCompanies,
    #[serde(default, deserialize_with = "nullable")]
    genres: Vec<Named>,
    #[serde(default, deserialize_with = "nullable")]
    characters: Vec<CharacterRecord>,
    #[serde(default, rename = "contentRatings", deserialize_with = "nullable")]
    ratings: Vec<ContentRating>,
    #[serde(default, rename = "remoteIds", deserialize_with = "nullable")]
    remote_ids: Vec<RemoteId>,
    #[serde(default, deserialize_with = "nullable")]
    artworks: Vec<ArtEntry>,
    image: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    translations: Translations,
}

#[derive(Debug, Default, Deserialize)]
struct EpisodeBase {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    overview: Option<String>,
    aired: Option<String>,
    runtime: Option<i64>,
    image: Option<String>,
    number: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SeasonRecord {
    #[serde(default)]
    name: Option<String>,
    image: Option<String>,
    year: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    artwork: Vec<ArtEntry>,
    #[serde(default, deserialize_with = "nullable")]
    episodes: Vec<EpisodeBase>,
    #[serde(default, deserialize_with = "nullable")]
    translations: Vec<Translation>,
}

#[derive(Debug, Deserialize)]
struct EpisodeRecord {
    #[serde(default)]
    name: Option<String>,
    overview: Option<String>,
    aired: Option<String>,
    runtime: Option<i64>,
    image: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    characters: Vec<CharacterRecord>,
    #[serde(default, rename = "remoteIds", deserialize_with = "nullable")]
    remote_ids: Vec<RemoteId>,
    #[serde(default, deserialize_with = "nullable")]
    translations: Translations,
}

#[derive(Debug, Deserialize)]
struct EpisodeListing {
    #[serde(default, deserialize_with = "nullable")]
    episodes: Vec<EpisodeBase>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    tvdb_id: Option<String>,
    name: Option<String>,
    year: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    aliases: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    translations: HashMap<String, String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RemoteHit {
    id: u64,
}

#[derive(Debug, Default, Deserialize)]
struct RemoteResult {
    series: Option<RemoteHit>,
    movie: Option<RemoteHit>,
}

// ---- parsing --------------------------------------------------------------

fn json<'a, T: Deserialize<'a>>(body: &'a [u8], what: &str) -> Result<T, String> {
    serde_json::from_slice(body).map_err(|e| format!("tvdb: malformed {what} response: {e}"))
}

fn data<'a, T: Deserialize<'a>>(body: &'a [u8], what: &str) -> Result<T, String> {
    json::<Envelope<T>>(body, what).map(|envelope| envelope.data)
}

/// The bearer token from a `/login` response.
///
/// # Errors
/// The body is not a login response.
pub fn parse_login(body: &[u8]) -> Result<String, String> {
    data::<LoginData>(body, "login").map(|login| login.token)
}

/// Matches from a search response, best first, scored against `query`/`year`.
///
/// # Errors
/// The body is not a search response.
pub fn parse_search(
    kind: SearchKind,
    body: &[u8],
    query: &str,
    year: Option<i32>,
) -> Result<Vec<Match>, String> {
    let results: Vec<SearchResult> = data(body, "search")?;
    let mut out: Vec<Match> = results
        .into_iter()
        .filter(|r| r.kind.as_deref().is_none_or(|k| k == kind.as_str()))
        .take(MAX_MATCHES)
        .enumerate()
        .filter_map(|(rank, r)| {
            let id = r.tvdb_id.as_deref()?.parse().ok()?;
            let english = r.translations.get("eng").and_then(|t| non_empty_str(t));
            let title = english
                .or(r.name.as_deref().and_then(non_empty_str))?
                .to_string();
            let result_year = r.year.as_deref().and_then(year_from_date);
            let mut names: Vec<&str> = r.aliases.iter().map(String::as_str).collect();
            names.extend(r.name.as_deref());
            let confidence = confidence(query, year, &title, &names, result_year, rank);
            Some(Match {
                id: match kind {
                    SearchKind::Movie => TvdbId::Movie(id),
                    SearchKind::Series => TvdbId::Series(id),
                },
                title,
                year: result_year,
                confidence,
            })
        })
        .collect();
    out.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    Ok(out)
}

/// The series or movie a remote id resolved to, when it names one.
///
/// # Errors
/// The body is not a remote-id response.
pub fn parse_remote_id(body: &[u8]) -> Result<Option<TvdbId>, String> {
    let results: Vec<RemoteResult> = data(body, "remote id")?;
    Ok(results.into_iter().find_map(|r| {
        r.series
            .map(|s| TvdbId::Series(s.id))
            .or_else(|| r.movie.map(|m| TvdbId::Movie(m.id)))
    }))
}

/// The series' seasons in its default order. TVDB lists a season once per
/// order (aired, DVD, absolute), which share numbers.
///
/// # Errors
/// The body is not a series record.
pub fn parse_seasons(body: &[u8]) -> Result<Vec<SeasonRef>, String> {
    let series: SeriesRecord = data(body, "series")?;
    Ok(default_order(&series.seasons, series.default_season_type)
        .filter_map(|s| {
            Some(SeasonRef {
                id: s.id,
                number: s.number?,
            })
        })
        .collect())
}

/// The seasons in `default` order, or in aired order when no order is named.
fn default_order(
    seasons: &[SeasonBase],
    default: Option<u64>,
) -> impl Iterator<Item = &SeasonBase> {
    seasons.iter().filter(move |s| match default {
        Some(default) => s.season_type.id == Some(default),
        None => s.season_type.name.as_deref() == Some("official"),
    })
}

/// The episode's own number from an [`episode_lookup_url`] response.
///
/// # Errors
/// The body is not an episode listing.
pub fn parse_episode_lookup(body: &[u8]) -> Result<Option<u64>, String> {
    let listing: EpisodeListing = data(body, "episode listing")?;
    Ok(listing.episodes.first().map(|e| e.id))
}

/// The payload and full artwork listing of a [`movie_url`] response.
///
/// # Errors
/// The body is not a movie record.
pub fn parse_movie(body: &[u8]) -> Result<(Payload, Vec<ArtEntry>), String> {
    let m: MovieRecord = data(body, "movie")?;
    let mut art = m.artworks;
    push_record_image(&mut art, m.image, ART_MOVIE_POSTER);
    let id = TvdbId::Movie(m.id);
    let (title, original_title) = titles(&m.translations, m.name);
    let release_date = m.first_release.and_then(|r| non_empty(r.date));
    let mut studios = names(m.studios);
    for company in names(m.companies.production)
        .into_iter()
        .chain(names(m.companies.studio))
    {
        if !studios.contains(&company) {
            studios.push(company);
        }
    }
    let payload = Payload {
        ids: ids(id, "movie", &m.remote_ids),
        year: m
            .year
            .as_deref()
            .and_then(year_from_date)
            .or_else(|| release_date.as_deref().and_then(year_from_date)),
        release_date,
        title,
        original_title,
        overview: overview(&m.translations, m.overview),
        tagline: tagline(&m.translations),
        runtime_ms: runtime_ms(m.runtime),
        genres: names(m.genres),
        studios,
        people: people(m.characters),
        images: payload_art(&art, id),
        rating: pick_rating(m.ratings),
        original_language: m.language.as_deref().and_then(two_letter_language),
        origin_countries: m
            .country
            .as_deref()
            .and_then(two_letter_country)
            .into_iter()
            .collect(),
        // TVDB gives budgets as free text; only a plain number is one to trust.
        budget_usd: m
            .budget
            .and_then(|b| b.trim().parse().ok())
            .filter(|b| *b > 0),
    };
    Ok((payload, art))
}

/// The payload and full artwork listing of a [`series_url`] response.
///
/// # Errors
/// The body is not a series record.
pub fn parse_series(body: &[u8]) -> Result<(Payload, Vec<ArtEntry>), String> {
    let s: SeriesRecord = data(body, "series")?;
    let mut art = s.artworks;
    push_record_image(&mut art, s.image, ART_SERIES_POSTER);
    let id = TvdbId::Series(s.id);
    let (title, original_title) = titles(&s.translations, s.name);
    let mut studios: Vec<String> = s.network.map(|n| n.name).into_iter().collect();
    // 1 is a network, 3 a production company; TVDB also lists distributors.
    let companies = s
        .companies
        .into_iter()
        .filter(|c| matches!(c.company_type, Some(1 | 3)))
        .map(|c| c.name);
    for company in companies {
        if !company.trim().is_empty() && !studios.contains(&company) {
            studios.push(company);
        }
    }
    let payload = Payload {
        ids: ids(id, "tv", &s.remote_ids),
        year: s
            .first_aired
            .as_deref()
            .or(s.year.as_deref())
            .and_then(year_from_date),
        release_date: non_empty(s.first_aired),
        title,
        original_title,
        overview: overview(&s.translations, s.overview),
        tagline: tagline(&s.translations),
        runtime_ms: runtime_ms(s.average_runtime),
        genres: names(s.genres),
        studios,
        people: people(s.characters),
        images: payload_art(&art, id),
        rating: pick_rating(s.ratings),
        original_language: s.language.as_deref().and_then(two_letter_language),
        origin_countries: s
            .country
            .as_deref()
            .and_then(two_letter_country)
            .into_iter()
            .collect(),
        budget_usd: None,
    };
    Ok((payload, art))
}

/// The payload and artwork listing of a [`season_url`] response, for the season
/// `season` of `series`. A season with no English name leaves the title empty,
/// so the scanner's "Season 1" stands rather than a name in another language.
///
/// # Errors
/// The body is not a season record.
pub fn parse_season(
    series: u64,
    season: u32,
    body: &[u8],
) -> Result<(Payload, Vec<ArtEntry>), String> {
    let s: SeasonRecord = data(body, "season")?;
    let mut art = s.artwork;
    push_record_image(&mut art, s.image, ART_SEASON_POSTER);
    let id = TvdbId::Season { series, season };
    let english = s
        .translations
        .iter()
        .find(|t| t.language.as_deref() == Some("eng"));
    let aired = s.episodes.iter().find_map(|e| e.aired.clone());
    let payload = Payload {
        ids: vec![(IdKind::Tvdb, id.path())],
        title: english
            .and_then(|t| non_empty(t.name.clone()))
            .or(non_empty(s.name))
            .unwrap_or_default(),
        overview: english.and_then(|t| non_empty(t.overview.clone())),
        year: s
            .year
            .as_deref()
            .or(aired.as_deref())
            .and_then(year_from_date),
        release_date: non_empty(aired),
        images: payload_art(&art, id),
        ..Payload::default()
    };
    Ok((payload, art))
}

/// The payload and artwork listing of an [`episode_url`] response, for the given
/// place in `series`.
///
/// # Errors
/// The body is not an episode record.
pub fn parse_episode(
    series: u64,
    season: u32,
    episode: u32,
    body: &[u8],
) -> Result<(Payload, Vec<ArtEntry>), String> {
    let e: EpisodeRecord = data(body, "episode")?;
    let mut art = Vec::new();
    push_record_image(&mut art, e.image, ART_EPISODE_IMAGE);
    let id = TvdbId::Episode {
        series,
        season,
        episode,
    };
    let mut ids = vec![(IdKind::Tvdb, id.path())];
    if let Some(imdb) = remote_id(&e.remote_ids, "imdb") {
        ids.push((IdKind::Imdb, imdb));
    }
    let (title, _) = titles(&e.translations, e.name.unwrap_or_default());
    let payload = Payload {
        ids,
        year: e.aired.as_deref().and_then(year_from_date),
        release_date: non_empty(e.aired),
        title,
        overview: overview(&e.translations, e.overview),
        runtime_ms: runtime_ms(e.runtime),
        people: people(e.characters),
        images: payload_art(&art, id),
        ..Payload::default()
    };
    Ok((payload, art))
}

/// Payloads for a series' seasons, from its [`series_url`] response.
///
/// # Errors
/// The body is not a series record.
pub fn parse_series_children(series: u64, body: &[u8]) -> Result<Vec<Payload>, String> {
    let s: SeriesRecord = data(body, "series")?;
    Ok(default_order(&s.seasons, s.default_season_type)
        .filter_map(|season| {
            let number = season.number?;
            let id = TvdbId::Season {
                series,
                season: number,
            };
            Some(Payload {
                ids: vec![(IdKind::Tvdb, id.path())],
                title: non_empty(season.name.clone()).unwrap_or_default(),
                year: season.year.as_deref().and_then(year_from_date),
                images: non_empty(season.image.clone())
                    .map(|image| {
                        vec![Art {
                            kind: ArtKind::Primary,
                            url: art_url(&image),
                            language: None,
                            width: None,
                            height: None,
                        }]
                    })
                    .unwrap_or_default(),
                ..Payload::default()
            })
        })
        .collect())
}

/// Payloads for a season's episodes, from its [`season_url`] response.
///
/// # Errors
/// The body is not a season record.
pub fn parse_season_children(
    series: u64,
    season: u32,
    body: &[u8],
) -> Result<Vec<Payload>, String> {
    let s: SeasonRecord = data(body, "season")?;
    Ok(s.episodes
        .into_iter()
        .filter_map(|e| {
            let id = TvdbId::Episode {
                series,
                season,
                episode: e.number?,
            };
            Some(Payload {
                ids: vec![(IdKind::Tvdb, id.path())],
                title: non_empty(e.name).unwrap_or_default(),
                overview: non_empty(e.overview),
                year: e.aired.as_deref().and_then(year_from_date),
                release_date: non_empty(e.aired),
                runtime_ms: runtime_ms(e.runtime),
                images: non_empty(e.image)
                    .map(|image| {
                        vec![Art {
                            kind: ArtKind::Thumb,
                            url: art_url(&image),
                            language: None,
                            width: None,
                            height: None,
                        }]
                    })
                    .unwrap_or_default(),
                ..Payload::default()
            })
        })
        .collect())
}

/// The record's id, then its IMDb and TMDb ids. `tmdb_path` starts the TMDb
/// plugin's value for this record type.
fn ids(id: TvdbId, tmdb_path: &str, remote: &[RemoteId]) -> Vec<(IdKind, String)> {
    let mut out = vec![(IdKind::Tvdb, id.path())];
    if let Some(imdb) = remote_id(remote, "imdb") {
        out.push((IdKind::Imdb, imdb));
    }
    if let Some(tmdb) = remote_id(remote, "themoviedb") {
        out.push((IdKind::Tmdb, format!("{tmdb_path}/{tmdb}")));
    }
    out
}

/// The id filed under a source whose name holds `source`, lowercase. TVDB
/// spells them `IMDB` and `TheMovieDB.com`.
fn remote_id(remote: &[RemoteId], source: &str) -> Option<String> {
    remote
        .iter()
        .find(|r| r.source.to_ascii_lowercase().contains(source))
        .map(|r| r.id.trim().to_string())
        .filter(|id| !id.is_empty())
}

/// The title to show and the original when they differ: TVDB's `name` is in the
/// record's own language, so an English translation takes its place.
fn titles(translations: &Translations, name: String) -> (String, Option<String>) {
    let english = translations
        .names
        .iter()
        .find(|t| t.language.as_deref() == Some("eng") && !t.is_alias)
        .and_then(|t| non_empty(t.name.clone()));
    match english {
        Some(english) => {
            let original = non_empty(Some(name)).filter(|n| *n != english);
            (english, original)
        }
        None => (name, None),
    }
}

fn overview(translations: &Translations, record: Option<String>) -> Option<String> {
    translations
        .overviews
        .iter()
        .find(|t| t.language.as_deref() == Some("eng"))
        .and_then(|t| non_empty(t.overview.clone()))
        .or_else(|| non_empty(record))
}

fn tagline(translations: &Translations) -> Option<String> {
    translations
        .overviews
        .iter()
        .chain(&translations.names)
        .find(|t| t.language.as_deref() == Some("eng") && t.tagline.is_some())
        .and_then(|t| non_empty(t.tagline.clone()))
}

fn runtime_ms(minutes: Option<i64>) -> Option<i64> {
    minutes.filter(|m| *m > 0).map(|m| m * 60_000)
}

/// Lists the record's own `image` as a last resort, for records with no
/// artwork listing or none of that type.
fn push_record_image(art: &mut Vec<ArtEntry>, image: Option<String>, kind: u32) {
    if let Some(image) = non_empty(image) {
        art.push(ArtEntry {
            image,
            kind,
            last_resort: true,
            ..ArtEntry::default()
        });
    }
}

fn kinds_for(id: TvdbId) -> &'static [ArtKind] {
    match id {
        TvdbId::Movie(_) | TvdbId::Series(_) => {
            &[ArtKind::Primary, ArtKind::Backdrop, ArtKind::Logo]
        }
        TvdbId::Season { .. } => &[ArtKind::Primary],
        TvdbId::Episode { .. } => &[ArtKind::Thumb],
    }
}

fn art_type(id: TvdbId, kind: ArtKind) -> Option<u32> {
    Some(match (id, kind) {
        (TvdbId::Movie(_), ArtKind::Primary) => ART_MOVIE_POSTER,
        (TvdbId::Movie(_), ArtKind::Backdrop) => ART_MOVIE_BACKGROUND,
        (TvdbId::Movie(_), ArtKind::Logo) => ART_MOVIE_CLEARLOGO,
        (TvdbId::Series(_), ArtKind::Primary) => ART_SERIES_POSTER,
        (TvdbId::Series(_), ArtKind::Backdrop) => ART_SERIES_BACKGROUND,
        (TvdbId::Series(_), ArtKind::Logo) => ART_SERIES_CLEARLOGO,
        (TvdbId::Season { .. }, ArtKind::Primary) => ART_SEASON_POSTER,
        (TvdbId::Episode { .. }, ArtKind::Thumb) => ART_EPISODE_IMAGE,
        _ => return None,
    })
}

/// Picks the best `kind` image of `id` for the BCP-47 tag `language`: an image in
/// that language first, then text-free art, then anything; ties go to the higher
/// score, then the wider image. Backdrops rank text-free art first, since text on
/// them fights the UI laid over them. `None` when `id`'s record type has no such
/// kind or the listing has no usable entry.
pub fn choose(
    art: &[ArtEntry],
    id: TvdbId,
    kind: ArtKind,
    language: Option<&str>,
) -> Option<Chosen> {
    let wanted = art_type(id, kind)?;
    let language = language.and_then(tvdb_language);
    let textless_first = kind == ArtKind::Backdrop;
    let tier = |e: &ArtEntry| -> u8 {
        if e.last_resort {
            return 3;
        }
        let lang = e.language.as_deref().filter(|l| !l.is_empty());
        let matches = matches!((lang, language), (Some(l), Some(w)) if l.eq_ignore_ascii_case(w));
        match (lang.is_none(), matches, textless_first) {
            (true, _, true) | (false, true, false) => 0,
            (true, _, false) | (false, true, true) => 1,
            _ => 2,
        }
    };
    art.iter()
        .filter(|e| e.kind == wanted && !e.image.is_empty())
        .min_by(|a, b| {
            tier(a)
                .cmp(&tier(b))
                .then(b.score.total_cmp(&a.score))
                .then(b.width.unwrap_or(0).cmp(&a.width.unwrap_or(0)))
        })
        .map(|e| Chosen {
            url: art_url(&e.image),
            language: e.language.as_deref().and_then(two_letter_language),
            width: e.width,
            height: e.height,
        })
}

/// One entry per kind with art, chosen for English; the host re-requests through
/// the `image` hook in its own preferred language.
fn payload_art(art: &[ArtEntry], id: TvdbId) -> Vec<Art> {
    kinds_for(id)
        .iter()
        .filter_map(|kind| {
            choose(art, id, *kind, Some("en")).map(|c| Art {
                kind: *kind,
                url: c.url,
                language: c.language,
                width: c.width,
                height: c.height,
            })
        })
        .collect()
}

fn art_url(image: &str) -> String {
    if image.starts_with("http://") || image.starts_with("https://") {
        image.to_string()
    } else {
        format!("{ART_BASE}/{}", image.trim_start_matches('/'))
    }
}

fn people(characters: Vec<CharacterRecord>) -> Vec<Person> {
    let mut sorted = characters;
    sorted.sort_by_key(|c| c.sort);
    let mut out: Vec<Person> = Vec::new();
    let mut cast = 0;
    for c in sorted {
        if c.person.trim().is_empty() {
            continue;
        }
        let actor = matches!(c.people_type.as_str(), "Actor" | "Guest Star");
        let role = match c.people_type.as_str() {
            "Actor" | "Guest Star" => "Actor",
            "Director" => "Director",
            "Writer" => "Writer",
            "Producer" => "Producer",
            _ => continue,
        };
        if actor {
            if cast == MAX_CAST {
                continue;
            }
            cast += 1;
        } else if out
            .iter()
            .any(|p| p.name == c.person && p.role.as_deref() == Some(role))
        {
            continue;
        }
        out.push(Person {
            name: c.person,
            role: Some(role.into()),
            character: if actor { non_empty(c.name) } else { None },
            photo_url: non_empty(c.photo).map(|photo| art_url(&photo)),
        });
    }
    out
}

/// The US rating when there is one, else the first.
fn pick_rating(ratings: Vec<ContentRating>) -> Option<String> {
    let usable: Vec<ContentRating> = ratings
        .into_iter()
        .filter(|r| !r.name.trim().is_empty())
        .collect();
    usable
        .iter()
        .find(|r| r.country.eq_ignore_ascii_case("usa"))
        .or_else(|| usable.first())
        .map(|r| r.name.trim().to_string())
}

fn names(list: Vec<Named>) -> Vec<String> {
    list.into_iter()
        .map(|n| n.name)
        .filter(|n| !n.trim().is_empty())
        .collect()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn non_empty_str(value: &str) -> Option<&str> {
    Some(value).filter(|v| !v.trim().is_empty())
}

fn year_from_date(date: &str) -> Option<i32> {
    date.get(..4)?.parse().ok()
}

// ---- matching -------------------------------------------------------------

fn normalize(title: &str) -> Vec<String> {
    let lowered: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    lowered
        .split_whitespace()
        .filter(|w| !matches!(*w, "the" | "a" | "an"))
        .map(str::to_string)
        .collect()
}

/// 1.0 for the same normalized title, else the Dice coefficient of the
/// two word sets.
fn title_similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalize(a), normalize(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let shared = a.iter().filter(|w| b.contains(w)).count();
    (2 * shared) as f32 / (a.len() + b.len()) as f32
}

/// Match confidence in [0, 1]: title similarity (against the title or any other
/// name it goes by), scaled down for a year mismatch or missing year and slightly
/// by search rank, so equally-titled remakes found without a year land close
/// together and the host can see the ambiguity.
pub fn confidence(
    query: &str,
    query_year: Option<i32>,
    title: &str,
    other_names: &[&str],
    year: Option<i32>,
    rank: usize,
) -> f32 {
    let similarity = other_names
        .iter()
        .map(|name| title_similarity(query, name))
        .fold(title_similarity(query, title), f32::max);
    let year_factor = match (query_year, year) {
        (Some(q), Some(y)) if q == y => 1.0,
        (Some(q), Some(y)) if (q - y).abs() == 1 => 0.9,
        (Some(_), Some(_)) => 0.5,
        (Some(_), None) => 0.8,
        (None, _) => 0.95,
    };
    let rank_factor = 1.0 - 0.02 * rank.min(10) as f32;
    (similarity * year_factor * rank_factor).clamp(0.0, 1.0)
}

/// A season or episode id derived from the host's search hints — the parent
/// series' id (`parent-external-id:tvdb:series/{id}`) plus `season:{n}` and, for
/// episodes, `episode:{m}`. `None` when the hints don't name a TVDB series or the
/// numbers are missing.
pub fn child_from_hints(hints: &[String], want_episode: bool) -> Option<TvdbId> {
    let value = |prefix: &str| {
        hints
            .iter()
            .find_map(|h| h.strip_prefix(prefix).map(str::trim))
    };
    let series = match TvdbId::parse(value("parent-external-id:tvdb:")?).ok()? {
        TvdbId::Series(id) => id,
        TvdbId::Season { series, .. } => series,
        _ => return None,
    };
    let season = value("season:")?.parse().ok()?;
    if !want_episode {
        return Some(TvdbId::Season { series, season });
    }
    let episode = value("episode:")?.parse().ok()?;
    Some(TvdbId::Episode {
        series,
        season,
        episode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIES: &[u8] = include_bytes!("../fixtures/series_121361.json");
    const MOVIE: &[u8] = include_bytes!("../fixtures/movie_169.json");
    const SEASON: &[u8] = include_bytes!("../fixtures/season_364731.json");
    const EPISODE: &[u8] = include_bytes!("../fixtures/episode_3254641.json");
    const LOOKUP: &[u8] = include_bytes!("../fixtures/series_121361_episodes.json");
    const SEARCH: &[u8] = include_bytes!("../fixtures/search_series.json");

    /// The real shape, `null`s included. The fixtures above are written to pin
    /// down behaviour, and can't show what the API really leaves out.
    #[test]
    fn responses_recorded_from_the_api_parse() {
        let found = parse_search(
            SearchKind::Series,
            include_bytes!("../fixtures/live_search_series.json"),
            "Caminandes",
            Some(2013),
        )
        .unwrap();
        assert_eq!(found[0].id.path(), "series/276116");

        let (movie, art) = parse_movie(include_bytes!("../fixtures/live_movie_9413.json")).unwrap();
        assert_eq!((movie.title.as_str(), movie.year), ("Sintel", Some(2010)));
        assert!(!art.is_empty());

        let (series, art) =
            parse_series(include_bytes!("../fixtures/live_series_276116.json")).unwrap();
        assert_eq!(series.title, "Caminandes");
        assert!(!art.is_empty());

        let (season, _) = parse_season(
            276116,
            1,
            include_bytes!("../fixtures/live_season_569834.json"),
        )
        .unwrap();
        assert!(
            season
                .ids
                .iter()
                .any(|(_, id)| id == "series/276116/season/1")
        );

        let (episode, _) = parse_episode(
            276116,
            1,
            1,
            include_bytes!("../fixtures/live_episode_4734685.json"),
        )
        .unwrap();
        assert_eq!(episode.title, "Llama Drama");
    }

    #[test]
    fn ids_round_trip_through_their_paths() {
        for value in [
            "movie/169",
            "series/121361",
            "series/121361/season/1",
            "series/121361/season/1/episode/1",
        ] {
            assert_eq!(TvdbId::parse(value).expect(value).path(), value);
        }
        for bad in ["121361", "series/x", "episode/5", "series/1/episode/1", ""] {
            assert!(TvdbId::parse(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn login_leaves_out_a_missing_pin() {
        let parse = |body: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(
            parse(login_body("k", None)),
            serde_json::json!({"apikey": "k"})
        );
        assert_eq!(
            parse(login_body("k", Some("p"))),
            serde_json::json!({"apikey": "k", "pin": "p"})
        );
        assert_eq!(
            parse_login(br#"{"status":"success","data":{"token":"t"}}"#).unwrap(),
            "t"
        );
    }

    #[test]
    fn search_url_narrows_by_kind_and_year() {
        assert_eq!(
            search_url(SearchKind::Series, " Game of Thrones ", Some(2011)),
            "https://api4.thetvdb.com/v4/search?query=Game%20of%20Thrones&type=series&limit=10&year=2011"
        );
        assert!(!search_url(SearchKind::Movie, "Dune", None).contains("year"));
    }

    #[test]
    fn search_scores_english_titles_and_ranks_the_exact_one_first() {
        let matches = parse_search(SearchKind::Series, SEARCH, "Game of Thrones", Some(2011))
            .expect("parses");
        assert_eq!(matches[0].id, TvdbId::Series(121361));
        assert_eq!(matches[0].title, "Game of Thrones");
        assert_eq!(matches[0].year, Some(2011));
        assert!(
            matches[0].confidence >= 0.85,
            "an exact match must clear the host's threshold, got {}",
            matches[0].confidence
        );
        assert!(matches[0].confidence - matches[1].confidence >= 0.05);
        let alias = parse_search(SearchKind::Series, SEARCH, "Le Trone de Fer", None).unwrap();
        assert_eq!(alias[0].id, TvdbId::Series(121361), "matched by alias");
    }

    #[test]
    fn series_takes_english_over_the_records_own_language() {
        let (payload, _) = parse_series(SERIES).expect("parses");
        assert_eq!(payload.title, "Game of Thrones");
        assert_eq!(payload.original_title.as_deref(), Some("Le Trône de fer"));
        assert_eq!(
            payload.overview.as_deref(),
            Some("Seven noble families fight for control of Westeros.")
        );
        assert_eq!(payload.year, Some(2011));
        assert_eq!(payload.runtime_ms, Some(57 * 60_000));
        assert_eq!(payload.genres, ["Drama", "Fantasy"]);
        assert_eq!(payload.studios, ["HBO", "Television 360"]);
        assert_eq!(payload.rating.as_deref(), Some("TV-MA"));
        assert_eq!(payload.original_language.as_deref(), Some("en"));
        assert_eq!(payload.origin_countries, ["US"]);
        assert_eq!(
            payload.ids,
            [
                (IdKind::Tvdb, "series/121361".to_string()),
                (IdKind::Imdb, "tt0944947".to_string()),
                (IdKind::Tmdb, "tv/1399".to_string()),
            ]
        );
    }

    #[test]
    fn series_people_follow_billing_and_keep_only_known_roles() {
        let (payload, _) = parse_series(SERIES).expect("parses");
        let people: Vec<(&str, &str, Option<&str>)> = payload
            .people
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.role.as_deref().unwrap(),
                    p.character.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            people,
            [
                ("Emilia Clarke", "Actor", Some("Daenerys Targaryen")),
                ("Peter Dinklage", "Actor", Some("Tyrion Lannister")),
                ("David Benioff", "Writer", None),
            ]
        );
    }

    #[test]
    fn seasons_keep_only_the_default_order() {
        assert_eq!(
            parse_seasons(SERIES).expect("parses"),
            [
                SeasonRef {
                    id: 364730,
                    number: 0
                },
                SeasonRef {
                    id: 364731,
                    number: 1
                }
            ]
        );
    }

    #[test]
    fn art_ranks_language_then_text_free_then_score() {
        let (payload, art) = parse_series(SERIES).expect("parses");
        let id = TvdbId::Series(121361);
        let poster = choose(&art, id, ArtKind::Primary, Some("en-US")).expect("poster");
        assert!(
            poster.url.ends_with("posters/english.jpg"),
            "{}",
            poster.url
        );
        assert_eq!(poster.language.as_deref(), Some("en"));
        let german = choose(&art, id, ArtKind::Primary, Some("de")).expect("poster");
        assert!(german.url.ends_with("posters/german.jpg"), "{}", german.url);
        // Backgrounds prefer no text even when one in the language scores higher.
        let backdrop = choose(&art, id, ArtKind::Backdrop, Some("en")).expect("backdrop");
        assert!(
            backdrop.url.ends_with("backgrounds/clean.jpg"),
            "{}",
            backdrop.url
        );
        let logo = choose(&art, id, ArtKind::Logo, Some("en")).expect("logo");
        assert!(logo.url.ends_with("clearlogo.png"), "{}", logo.url);
        assert!(choose(&art, id, ArtKind::Thumb, None).is_none());
        let kinds: Vec<ArtKind> = payload.images.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [ArtKind::Primary, ArtKind::Backdrop, ArtKind::Logo]);
    }

    #[test]
    fn a_relative_image_lands_on_the_artwork_host() {
        assert_eq!(
            art_url("/banners/a.jpg"),
            "https://artworks.thetvdb.com/banners/a.jpg"
        );
        assert_eq!(art_url("banners/a.jpg"), art_url("/banners/a.jpg"));
        assert_eq!(
            art_url("https://artworks.thetvdb.com/x.jpg"),
            "https://artworks.thetvdb.com/x.jpg"
        );
    }

    #[test]
    fn movie_uses_its_own_art_types_and_a_numeric_budget() {
        let (payload, art) = parse_movie(MOVIE).expect("parses");
        assert_eq!(payload.title, "The Matrix");
        assert_eq!(payload.year, Some(1999));
        assert_eq!(payload.release_date.as_deref(), Some("1999-03-31"));
        assert_eq!(payload.runtime_ms, Some(136 * 60_000));
        assert_eq!(payload.budget_usd, Some(63_000_000));
        assert_eq!(payload.studios, ["Warner Bros.", "Village Roadshow"]);
        assert_eq!(payload.rating.as_deref(), Some("R"));
        assert_eq!(
            payload.ids,
            [
                (IdKind::Tvdb, "movie/169".to_string()),
                (IdKind::Imdb, "tt0133093".to_string()),
                (IdKind::Tmdb, "movie/603".to_string()),
            ]
        );
        let poster = choose(&art, TvdbId::Movie(169), ArtKind::Primary, None).expect("poster");
        assert!(poster.url.ends_with("movies/poster.jpg"), "{}", poster.url);
    }

    #[test]
    fn season_falls_back_to_its_record_image_and_first_episode() {
        let (payload, art) = parse_season(121361, 1, SEASON).expect("parses");
        assert_eq!(payload.ids[0].1, "series/121361/season/1");
        assert_eq!(payload.title, "Season 1");
        assert_eq!(payload.year, Some(2011));
        assert_eq!(payload.release_date.as_deref(), Some("2011-04-17"));
        let id = TvdbId::Season {
            series: 121361,
            season: 1,
        };
        let poster = choose(&art, id, ArtKind::Primary, Some("en")).expect("poster");
        assert!(poster.url.ends_with("seasons/1.jpg"), "{}", poster.url);
    }

    #[test]
    fn episode_lookup_finds_tvdbs_own_number() {
        assert_eq!(parse_episode_lookup(LOOKUP).unwrap(), Some(3254641));
        assert_eq!(
            parse_episode_lookup(br#"{"data":{"episodes":[]}}"#).unwrap(),
            None
        );
    }

    #[test]
    fn episode_translates_and_uses_its_own_image_as_thumb() {
        let (payload, art) = parse_episode(121361, 1, 1, EPISODE).expect("parses");
        assert_eq!(payload.title, "Winter Is Coming");
        assert_eq!(
            payload.overview.as_deref(),
            Some("Ned Stark is torn between his family and an old friend.")
        );
        assert_eq!(payload.release_date.as_deref(), Some("2011-04-17"));
        assert_eq!(payload.runtime_ms, Some(62 * 60_000));
        assert_eq!(
            payload.ids,
            [
                (IdKind::Tvdb, "series/121361/season/1/episode/1".to_string()),
                (IdKind::Imdb, "tt1480055".to_string()),
            ]
        );
        assert_eq!(payload.people.len(), 1);
        assert_eq!(payload.people[0].role.as_deref(), Some("Director"));
        let id = TvdbId::Episode {
            series: 121361,
            season: 1,
            episode: 1,
        };
        assert!(choose(&art, id, ArtKind::Thumb, None).is_some());
    }

    #[test]
    fn children_come_from_the_parents_response() {
        let seasons = parse_series_children(121361, SERIES).expect("parses");
        assert_eq!(seasons.len(), 2);
        assert_eq!(seasons[1].ids[0].1, "series/121361/season/1");
        let episodes = parse_season_children(121361, 1, SEASON).expect("parses");
        assert_eq!(episodes[0].ids[0].1, "series/121361/season/1/episode/1");
        assert_eq!(episodes[0].title, "Winter Is Coming");
    }

    #[test]
    fn hints_derive_a_child_of_a_tvdb_series_only() {
        let hints = |parent: &str| {
            vec![
                format!("parent-external-id:{parent}"),
                "season:2".to_string(),
                "episode:3".to_string(),
            ]
        };
        assert_eq!(
            child_from_hints(&hints("tvdb:series/5"), true),
            Some(TvdbId::Episode {
                series: 5,
                season: 2,
                episode: 3
            })
        );
        assert_eq!(
            child_from_hints(&hints("tvdb:series/5"), false),
            Some(TvdbId::Season {
                series: 5,
                season: 2
            })
        );
        assert_eq!(child_from_hints(&hints("tmdb:tv/5"), true), None);
    }

    #[test]
    fn remote_ids_resolve_series_and_movies() {
        assert_eq!(
            parse_remote_id(br#"{"data":[{"series":{"id":7}}]}"#).unwrap(),
            Some(TvdbId::Series(7))
        );
        assert_eq!(
            parse_remote_id(br#"{"data":[{"movie":{"id":8}}]}"#).unwrap(),
            Some(TvdbId::Movie(8))
        );
        assert_eq!(parse_remote_id(br#"{"data":[]}"#).unwrap(), None);
        assert_eq!(tmdb_record_number("tv/1399"), Some("1399"));
        assert_eq!(tmdb_record_number("tv/1399/season/1"), None);
    }
}
