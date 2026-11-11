// SPDX-License-Identifier: AGPL-3.0-only

use serde::Deserialize;

pub const API_BASE: &str = "https://musicbrainz.org/ws/2";
pub const USER_AGENT: &str = "NoFlex/0.0.1 (https://github.com/noflex-media/noflex)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Artist,
    Album,
    Track,
}

impl EntityKind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "release-group",
            Self::Track => "recording",
        }
    }

    fn query_field(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "releasegroup",
            Self::Track => "recording",
        }
    }

    fn includes(self) -> &'static str {
        match self {
            Self::Artist => "genres+ratings",
            Self::Album | Self::Track => "artist-credits+genres+releases+ratings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Artist,
    ReleaseGroup,
    Recording,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub id_kind: IdKind,
    pub id: String,
    pub title: String,
    pub year: Option<i32>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Payload {
    pub external_id_kind: IdKind,
    pub external_id: String,
    pub title: String,
    pub sort_title: Option<String>,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    pub overview: Option<String>,
    pub runtime_ms: Option<i64>,
    pub genres: Vec<String>,
    pub people: Vec<Person>,
    /// MusicBrainz users' average, out of 5.
    pub rating: Option<Score>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub average: f32,
    pub votes: u64,
}

pub fn headers() -> Vec<(String, String)> {
    vec![
        ("accept".into(), "application/json".into()),
        ("user-agent".into(), USER_AGENT.into()),
    ]
}

pub fn search_url(kind: EntityKind, title: &str, year: Option<i32>) -> String {
    let escaped = title.replace(['\\', '"'], " ");
    let mut query = format!("{}:\"{}\"", kind.query_field(), escaped.trim());
    if let Some(year) = year {
        let field = match kind {
            EntityKind::Artist => "begin",
            EntityKind::Album => "firstreleasedate",
            EntityKind::Track => "firstreleasedate",
        };
        query.push_str(&format!(" AND {field}:{year}"));
    }
    format!(
        "{}/{}/?query={}&fmt=json&limit=25",
        API_BASE,
        kind.endpoint(),
        percent_encode(&query)
    )
}

pub fn fetch_url(kind: EntityKind, id: &str) -> Result<String, String> {
    if !is_mbid(id) {
        return Err("invalid MusicBrainz identifier".into());
    }
    Ok(format!(
        "{}/{}/{}?fmt=json&inc={}",
        API_BASE,
        kind.endpoint(),
        id,
        kind.includes()
    ))
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

fn is_mbid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    artists: Vec<SearchArtist>,
    #[serde(rename = "release-groups", default)]
    release_groups: Vec<SearchReleaseGroup>,
    #[serde(default)]
    recordings: Vec<SearchRecording>,
}

#[derive(Debug, Deserialize)]
struct SearchArtist {
    id: String,
    name: String,
    score: Option<u16>,
    #[serde(rename = "life-span", default)]
    life_span: LifeSpan,
}

#[derive(Debug, Deserialize)]
struct SearchReleaseGroup {
    id: String,
    title: String,
    score: Option<u16>,
    #[serde(rename = "first-release-date")]
    first_release_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchRecording {
    id: String,
    title: String,
    score: Option<u16>,
    #[serde(default)]
    releases: Vec<Release>,
}

pub fn parse_search(kind: EntityKind, body: &[u8]) -> Result<Vec<Match>, String> {
    let response: SearchResponse =
        serde_json::from_slice(body).map_err(|e| format!("parse search response: {e}"))?;
    let matches = match kind {
        EntityKind::Artist => response
            .artists
            .into_iter()
            .map(|item| Match {
                id_kind: IdKind::Artist,
                id: item.id,
                title: item.name,
                year: item.life_span.begin.as_deref().and_then(year_from_date),
                confidence: confidence(item.score),
            })
            .collect(),
        EntityKind::Album => response
            .release_groups
            .into_iter()
            .map(|item| Match {
                id_kind: IdKind::ReleaseGroup,
                id: item.id,
                title: item.title,
                year: item.first_release_date.as_deref().and_then(year_from_date),
                confidence: confidence(item.score),
            })
            .collect(),
        EntityKind::Track => response
            .recordings
            .into_iter()
            .map(|item| Match {
                id_kind: IdKind::Recording,
                id: item.id,
                title: item.title,
                year: earliest_release_date(&item.releases)
                    .as_deref()
                    .and_then(year_from_date),
                confidence: confidence(item.score),
            })
            .collect(),
    };
    Ok(matches)
}

fn confidence(score: Option<u16>) -> f32 {
    f32::from(score.unwrap_or(0).min(100)) / 100.0
}

#[derive(Debug, Default, Deserialize)]
struct LifeSpan {
    begin: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtistResponse {
    id: String,
    name: String,
    #[serde(rename = "sort-name")]
    sort_name: Option<String>,
    disambiguation: Option<String>,
    #[serde(rename = "life-span", default)]
    life_span: LifeSpan,
    #[serde(default)]
    genres: Vec<Genre>,
    rating: Option<RatingResponse>,
}

#[derive(Debug, Deserialize)]
struct ReleaseGroupResponse {
    id: String,
    title: String,
    #[serde(rename = "first-release-date")]
    first_release_date: Option<String>,
    #[serde(default)]
    genres: Vec<Genre>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<ArtistCredit>,
    rating: Option<RatingResponse>,
}

#[derive(Debug, Deserialize)]
struct RecordingResponse {
    id: String,
    title: String,
    length: Option<i64>,
    #[serde(default)]
    genres: Vec<Genre>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default)]
    releases: Vec<Release>,
    rating: Option<RatingResponse>,
}

/// Null with no votes.
#[derive(Debug, Deserialize)]
struct RatingResponse {
    value: Option<f32>,
    #[serde(rename = "votes-count", default)]
    votes_count: u64,
}

impl RatingResponse {
    fn score(self) -> Option<Score> {
        self.value
            .filter(|_| self.votes_count > 0)
            .map(|average| Score {
                average,
                votes: self.votes_count,
            })
    }
}

#[derive(Debug, Deserialize)]
struct Genre {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ArtistCredit {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    date: Option<String>,
}

pub fn parse_fetch(kind: EntityKind, body: &[u8]) -> Result<Payload, String> {
    match kind {
        EntityKind::Artist => {
            let item: ArtistResponse =
                serde_json::from_slice(body).map_err(|e| format!("parse artist response: {e}"))?;
            Ok(Payload {
                external_id_kind: IdKind::Artist,
                external_id: item.id,
                title: item.name,
                sort_title: item.sort_name,
                year: item.life_span.begin.as_deref().and_then(year_from_date),
                release_date: item.life_span.begin,
                overview: item.disambiguation.filter(|value| !value.is_empty()),
                runtime_ms: None,
                genres: genre_names(item.genres),
                people: Vec::new(),
                rating: item.rating.and_then(RatingResponse::score),
            })
        }
        EntityKind::Album => {
            let item: ReleaseGroupResponse = serde_json::from_slice(body)
                .map_err(|e| format!("parse release-group response: {e}"))?;
            Ok(Payload {
                external_id_kind: IdKind::ReleaseGroup,
                external_id: item.id,
                title: item.title,
                sort_title: None,
                year: item.first_release_date.as_deref().and_then(year_from_date),
                release_date: item.first_release_date,
                overview: None,
                runtime_ms: None,
                genres: genre_names(item.genres),
                people: people(item.artist_credit),
                rating: item.rating.and_then(RatingResponse::score),
            })
        }
        EntityKind::Track => {
            let item: RecordingResponse = serde_json::from_slice(body)
                .map_err(|e| format!("parse recording response: {e}"))?;
            let release_date = earliest_release_date(&item.releases);
            Ok(Payload {
                external_id_kind: IdKind::Recording,
                external_id: item.id,
                title: item.title,
                sort_title: None,
                year: release_date.as_deref().and_then(year_from_date),
                release_date,
                overview: None,
                runtime_ms: item.length,
                genres: genre_names(item.genres),
                people: people(item.artist_credit),
                rating: item.rating.and_then(RatingResponse::score),
            })
        }
    }
}

fn genre_names(genres: Vec<Genre>) -> Vec<String> {
    genres.into_iter().map(|genre| genre.name).collect()
}

fn people(credits: Vec<ArtistCredit>) -> Vec<Person> {
    credits
        .into_iter()
        .map(|credit| Person {
            name: credit.name,
            role: Some("Artist".into()),
        })
        .collect()
}

fn earliest_release_date(releases: &[Release]) -> Option<String> {
    releases
        .iter()
        .filter_map(|release| release.date.as_deref())
        .min()
        .map(str::to_string)
}

fn year_from_date(date: &str) -> Option<i32> {
    date.get(..4)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTIST_ID: &str = "f27ec8db-af05-4f36-916e-3d57f91ecf5e";
    const ALBUM_ID: &str = "b1392450-e666-3926-a536-22c65f834433";
    const TRACK_ID: &str = "9f9cf187-d6f9-437f-9d98-d59cdbd52757";

    #[test]
    fn urls_and_headers_follow_musicbrainz_contract() {
        assert_eq!(
            headers(),
            vec![
                ("accept".into(), "application/json".into()),
                ("user-agent".into(), USER_AGENT.into()),
            ]
        );
        assert_eq!(
            search_url(EntityKind::Album, "Kind of Blue", Some(1959)),
            "https://musicbrainz.org/ws/2/release-group/?query=releasegroup%3A%22Kind%20of%20Blue%22%20AND%20firstreleasedate%3A1959&fmt=json&limit=25"
        );
        assert_eq!(
            fetch_url(EntityKind::Track, TRACK_ID).unwrap(),
            format!(
                "{API_BASE}/recording/{TRACK_ID}?fmt=json&inc=artist-credits+genres+releases+ratings"
            )
        );
        assert!(fetch_url(EntityKind::Artist, "not-an-mbid").is_err());
    }

    #[test]
    fn parses_search_fixtures_for_all_music_kinds() {
        let artist = br#"{"artists":[{"id":"f27ec8db-af05-4f36-916e-3d57f91ecf5e","name":"Michael Jackson","score":98,"life-span":{"begin":"1958-08-29"}}]}"#;
        let album = br#"{"release-groups":[{"id":"b1392450-e666-3926-a536-22c65f834433","title":"Kind of Blue","score":100,"first-release-date":"1959-08-17"}]}"#;
        let track = br#"{"recordings":[{"id":"9f9cf187-d6f9-437f-9d98-d59cdbd52757","title":"So What","score":95,"releases":[{"date":"1960"},{"date":"1959-08-17"}]}]}"#;

        let a = parse_search(EntityKind::Artist, artist).unwrap();
        assert_eq!((a[0].id_kind, a[0].year), (IdKind::Artist, Some(1958)));
        assert_eq!(a[0].confidence, 0.98);
        let a = parse_search(EntityKind::Album, album).unwrap();
        assert_eq!(
            (a[0].id_kind, a[0].year),
            (IdKind::ReleaseGroup, Some(1959))
        );
        let a = parse_search(EntityKind::Track, track).unwrap();
        assert_eq!((a[0].id_kind, a[0].year), (IdKind::Recording, Some(1959)));
    }

    #[test]
    fn parses_artist_album_and_track_payloads() {
        let artist = format!(
            r#"{{"id":"{ARTIST_ID}","name":"Miles Davis","sort-name":"Davis, Miles","disambiguation":"American jazz trumpeter","life-span":{{"begin":"1926-05-26"}},"genres":[{{"name":"jazz"}}]}}"#
        );
        let album = format!(
            r#"{{"id":"{ALBUM_ID}","title":"Kind of Blue","first-release-date":"1959-08-17","artist-credit":[{{"name":"Miles Davis"}}],"genres":[{{"name":"modal jazz"}}],"rating":{{"value":4.55,"votes-count":89}}}}"#
        );
        let track = format!(
            r#"{{"id":"{TRACK_ID}","title":"So What","length":562000,"artist-credit":[{{"name":"Miles Davis"}}],"genres":[{{"name":"jazz"}}],"releases":[{{"date":"1960"}},{{"date":"1959-08-17"}}],"rating":{{"value":null,"votes-count":0}}}}"#
        );

        let a = parse_fetch(EntityKind::Artist, artist.as_bytes()).unwrap();
        assert_eq!(a.external_id_kind, IdKind::Artist);
        assert_eq!(a.sort_title.as_deref(), Some("Davis, Miles"));
        assert_eq!(a.overview.as_deref(), Some("American jazz trumpeter"));
        let a = parse_fetch(EntityKind::Album, album.as_bytes()).unwrap();
        assert_eq!(a.external_id_kind, IdKind::ReleaseGroup);
        assert_eq!(a.people[0].role.as_deref(), Some("Artist"));
        assert_eq!(
            a.rating,
            Some(Score {
                average: 4.55,
                votes: 89
            })
        );
        let a = parse_fetch(EntityKind::Track, track.as_bytes()).unwrap();
        assert_eq!(a.external_id_kind, IdKind::Recording);
        assert_eq!(a.runtime_ms, Some(562000));
        assert_eq!(a.release_date.as_deref(), Some("1959-08-17"));
        assert_eq!(a.rating, None, "an unrated recording");
    }

    #[test]
    fn malformed_responses_fail_without_network() {
        assert!(parse_search(EntityKind::Artist, b"not-json").is_err());
        assert!(parse_fetch(EntityKind::Track, br#"{"title":"missing id"}"#).is_err());
    }
}
