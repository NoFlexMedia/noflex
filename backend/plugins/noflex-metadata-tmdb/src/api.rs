// SPDX-License-Identifier: AGPL-3.0-only
//
// TMDb v3 request building and response parsing. Pure (bytes in, plain
// structs out) so it is testable off-target against recorded fixtures;
// the guest module maps these onto the WIT types.
//
// External-id convention for `ExternalIdKind::Tmdb` values: the API path
// of the record, which is also what this module requests.
//   movie    `movie/{id}`
//   series   `tv/{id}`
//   season   `tv/{id}/season/{n}`
//   episode  `tv/{id}/season/{n}/episode/{m}`
//   person   `person/{id}`, which only the image hook answers (a photo)

use serde::{Deserialize, Serialize};

/// TMDb v3 API root.
pub const API_BASE: &str = "https://api.themoviedb.org/3";
/// Image CDN root; a size segment (`original`, `w185`) and the file path follow.
pub const IMAGE_BASE: &str = "https://image.tmdb.org/t/p";
/// The CDN size kept for a person's photo; credits draw it small.
pub const PROFILE_SIZE: &str = "w185";
/// The CDN size of a candidate's preview in a picker grid.
pub const PREVIEW_SIZE: &str = "w342";
/// Sent on every request so TMDb can attribute traffic.
pub const USER_AGENT: &str = "NoFlex/0.0.1 (https://github.com/noflex-media/noflex)";

/// Cast members kept per payload; TMDb lists whole ensembles.
const MAX_CAST: usize = 15;
/// Search results turned into matches.
const MAX_MATCHES: usize = 10;

/// One TMDb record, addressed by the path convention in this module's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmdbId {
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

impl TmdbId {
    /// Parses the path convention above; anything else is an error.
    pub fn parse(value: &str) -> Result<Self, String> {
        let bad = || format!("not a TMDb id: {value:?}");
        let parts: Vec<&str> = value.trim().split('/').collect();
        let num = |s: &str| s.parse::<u64>().map_err(|_| bad());
        let small = |s: &str| s.parse::<u32>().map_err(|_| bad());
        match parts.as_slice() {
            ["movie", id] => Ok(Self::Movie(num(id)?)),
            ["tv", id] => Ok(Self::Series(num(id)?)),
            ["tv", id, "season", n] => Ok(Self::Season {
                series: num(id)?,
                season: small(n)?,
            }),
            ["tv", id, "season", n, "episode", m] => Ok(Self::Episode {
                series: num(id)?,
                season: small(n)?,
                episode: small(m)?,
            }),
            _ => Err(bad()),
        }
    }

    /// The API path, which is also the external-id value.
    pub fn path(&self) -> String {
        match self {
            Self::Movie(id) => format!("movie/{id}"),
            Self::Series(id) => format!("tv/{id}"),
            Self::Season { series, season } => format!("tv/{series}/season/{season}"),
            Self::Episode {
                series,
                season,
                episode,
            } => format!("tv/{series}/season/{season}/episode/{episode}"),
        }
    }
}

/// The two record types TMDb searches by title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Movie,
    Series,
}

/// Non-TMDb ids TMDb can resolve through `/find`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindSource {
    Imdb,
    Tvdb,
}

/// External-id kinds a payload can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Tmdb,
    Imdb,
    Tvdb,
}

/// The image kinds TMDb has, named after the WIT `image-kind` they map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// A credited person; `role` is `Director`, `Writer`, `Producer`, `Composer` or `Actor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub role: Option<String>,
    pub character: Option<String>,
    /// TMDb's id for them, named `person/{id}` as an external id.
    pub id: Option<u64>,
    /// Their photo's CDN file path, when the credit carries one.
    pub profile: Option<String>,
}

/// A parsed record in WIT-payload shape; empty fields mean TMDb had none.
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
    pub community_rating: Option<f32>,
    pub original_language: Option<String>,
    pub origin_countries: Vec<String>,
    pub keywords: Vec<String>,
    pub budget_usd: Option<u64>,
}

/// One search result with this plugin's confidence (see [`confidence`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub id: TmdbId,
    pub title: String,
    pub year: Option<i32>,
    pub confidence: f32,
}

/// One downloadable image chosen for an `image::fetch` request.
#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    pub url: String,
    pub language: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// One image offered to an administrator choosing artwork by hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// [`CandidateHandle`] as JSON.
    pub handle: String,
    pub language: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// TMDb's vote average for the image.
    pub score: f32,
}

/// What a candidate's opaque handle carries: enough to fetch it again
/// without another listing request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateHandle {
    #[serde(rename = "k")]
    pub kind: ArtKind,
    /// CDN file path, `/` and a file name.
    #[serde(rename = "p")]
    pub path: String,
    #[serde(rename = "l", default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(rename = "w", default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(rename = "h", default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl CandidateHandle {
    /// Parses a handle [`candidates`] made. The path must be a bare CDN file
    /// name, so a handle can only ever name an image on TMDb's CDN.
    pub fn decode(handle: &str) -> Result<Self, String> {
        let decoded: Self =
            serde_json::from_str(handle).map_err(|e| format!("tmdb: bad image handle: {e}"))?;
        let name = decoded.path.strip_prefix('/').unwrap_or_default();
        let plain = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !plain || name.starts_with('.') {
            return Err("tmdb: bad image handle path".into());
        }
        Ok(decoded)
    }

    /// The CDN URL of the image: a [`PREVIEW_SIZE`] rendition or the original.
    pub fn url(&self, preview: bool) -> String {
        image_url(if preview { PREVIEW_SIZE } else { "original" }, &self.path)
    }
}

/// NoFlex's own key, so metadata works without any setup. Anyone holding the
/// plugin can read it, so it was never going to stay private, and TMDb issues
/// it to a free application. Its terms cover non-commercial use only.
const SHARED_API_KEY: &str = "2eebc214f9c6f772dc6f859c76179249";

/// The operator's `api_key` secret when they set one, otherwise the shared key.
pub fn key_to_use(secret: Option<&str>) -> &str {
    secret
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .unwrap_or(SHARED_API_KEY)
}

/// How a request authenticates: v4 read tokens (JWTs) go in a bearer
/// header, v3 keys in the query string.
pub fn authorize(url: &str, key: &str) -> (String, Vec<(String, String)>) {
    let mut headers = vec![
        ("accept".to_string(), "application/json".to_string()),
        ("user-agent".to_string(), USER_AGENT.to_string()),
    ];
    let key = key.trim();
    if key.starts_with("eyJ") {
        headers.push(("authorization".into(), format!("Bearer {key}")));
        (url.to_string(), headers)
    } else {
        let sep = if url.contains('?') { '&' } else { '?' };
        (
            format!("{url}{sep}api_key={}", percent_encode(key)),
            headers,
        )
    }
}

/// The ISO 639-1 part of a BCP-47 tag (`en-US` -> `en`), lowercased.
pub fn primary_language(tag: &str) -> Option<String> {
    let primary = tag.split(['-', '_']).next()?.trim().to_ascii_lowercase();
    (primary.len() == 2 && primary.bytes().all(|b| b.is_ascii_alphabetic())).then_some(primary)
}

/// Title search, narrowed to `year` (release year, or first-air year for series) when given.
pub fn search_url(kind: SearchKind, title: &str, year: Option<i32>) -> String {
    let (endpoint, year_param) = match kind {
        SearchKind::Movie => ("movie", "year"),
        SearchKind::Series => ("tv", "first_air_date_year"),
    };
    let mut url = format!(
        "{API_BASE}/search/{endpoint}?query={}&include_adult=false",
        percent_encode(title.trim())
    );
    if let Some(year) = year {
        url.push_str(&format!("&{year_param}={year}"));
    }
    url
}

/// The record's details with credits, images (English and text-free), ids and ratings appended.
pub fn details_url(id: TmdbId) -> String {
    let append = match id {
        TmdbId::Movie(_) => "credits,images,release_dates,external_ids,keywords",
        TmdbId::Series(_) => "credits,images,content_ratings,external_ids,keywords",
        TmdbId::Season { .. } => "images",
        TmdbId::Episode { .. } => "credits,images,external_ids",
    };
    // `null` keeps text-free art, which backdrops are mostly made of.
    format!(
        "{API_BASE}/{}?append_to_response={append}&include_image_language=en,null",
        id.path()
    )
}

/// Image listing for `id`, narrowed to `language` plus text-free art
/// when one is given.
pub fn images_url(id: TmdbId, language: Option<&str>) -> String {
    let mut url = format!("{API_BASE}/{}/images", id.path());
    if let Some(lang) = language {
        url.push_str(&format!("?include_image_language={lang},null"));
    }
    url
}

/// The number in a `person/{id}` external id.
pub fn person_id(value: &str) -> Option<u64> {
    value.trim().strip_prefix("person/")?.parse().ok()
}

/// A person's photo listing.
pub fn person_images_url(id: u64) -> String {
    format!("{API_BASE}/person/{id}/images")
}

/// The number `/find` wants from a `series/{id}` or `movie/{id}` TVDB id, the
/// forms that name a TVDB record directly. TMDb can't look up the
/// `series/{id}/season/{n}` forms.
pub fn tvdb_record_number(value: &str) -> Option<&str> {
    match value.trim().split('/').collect::<Vec<_>>().as_slice() {
        ["series" | "movie", id] if id.bytes().all(|b| b.is_ascii_digit()) => Some(id),
        _ => None,
    }
}

/// Resolves an IMDb or TVDB id to TMDb records.
pub fn find_url(source: FindSource, id: &str) -> String {
    let param = match source {
        FindSource::Imdb => "imdb_id",
        FindSource::Tvdb => "tvdb_id",
    };
    format!(
        "{API_BASE}/find/{}?external_source={param}",
        percent_encode(id.trim())
    )
}

/// CDN URL for `file_path` (which starts with `/`) at `size`.
pub fn image_url(size: &str, file_path: &str) -> String {
    format!("{IMAGE_BASE}/{size}{file_path}")
}

/// Content type for a downloaded image: the response header when it
/// names an image type, else a guess from the file extension.
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
        "svg" => "image/svg+xml",
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

// ---- response shapes ------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct Named {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct CastMember {
    id: Option<u64>,
    #[serde(default)]
    name: String,
    character: Option<String>,
    profile_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CrewMember {
    id: Option<u64>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    job: String,
    profile_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Credits {
    #[serde(default)]
    cast: Vec<CastMember>,
    #[serde(default)]
    guest_stars: Vec<CastMember>,
    #[serde(default)]
    crew: Vec<CrewMember>,
}

/// One entry of an image listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImageEntry {
    #[serde(default)]
    file_path: String,
    iso_639_1: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    #[serde(default)]
    vote_average: f32,
    #[serde(default)]
    vote_count: u32,
}

/// The `/images` listing (also appended to detail responses).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Images {
    #[serde(default)]
    posters: Vec<ImageEntry>,
    #[serde(default)]
    backdrops: Vec<ImageEntry>,
    #[serde(default)]
    logos: Vec<ImageEntry>,
    #[serde(default)]
    stills: Vec<ImageEntry>,
    /// A person's photos, from [`person_images_url`].
    #[serde(default)]
    profiles: Vec<ImageEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct ExternalIds {
    imdb_id: Option<String>,
    tvdb_id: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct ReleaseDates {
    #[serde(default)]
    results: Vec<CountryReleases>,
}

#[derive(Debug, Default, Deserialize)]
struct CountryReleases {
    #[serde(default)]
    iso_3166_1: String,
    #[serde(default)]
    release_dates: Vec<Certification>,
}

#[derive(Debug, Default, Deserialize)]
struct Certification {
    #[serde(default)]
    certification: String,
}

#[derive(Debug, Default, Deserialize)]
struct ContentRatings {
    #[serde(default)]
    results: Vec<CountryRating>,
}

#[derive(Debug, Default, Deserialize)]
struct CountryRating {
    #[serde(default)]
    iso_3166_1: String,
    #[serde(default)]
    rating: String,
}

#[derive(Debug, Deserialize)]
struct Movie {
    id: u64,
    #[serde(default)]
    title: String,
    original_language: Option<String>,
    #[serde(default)]
    origin_country: Vec<String>,
    #[serde(default)]
    budget: u64,
    #[serde(default)]
    keywords: Keywords,
    original_title: Option<String>,
    overview: Option<String>,
    tagline: Option<String>,
    release_date: Option<String>,
    runtime: Option<i64>,
    #[serde(default)]
    genres: Vec<Named>,
    #[serde(default)]
    production_companies: Vec<Named>,
    vote_average: Option<f32>,
    imdb_id: Option<String>,
    #[serde(default)]
    credits: Credits,
    #[serde(default)]
    images: Images,
    #[serde(default)]
    release_dates: ReleaseDates,
    #[serde(default)]
    external_ids: ExternalIds,
}

/// TMDb names the list `keywords` on movies and `results` on series.
#[derive(Debug, Default, Deserialize)]
struct Keywords {
    #[serde(default, alias = "results")]
    keywords: Vec<Named>,
}

#[derive(Debug, Default, Deserialize)]
struct SeasonSummary {
    season_number: u32,
    #[serde(default)]
    name: String,
    overview: Option<String>,
    air_date: Option<String>,
    poster_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Series {
    id: u64,
    #[serde(default)]
    name: String,
    original_language: Option<String>,
    #[serde(default)]
    origin_country: Vec<String>,
    #[serde(default)]
    keywords: Keywords,
    original_name: Option<String>,
    overview: Option<String>,
    tagline: Option<String>,
    first_air_date: Option<String>,
    #[serde(default)]
    episode_run_time: Vec<i64>,
    #[serde(default)]
    genres: Vec<Named>,
    #[serde(default)]
    networks: Vec<Named>,
    #[serde(default)]
    production_companies: Vec<Named>,
    vote_average: Option<f32>,
    #[serde(default)]
    credits: Credits,
    #[serde(default)]
    images: Images,
    #[serde(default)]
    content_ratings: ContentRatings,
    #[serde(default)]
    external_ids: ExternalIds,
    #[serde(default)]
    seasons: Vec<SeasonSummary>,
}

#[derive(Debug, Default, Deserialize)]
struct EpisodeSummary {
    episode_number: u32,
    #[serde(default)]
    name: String,
    overview: Option<String>,
    air_date: Option<String>,
    runtime: Option<i64>,
    still_path: Option<String>,
    vote_average: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct Season {
    #[serde(default)]
    name: String,
    overview: Option<String>,
    air_date: Option<String>,
    poster_path: Option<String>,
    #[serde(default)]
    images: Images,
    #[serde(default)]
    episodes: Vec<EpisodeSummary>,
}

#[derive(Debug, Deserialize)]
struct Episode {
    #[serde(default)]
    name: String,
    overview: Option<String>,
    air_date: Option<String>,
    runtime: Option<i64>,
    still_path: Option<String>,
    vote_average: Option<f32>,
    #[serde(default)]
    credits: Credits,
    #[serde(default)]
    images: Images,
    #[serde(default)]
    external_ids: ExternalIds,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    id: u64,
    title: Option<String>,
    name: Option<String>,
    original_title: Option<String>,
    original_name: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FindResponse {
    #[serde(default)]
    movie_results: Vec<FindHit>,
    #[serde(default)]
    tv_results: Vec<FindHit>,
    #[serde(default)]
    tv_episode_results: Vec<FindEpisodeHit>,
}

#[derive(Debug, Deserialize)]
struct FindHit {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct FindEpisodeHit {
    show_id: u64,
    season_number: u32,
    episode_number: u32,
}

// ---- parsing --------------------------------------------------------------

fn json<'a, T: Deserialize<'a>>(body: &'a [u8], what: &str) -> Result<T, String> {
    serde_json::from_slice(body).map_err(|e| format!("tmdb: malformed {what} response: {e}"))
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
    let response: SearchResponse = json(body, "search")?;
    let mut out: Vec<Match> = response
        .results
        .into_iter()
        .take(MAX_MATCHES)
        .enumerate()
        .filter_map(|(rank, r)| {
            let (title, original, date) = match kind {
                SearchKind::Movie => (r.title, r.original_title, r.release_date),
                SearchKind::Series => (r.name, r.original_name, r.first_air_date),
            };
            let title = title.filter(|t| !t.trim().is_empty())?;
            let result_year = date.as_deref().and_then(year_from_date);
            let confidence =
                confidence(query, year, &title, original.as_deref(), result_year, rank);
            Some(Match {
                id: match kind {
                    SearchKind::Movie => TmdbId::Movie(r.id),
                    SearchKind::Series => TmdbId::Series(r.id),
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

/// The TMDb record `/find` resolved an outside id to, preferring the
/// kind the caller is fetching.
pub fn parse_find(body: &[u8], prefer_series: bool) -> Result<Option<TmdbId>, String> {
    let response: FindResponse = json(body, "find")?;
    let movie = response.movie_results.first().map(|h| TmdbId::Movie(h.id));
    let series = response.tv_results.first().map(|h| TmdbId::Series(h.id));
    let episode = response
        .tv_episode_results
        .first()
        .map(|h| TmdbId::Episode {
            series: h.show_id,
            season: h.season_number,
            episode: h.episode_number,
        });
    Ok(if prefer_series {
        series.or(episode).or(movie)
    } else {
        movie.or(series).or(episode)
    })
}

/// The payload for `id` from its [`details_url`] response.
///
/// # Errors
/// The body is not a detail response of `id`'s record type.
pub fn parse_details(id: TmdbId, body: &[u8]) -> Result<Payload, String> {
    match id {
        TmdbId::Movie(_) => parse_movie(body),
        TmdbId::Series(_) => parse_series(body),
        TmdbId::Season { series, season } => parse_season(series, season, body),
        TmdbId::Episode {
            series,
            season,
            episode,
        } => parse_episode(series, season, episode, body),
    }
}

fn parse_movie(body: &[u8]) -> Result<Payload, String> {
    let m: Movie = json(body, "movie")?;
    let mut ids = vec![(IdKind::Tmdb, TmdbId::Movie(m.id).path())];
    if let Some(imdb) = m
        .imdb_id
        .or(m.external_ids.imdb_id)
        .filter(|s| !s.is_empty())
    {
        ids.push((IdKind::Imdb, imdb));
    }
    let rating = pick_certification(
        m.release_dates
            .results
            .into_iter()
            .map(|c| {
                let cert = c
                    .release_dates
                    .into_iter()
                    .map(|r| r.certification)
                    .find(|r| !r.trim().is_empty())
                    .unwrap_or_default();
                (c.iso_3166_1, cert)
            })
            .collect(),
    );
    Ok(Payload {
        ids,
        year: m.release_date.as_deref().and_then(year_from_date),
        release_date: non_empty(m.release_date),
        original_title: non_empty(m.original_title).filter(|o| *o != m.title),
        title: m.title,
        overview: non_empty(m.overview),
        tagline: non_empty(m.tagline),
        runtime_ms: m.runtime.filter(|r| *r > 0).map(|r| r * 60_000),
        genres: names(m.genres),
        studios: names(m.production_companies),
        people: people(m.credits),
        images: payload_art(
            &m.images,
            &[ArtKind::Primary, ArtKind::Backdrop, ArtKind::Logo],
        ),
        rating,
        community_rating: rating_value(m.vote_average),
        original_language: non_empty(m.original_language),
        origin_countries: m.origin_country,
        keywords: names(m.keywords.keywords),
        // TMDb reports an unknown budget as 0.
        budget_usd: Some(m.budget).filter(|budget| *budget > 0),
    })
}

fn parse_series(body: &[u8]) -> Result<Payload, String> {
    let s: Series = json(body, "series")?;
    let mut ids = vec![(IdKind::Tmdb, TmdbId::Series(s.id).path())];
    if let Some(imdb) = s.external_ids.imdb_id.filter(|v| !v.is_empty()) {
        ids.push((IdKind::Imdb, imdb));
    }
    if let Some(tvdb) = s.external_ids.tvdb_id {
        ids.push((IdKind::Tvdb, format!("series/{tvdb}")));
    }
    let mut studios = names(s.networks);
    for studio in names(s.production_companies) {
        if !studios.contains(&studio) {
            studios.push(studio);
        }
    }
    Ok(Payload {
        ids,
        year: s.first_air_date.as_deref().and_then(year_from_date),
        release_date: non_empty(s.first_air_date),
        original_title: non_empty(s.original_name).filter(|o| *o != s.name),
        title: s.name,
        overview: non_empty(s.overview),
        tagline: non_empty(s.tagline),
        runtime_ms: s
            .episode_run_time
            .first()
            .filter(|r| **r > 0)
            .map(|r| r * 60_000),
        genres: names(s.genres),
        studios,
        people: people(s.credits),
        images: payload_art(
            &s.images,
            &[ArtKind::Primary, ArtKind::Backdrop, ArtKind::Logo],
        ),
        rating: pick_certification(
            s.content_ratings
                .results
                .into_iter()
                .map(|r| (r.iso_3166_1, r.rating))
                .collect(),
        ),
        community_rating: rating_value(s.vote_average),
        original_language: non_empty(s.original_language),
        origin_countries: s.origin_country,
        keywords: names(s.keywords.keywords),
        budget_usd: None,
    })
}

fn parse_season(series: u64, season: u32, body: &[u8]) -> Result<Payload, String> {
    let s: Season = json(body, "season")?;
    let mut images = payload_art(&s.images, &[ArtKind::Primary]);
    if images.is_empty()
        && let Some(path) = non_empty(s.poster_path)
    {
        images.push(art_from_path(ArtKind::Primary, &path));
    }
    Ok(Payload {
        ids: vec![(IdKind::Tmdb, TmdbId::Season { series, season }.path())],
        year: s.air_date.as_deref().and_then(year_from_date),
        release_date: non_empty(s.air_date),
        title: s.name,
        overview: non_empty(s.overview),
        images,
        ..Payload::default()
    })
}

fn parse_episode(series: u64, season: u32, episode: u32, body: &[u8]) -> Result<Payload, String> {
    let e: Episode = json(body, "episode")?;
    let id = TmdbId::Episode {
        series,
        season,
        episode,
    };
    let mut ids = vec![(IdKind::Tmdb, id.path())];
    if let Some(imdb) = e.external_ids.imdb_id.filter(|v| !v.is_empty()) {
        ids.push((IdKind::Imdb, imdb));
    }
    let mut images = payload_art(&e.images, &[ArtKind::Thumb]);
    if images.is_empty()
        && let Some(path) = non_empty(e.still_path)
    {
        images.push(art_from_path(ArtKind::Thumb, &path));
    }
    Ok(Payload {
        ids,
        year: e.air_date.as_deref().and_then(year_from_date),
        release_date: non_empty(e.air_date),
        title: e.name,
        overview: non_empty(e.overview),
        runtime_ms: e.runtime.filter(|r| *r > 0).map(|r| r * 60_000),
        people: people(e.credits),
        images,
        community_rating: rating_value(e.vote_average),
        ..Payload::default()
    })
}

/// Children of a series (its seasons) or a season (its episodes) from
/// the parent's detail response. Movies and episodes have none.
pub fn parse_children(parent: TmdbId, body: &[u8]) -> Result<Vec<Payload>, String> {
    match parent {
        TmdbId::Series(series) => {
            let s: Series = json(body, "series")?;
            Ok(s.seasons
                .into_iter()
                .map(|season| Payload {
                    ids: vec![(
                        IdKind::Tmdb,
                        TmdbId::Season {
                            series,
                            season: season.season_number,
                        }
                        .path(),
                    )],
                    year: season.air_date.as_deref().and_then(year_from_date),
                    release_date: non_empty(season.air_date),
                    title: season.name,
                    overview: non_empty(season.overview),
                    images: non_empty(season.poster_path)
                        .map(|p| vec![art_from_path(ArtKind::Primary, &p)])
                        .unwrap_or_default(),
                    ..Payload::default()
                })
                .collect())
        }
        TmdbId::Season { series, season } => {
            let s: Season = json(body, "season")?;
            Ok(s.episodes
                .into_iter()
                .map(|e| Payload {
                    ids: vec![(
                        IdKind::Tmdb,
                        TmdbId::Episode {
                            series,
                            season,
                            episode: e.episode_number,
                        }
                        .path(),
                    )],
                    year: e.air_date.as_deref().and_then(year_from_date),
                    release_date: non_empty(e.air_date),
                    title: e.name,
                    overview: non_empty(e.overview),
                    runtime_ms: e.runtime.filter(|r| *r > 0).map(|r| r * 60_000),
                    images: non_empty(e.still_path)
                        .map(|p| vec![art_from_path(ArtKind::Thumb, &p)])
                        .unwrap_or_default(),
                    community_rating: rating_value(e.vote_average),
                    ..Payload::default()
                })
                .collect())
        }
        TmdbId::Movie(_) | TmdbId::Episode { .. } => Ok(Vec::new()),
    }
}

/// An [`images_url`] response.
///
/// # Errors
/// The body is not an image listing.
pub fn parse_images(body: &[u8]) -> Result<Images, String> {
    json(body, "images")
}

#[derive(Debug, Deserialize)]
struct WithImages {
    images: Option<Images>,
}

/// The image listing a detail response carries via
/// `append_to_response=images`; an error when it has none.
pub fn parse_images_appended(body: &[u8]) -> Result<Images, String> {
    let details: WithImages = json(body, "details")?;
    details
        .images
        .ok_or_else(|| "tmdb: details carry no image listing".to_string())
}

/// The art kinds TMDb has for a record type.
pub fn kinds_for(id: TmdbId) -> &'static [ArtKind] {
    match id {
        TmdbId::Movie(_) | TmdbId::Series(_) => {
            &[ArtKind::Primary, ArtKind::Backdrop, ArtKind::Logo]
        }
        TmdbId::Season { .. } => &[ArtKind::Primary],
        TmdbId::Episode { .. } => &[ArtKind::Thumb],
    }
}

fn list_for(images: &Images, kind: ArtKind) -> &[ImageEntry] {
    match kind {
        ArtKind::Primary => &images.posters,
        ArtKind::Backdrop => &images.backdrops,
        ArtKind::Logo => &images.logos,
        ArtKind::Thumb => &images.stills,
    }
}

/// Picks the best `kind` image of `id` for `language` (an ISO 639-1
/// code): an image in that language first, then text-free art, then
/// anything; ties go to the higher-rated, then the wider image.
/// Backdrops and stills rank text-free art first, since text on them
/// fights the UI laid over them. `None` when `id`'s record type has no
/// such kind or the listing has no usable entry.
pub fn choose(
    images: &Images,
    id: TmdbId,
    kind: ArtKind,
    language: Option<&str>,
) -> Option<Chosen> {
    if !kinds_for(id).contains(&kind) {
        return None;
    }
    best(list_for(images, kind), kind, language)
}

fn best(list: &[ImageEntry], kind: ArtKind, language: Option<&str>) -> Option<Chosen> {
    let textless_first = matches!(kind, ArtKind::Backdrop | ArtKind::Thumb);
    let tier = |e: &ImageEntry| -> u8 {
        let lang = e.iso_639_1.as_deref().filter(|l| !l.is_empty());
        let wanted = matches!((lang, language), (Some(l), Some(w)) if l.eq_ignore_ascii_case(w));
        match (lang.is_none(), wanted, textless_first) {
            (true, _, true) | (false, true, false) => 0,
            (true, _, false) | (false, true, true) => 1,
            _ => 2,
        }
    };
    list.iter()
        .filter(|e| !e.file_path.is_empty())
        .min_by(|a, b| {
            tier(a)
                .cmp(&tier(b))
                .then(b.vote_average.total_cmp(&a.vote_average))
                .then(b.vote_count.cmp(&a.vote_count))
                .then(b.width.unwrap_or(0).cmp(&a.width.unwrap_or(0)))
        })
        .map(|e| Chosen {
            url: image_url("original", &e.file_path),
            language: e.iso_639_1.clone().filter(|l| !l.is_empty()),
            width: e.width,
            height: e.height,
        })
}

/// Every `kind` image of `id` with a file, best rated first (then most
/// voted, then widest). Empty when `id`'s record type has no such kind.
pub fn candidates(images: &Images, id: TmdbId, kind: ArtKind) -> Vec<Candidate> {
    if !kinds_for(id).contains(&kind) {
        return Vec::new();
    }
    let mut entries: Vec<&ImageEntry> = list_for(images, kind)
        .iter()
        .filter(|e| !e.file_path.is_empty())
        .collect();
    entries.sort_by(|a, b| {
        b.vote_average
            .total_cmp(&a.vote_average)
            .then(b.vote_count.cmp(&a.vote_count))
            .then(b.width.unwrap_or(0).cmp(&a.width.unwrap_or(0)))
    });
    entries
        .into_iter()
        .filter_map(|e| {
            let language = e.iso_639_1.clone().filter(|l| !l.is_empty());
            let handle = CandidateHandle {
                kind,
                path: e.file_path.clone(),
                language: language.clone(),
                width: e.width,
                height: e.height,
            };
            Some(Candidate {
                handle: serde_json::to_string(&handle).ok()?,
                language,
                width: e.width,
                height: e.height,
                score: e.vote_average,
            })
        })
        .collect()
}

/// The best-rated photo in a [`person_images_url`] listing, as a CDN file path.
pub fn best_profile(images: &Images) -> Option<String> {
    images
        .profiles
        .iter()
        .filter(|e| !e.file_path.is_empty())
        .max_by(|a, b| {
            a.vote_average
                .total_cmp(&b.vote_average)
                .then(a.vote_count.cmp(&b.vote_count))
        })
        .map(|e| e.file_path.clone())
}

/// One entry per kind with art, chosen for English (the language the
/// detail request narrows images to); the host re-requests through the
/// `image` hook in its own preferred language.
fn payload_art(images: &Images, kinds: &[ArtKind]) -> Vec<Art> {
    kinds
        .iter()
        .filter_map(|kind| {
            best(list_for(images, *kind), *kind, Some("en")).map(|c| Art {
                kind: *kind,
                url: c.url,
                language: c.language,
                width: c.width,
                height: c.height,
            })
        })
        .collect()
}

fn art_from_path(kind: ArtKind, path: &str) -> Art {
    Art {
        kind,
        url: image_url("original", path),
        language: None,
        width: None,
        height: None,
    }
}

fn people(credits: Credits) -> Vec<Person> {
    let mut out: Vec<Person> = Vec::new();
    for crew in credits.crew {
        let role = match crew.job.as_str() {
            "Director" => "Director",
            "Screenplay" | "Writer" | "Teleplay" => "Writer",
            "Producer" => "Producer",
            "Original Music Composer" => "Composer",
            _ => continue,
        };
        if crew.name.trim().is_empty()
            || out
                .iter()
                .any(|p| p.name == crew.name && p.role.as_deref() == Some(role))
        {
            continue;
        }
        out.push(Person {
            name: crew.name,
            role: Some(role.into()),
            character: None,
            id: crew.id,
            profile: non_empty(crew.profile_path),
        });
    }
    for cast in credits
        .cast
        .into_iter()
        .chain(credits.guest_stars)
        .take(MAX_CAST)
    {
        if cast.name.trim().is_empty() {
            continue;
        }
        out.push(Person {
            name: cast.name,
            role: Some("Actor".into()),
            character: non_empty(cast.character),
            id: cast.id,
            profile: non_empty(cast.profile_path),
        });
    }
    out
}

/// The US certification when there is one, else the first non-empty.
fn pick_certification(by_country: Vec<(String, String)>) -> Option<String> {
    let usable: Vec<(String, String)> = by_country
        .into_iter()
        .filter(|(_, r)| !r.trim().is_empty())
        .collect();
    usable
        .iter()
        .find(|(country, _)| country == "US")
        .or_else(|| usable.first())
        .map(|(_, rating)| rating.trim().to_string())
}

fn rating_value(vote: Option<f32>) -> Option<f32> {
    vote.filter(|v| *v > 0.0)
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

/// Match confidence in [0, 1]: title similarity (against the title or
/// the original title), scaled down for a year mismatch or missing year
/// and slightly by search rank, so equally-titled remakes found without
/// a year land close together and the host can see the ambiguity.
pub fn confidence(
    query: &str,
    query_year: Option<i32>,
    title: &str,
    original_title: Option<&str>,
    year: Option<i32>,
    rank: usize,
) -> f32 {
    let similarity = original_title
        .map(|o| title_similarity(query, o))
        .unwrap_or(0.0)
        .max(title_similarity(query, title));
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

/// A season or episode id derived from the host's search hints — the
/// parent series' id (`parent-external-id:tmdb:tv/{id}`) plus
/// `season:{n}` and, for episodes, `episode:{m}`. `None` when the hints
/// don't name a TMDb series or the numbers are missing.
pub fn child_from_hints(hints: &[String], want_episode: bool) -> Option<TmdbId> {
    let value = |prefix: &str| {
        hints
            .iter()
            .find_map(|h| h.strip_prefix(prefix).map(str::trim))
    };
    let series = match TmdbId::parse(value("parent-external-id:tmdb:")?).ok()? {
        TmdbId::Series(id) => id,
        TmdbId::Season { series, .. } => series,
        _ => return None,
    };
    let season = value("season:")?.parse().ok()?;
    if !want_episode {
        return Some(TmdbId::Season { series, season });
    }
    let episode = value("episode:")?.parse().ok()?;
    Some(TmdbId::Episode {
        series,
        season,
        episode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOVIE: &[u8] = include_bytes!("../fixtures/movie_603.json");
    const SERIES: &[u8] = include_bytes!("../fixtures/tv_1399.json");
    const SEASON: &[u8] = include_bytes!("../fixtures/tv_1399_season_1.json");
    const EPISODE: &[u8] = include_bytes!("../fixtures/tv_1399_season_1_episode_1.json");
    const SEARCH_MOVIE: &[u8] = include_bytes!("../fixtures/search_movie_dune.json");
    const SEARCH_TV: &[u8] = include_bytes!("../fixtures/search_tv_got.json");
    const IMAGES: &[u8] = include_bytes!("../fixtures/movie_603_images.json");
    const FIND: &[u8] = include_bytes!("../fixtures/find_imdb.json");

    #[test]
    fn ids_round_trip_through_the_path_convention() {
        for value in [
            "movie/603",
            "tv/1399",
            "tv/1399/season/1",
            "tv/1399/season/1/episode/1",
        ] {
            assert_eq!(TmdbId::parse(value).unwrap().path(), value);
        }
        for bad in ["603", "movie/", "movie/x", "tv/1/season", "person/1", ""] {
            assert!(TmdbId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn movie_and_series_carry_origin_keywords_and_budget() {
        let movie = parse_movie(
            br#"{"id": 1, "title": "Akira", "original_language": "ja", "origin_country": ["JP"],
                "budget": 9000000, "keywords": {"keywords": [{"name": "cyberpunk"}]}}"#,
        )
        .unwrap();
        assert_eq!(movie.original_language.as_deref(), Some("ja"));
        assert_eq!(movie.origin_countries, ["JP"]);
        assert_eq!(movie.keywords, ["cyberpunk"]);
        assert_eq!(movie.budget_usd, Some(9_000_000));

        let unknown_budget = parse_movie(br#"{"id": 2, "title": "Small", "budget": 0}"#).unwrap();
        assert_eq!(unknown_budget.budget_usd, None);

        let series = parse_series(
            br#"{"id": 3, "name": "Show", "origin_country": ["JP"],
                "keywords": {"results": [{"name": "mecha"}]}}"#,
        )
        .unwrap();
        assert_eq!(series.origin_countries, ["JP"]);
        assert_eq!(series.keywords, ["mecha"]);
    }

    #[test]
    fn urls_and_auth_follow_the_tmdb_contract() {
        assert_eq!(
            search_url(SearchKind::Movie, "Dune: Part Two", Some(2024)),
            "https://api.themoviedb.org/3/search/movie?query=Dune%3A%20Part%20Two&include_adult=false&year=2024"
        );
        assert_eq!(
            search_url(SearchKind::Series, "Game of Thrones", Some(2011)),
            "https://api.themoviedb.org/3/search/tv?query=Game%20of%20Thrones&include_adult=false&first_air_date_year=2011"
        );
        assert_eq!(
            details_url(TmdbId::Movie(603)),
            "https://api.themoviedb.org/3/movie/603?append_to_response=credits,images,release_dates,external_ids,keywords&include_image_language=en,null"
        );
        assert_eq!(
            images_url(
                TmdbId::Season {
                    series: 1,
                    season: 2
                },
                Some("de")
            ),
            "https://api.themoviedb.org/3/tv/1/season/2/images?include_image_language=de,null"
        );
        assert_eq!(
            images_url(TmdbId::Movie(603), None),
            "https://api.themoviedb.org/3/movie/603/images"
        );
        assert_eq!(
            find_url(FindSource::Imdb, "tt0133093"),
            "https://api.themoviedb.org/3/find/tt0133093?external_source=imdb_id"
        );

        let (url, headers) = authorize("https://x/3/movie/1?a=b", "abc123");
        assert_eq!(url, "https://x/3/movie/1?a=b&api_key=abc123");
        assert!(!headers.iter().any(|(k, _)| k == "authorization"));
        let (url, headers) = authorize("https://x/3/movie/1", "eyJhbGciOi.token");
        assert_eq!(url, "https://x/3/movie/1");
        assert!(headers.contains(&("authorization".into(), "Bearer eyJhbGciOi.token".into())));

        assert_eq!(key_to_use(Some(" mine\n")), "mine");
        assert_eq!(key_to_use(Some("  ")), SHARED_API_KEY);
        assert_eq!(key_to_use(None), SHARED_API_KEY);

        assert_eq!(primary_language("en-US").as_deref(), Some("en"));
        assert_eq!(primary_language("PT_br").as_deref(), Some("pt"));
        assert_eq!(primary_language("english"), None);
        assert_eq!(image_mime(Some("image/png; q=1"), "/a.jpg"), "image/png");
        assert_eq!(
            image_mime(Some("binary/octet-stream"), "/a.png"),
            "image/png"
        );
        assert_eq!(image_mime(None, "/a.jpg"), "image/jpeg");
    }

    #[test]
    fn parses_a_movie_with_people_rating_ids_and_art() {
        let p = parse_details(TmdbId::Movie(603), MOVIE).unwrap();
        assert_eq!(p.title, "The Matrix");
        assert_eq!(p.original_title, None, "same as the title");
        assert_eq!(p.year, Some(1999));
        assert_eq!(p.release_date.as_deref(), Some("1999-03-30"));
        assert_eq!(p.runtime_ms, Some(136 * 60_000));
        assert_eq!(p.tagline.as_deref(), Some("Welcome to the Real World."));
        assert_eq!(p.genres, vec!["Action", "Science Fiction"]);
        assert_eq!(
            p.studios,
            vec!["Warner Bros. Pictures", "Village Roadshow Pictures"]
        );
        assert_eq!(p.rating.as_deref(), Some("R"));
        assert_eq!(p.community_rating, Some(8.2));
        assert_eq!(
            p.ids,
            vec![
                (IdKind::Tmdb, "movie/603".to_string()),
                (IdKind::Imdb, "tt0133093".to_string())
            ]
        );
        assert_eq!(p.people[0].name, "Lana Wachowski");
        assert_eq!(p.people[0].role.as_deref(), Some("Director"));
        let neo = p.people.iter().find(|x| x.name == "Keanu Reeves").unwrap();
        assert_eq!(neo.character.as_deref(), Some("Neo"));
        assert_eq!(neo.id, Some(6384));
        assert_eq!(neo.profile.as_deref(), Some("/keanu.jpg"));
        let kinds: Vec<ArtKind> = p.images.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![ArtKind::Primary, ArtKind::Backdrop, ArtKind::Logo]
        );
        assert_eq!(
            p.images[0].url,
            "https://image.tmdb.org/t/p/original/poster-en-best.jpg"
        );
        assert_eq!(
            p.images[1].url,
            "https://image.tmdb.org/t/p/original/backdrop-textless.jpg"
        );
    }

    #[test]
    fn parses_series_season_and_episode() {
        let s = parse_details(TmdbId::Series(1399), SERIES).unwrap();
        assert_eq!(s.title, "Game of Thrones");
        assert_eq!(s.year, Some(2011));
        assert_eq!(s.runtime_ms, Some(60 * 60_000));
        assert_eq!(s.rating.as_deref(), Some("TV-MA"));
        assert_eq!(
            s.studios,
            vec!["HBO", "Revolution Sun Studios", "Television 360"]
        );
        assert_eq!(
            s.ids,
            vec![
                (IdKind::Tmdb, "tv/1399".to_string()),
                (IdKind::Imdb, "tt0944947".to_string()),
                (IdKind::Tvdb, "series/121361".to_string())
            ]
        );
        assert_eq!(s.images[0].kind, ArtKind::Primary);

        let season = parse_details(
            TmdbId::Season {
                series: 1399,
                season: 1,
            },
            SEASON,
        )
        .unwrap();
        assert_eq!(season.title, "Season 1");
        assert_eq!(season.ids[0].1, "tv/1399/season/1");
        assert_eq!(season.images.len(), 1);
        assert_eq!(season.images[0].kind, ArtKind::Primary);

        let ep = parse_details(
            TmdbId::Episode {
                series: 1399,
                season: 1,
                episode: 1,
            },
            EPISODE,
        )
        .unwrap();
        assert_eq!(ep.title, "Winter Is Coming");
        assert_eq!(ep.ids[0].1, "tv/1399/season/1/episode/1");
        assert_eq!(ep.runtime_ms, Some(62 * 60_000));
        assert_eq!(ep.images[0].kind, ArtKind::Thumb);
        assert!(
            ep.people
                .iter()
                .any(|p| p.role.as_deref() == Some("Director"))
        );
    }

    #[test]
    fn children_list_seasons_and_episodes() {
        let seasons = parse_children(TmdbId::Series(1399), SERIES).unwrap();
        let ids: Vec<&str> = seasons.iter().map(|s| s.ids[0].1.as_str()).collect();
        assert_eq!(ids, vec!["tv/1399/season/0", "tv/1399/season/1"]);
        let episodes = parse_children(
            TmdbId::Season {
                series: 1399,
                season: 1,
            },
            SEASON,
        )
        .unwrap();
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[1].ids[0].1, "tv/1399/season/1/episode/2");
        assert_eq!(episodes[1].title, "The Kingsroad");
        assert!(
            parse_children(TmdbId::Movie(603), MOVIE)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn search_ranks_exact_title_and_year_highest() {
        let with_year = parse_search(SearchKind::Movie, SEARCH_MOVIE, "Dune", Some(1984)).unwrap();
        assert_eq!(with_year[0].id, TmdbId::Movie(841));
        assert_eq!(with_year[0].year, Some(1984));
        assert!(with_year[0].confidence > 0.9);
        assert!(
            with_year[1].confidence < 0.6,
            "the 2021 film misses the year"
        );

        // Without a year the two "Dune"s are near-ties — ambiguous.
        let no_year = parse_search(SearchKind::Movie, SEARCH_MOVIE, "Dune", None).unwrap();
        assert!(no_year[0].confidence - no_year[1].confidence < 0.05);

        let tv =
            parse_search(SearchKind::Series, SEARCH_TV, "Game of Thrones", Some(2011)).unwrap();
        assert_eq!(tv[0].id, TmdbId::Series(1399));
        assert_eq!(tv[0].confidence, 1.0);
    }

    #[test]
    fn confidence_tracks_title_and_year() {
        assert_eq!(
            confidence("The Matrix", Some(1999), "Matrix", None, Some(1999), 0),
            1.0
        );
        assert!(
            confidence(
                "Matrix Reloaded",
                Some(2003),
                "The Matrix",
                None,
                Some(1999),
                0
            ) < 0.5
        );
        assert!(
            confidence(
                "Amelie",
                None,
                "Le Fabuleux Destin",
                Some("Amelie"),
                None,
                0
            ) > 0.9
        );
    }

    #[test]
    fn image_choice_honours_language_and_textless_art() {
        let images = parse_images(IMAGES).unwrap();
        let movie = TmdbId::Movie(603);
        let en = choose(&images, movie, ArtKind::Primary, Some("en")).unwrap();
        assert_eq!(
            en.url,
            "https://image.tmdb.org/t/p/original/poster-en-best.jpg"
        );
        assert_eq!(en.language.as_deref(), Some("en"));
        let de = choose(&images, movie, ArtKind::Primary, Some("de")).unwrap();
        assert_eq!(de.url, "https://image.tmdb.org/t/p/original/poster-de.jpg");
        let fr = choose(&images, movie, ArtKind::Primary, Some("fr")).unwrap();
        assert_eq!(
            fr.url, "https://image.tmdb.org/t/p/original/poster-textless.jpg",
            "no French poster: text-free beats another language"
        );
        let backdrop = choose(&images, movie, ArtKind::Backdrop, Some("en")).unwrap();
        assert_eq!(
            backdrop.url,
            "https://image.tmdb.org/t/p/original/backdrop-textless.jpg"
        );
        assert!(choose(&images, movie, ArtKind::Thumb, Some("en")).is_none());
        let appended = parse_images_appended(MOVIE).unwrap();
        assert_eq!(
            choose(&appended, movie, ArtKind::Logo, Some("en")).map(|c| c.url),
            Some("https://image.tmdb.org/t/p/original/logo-en.png".to_string())
        );
        assert!(parse_images_appended(FIND).is_err());
        assert!(
            choose(
                &images,
                TmdbId::Season {
                    series: 1,
                    season: 1
                },
                ArtKind::Logo,
                None
            )
            .is_none()
        );
    }

    #[test]
    fn find_and_hints_resolve_ids() {
        assert_eq!(parse_find(FIND, false).unwrap(), Some(TmdbId::Movie(603)));
        assert_eq!(parse_find(br#"{"movie_results":[]}"#, false).unwrap(), None);
        let hints = vec![
            "parent-external-id:tmdb:tv/1399".to_string(),
            "season:2".to_string(),
            "episode:5".to_string(),
        ];
        assert_eq!(
            child_from_hints(&hints, false),
            Some(TmdbId::Season {
                series: 1399,
                season: 2
            })
        );
        assert_eq!(
            child_from_hints(&hints, true),
            Some(TmdbId::Episode {
                series: 1399,
                season: 2,
                episode: 5
            })
        );
        assert_eq!(child_from_hints(&hints[1..], true), None, "no parent id");
        assert_eq!(
            child_from_hints(
                &["parent-external-id:tmdb:movie/1".into(), "season:1".into()],
                false
            ),
            None
        );
    }

    #[test]
    fn candidates_rank_by_rating_and_round_trip_their_handles() {
        let images = parse_images(IMAGES).unwrap();
        let movie = TmdbId::Movie(603);
        let posters = candidates(&images, movie, ArtKind::Primary);
        assert_eq!(posters.len(), 4);
        assert!(posters.windows(2).all(|w| w[0].score >= w[1].score));
        let best = CandidateHandle::decode(&posters[0].handle).unwrap();
        assert_eq!(best.path, "/poster-en-best.jpg");
        assert_eq!(best.kind, ArtKind::Primary);
        assert_eq!(
            best.url(true),
            "https://image.tmdb.org/t/p/w342/poster-en-best.jpg"
        );
        assert_eq!(
            best.url(false),
            "https://image.tmdb.org/t/p/original/poster-en-best.jpg"
        );
        assert!(candidates(&images, movie, ArtKind::Thumb).is_empty());
        let season = TmdbId::Season {
            series: 1,
            season: 1,
        };
        assert!(candidates(&images, season, ArtKind::Logo).is_empty());
    }

    #[test]
    fn handles_name_only_cdn_files() {
        for bad in [
            r#"{"k":"primary","p":"https://evil.example/x.jpg"}"#,
            r#"{"k":"primary","p":"/../x.jpg"}"#,
            r#"{"k":"primary","p":"/a/b.jpg"}"#,
            r#"{"k":"primary","p":"/"}"#,
            r#"{"k":"primary","p":"x.jpg"}"#,
            "not json",
        ] {
            assert!(CandidateHandle::decode(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn people_are_named_by_their_tmdb_id() {
        assert_eq!(person_id("person/6384"), Some(6384));
        assert_eq!(person_id("movie/603"), None);
        assert_eq!(person_id("person/x"), None);
        let images = parse_images(
            br#"{"profiles": [
                {"file_path": "/low.jpg", "vote_average": 1.0},
                {"file_path": "/high.jpg", "vote_average": 5.0},
                {"file_path": "", "vote_average": 9.0}
            ]}"#,
        )
        .unwrap();
        assert_eq!(best_profile(&images).as_deref(), Some("/high.jpg"));
        assert_eq!(best_profile(&Images::default()), None);
    }

    #[test]
    fn malformed_responses_fail_without_network() {
        assert!(parse_details(TmdbId::Movie(1), b"not-json").is_err());
        assert!(parse_search(SearchKind::Movie, b"{", "x", None).is_err());
        assert!(parse_images(b"\"posters\"").is_err());
    }
}
