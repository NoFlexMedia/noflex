// SPDX-License-Identifier: AGPL-3.0-only
//
// OMDb's by-id lookup, `GET https://www.omdbapi.com/?apikey=<key>&i=<imdb id>`,
// and its answer. Pure (bytes in, ratings out) so it is testable off-target.
//
// A found title answers `"Response":"True"` with a `Ratings` list of
// `{"Source", "Value"}` pairs ("Internet Movie Database" "7.8/10", "Rotten
// Tomatoes" "87%", "Metacritic" "74/100") beside `imdbRating`, `imdbVotes`
// ("1,234,567") and `Metascore`, any of them "N/A" when OMDb has none.
// Anything else answers `"Response":"False"` with an `Error` sentence, under
// HTTP 200 or 401 alike, so the sentence is what says what went wrong.

use serde::Deserialize;

pub const API_BASE: &str = "https://www.omdbapi.com/";
pub const USER_AGENT: &str = "NoFlex/0.0.1 (https://github.com/noflex-media/noflex)";

/// How long a spent daily allowance keeps the plugin from asking again. OMDb
/// doesn't say when it resets.
pub const LIMIT_BACKOFF_SECS: u32 = 60 * 60;

/// One rating as the plugin reports it, with its source's slug.
#[derive(Debug, Clone, PartialEq)]
pub struct Rating {
    pub source: String,
    pub value: f32,
    pub scale: f32,
    pub votes: Option<u64>,
}

/// What a lookup came back with.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Ratings(Vec<Rating>),
    /// OMDb has no such title, or the id isn't one it can look up.
    NotFound,
    /// The key is missing, wrong, or not activated yet.
    KeyRefused(String),
    /// The key's daily allowance is spent.
    LimitReached(String),
    /// Any other refusal, in OMDb's words.
    Refused(String),
}

/// The URL looking `imdb_id` up with `key`, or `None` for an id that isn't
/// IMDb's `tt` and digits, which OMDb would only refuse.
pub fn lookup_url(imdb_id: &str, key: &str) -> Option<String> {
    let digits = imdb_id.strip_prefix("tt")?;
    if digits.len() < 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "{API_BASE}?apikey={}&i={imdb_id}",
        percent_encode(key)
    ))
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(rename = "Response")]
    response: String,
    #[serde(rename = "Error")]
    error: Option<String>,
    #[serde(rename = "Ratings", default)]
    ratings: Vec<SourceRating>,
    #[serde(rename = "imdbRating")]
    imdb_rating: Option<String>,
    #[serde(rename = "imdbVotes")]
    imdb_votes: Option<String>,
    #[serde(rename = "Metascore")]
    metascore: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SourceRating {
    #[serde(rename = "Source")]
    source: String,
    #[serde(rename = "Value")]
    value: String,
}

/// Read a lookup's body. `Err` only when it isn't OMDb's JSON at all.
pub fn parse(body: &[u8]) -> Result<Answer, String> {
    let response: Response =
        serde_json::from_slice(body).map_err(|e| format!("parse OMDb response: {e}"))?;
    if !response.response.eq_ignore_ascii_case("true") {
        let error = response.error.unwrap_or_default();
        let lower = error.to_ascii_lowercase();
        return Ok(
            if lower.contains("not found") || lower.contains("incorrect imdb id") {
                Answer::NotFound
            } else if lower.contains("api key") {
                Answer::KeyRefused(error)
            } else if lower.contains("limit reached") {
                Answer::LimitReached(error)
            } else {
                Answer::Refused(error)
            },
        );
    }

    let votes = response.imdb_votes.as_deref().and_then(parse_votes);
    let mut out: Vec<Rating> = Vec::new();
    for rating in &response.ratings {
        let Some((value, scale)) = parse_value(&rating.value) else {
            continue;
        };
        let source = source_slug(&rating.source);
        if source.is_empty() || out.iter().any(|r| r.source == source) {
            continue;
        }
        out.push(Rating {
            votes: if source == "imdb" { votes } else { None },
            source,
            value,
            scale,
        });
    }
    // Episodes often carry these without a `Ratings` entry for them.
    if !out.iter().any(|r| r.source == "imdb")
        && let Some(value) = response.imdb_rating.as_deref().and_then(parse_number)
    {
        out.push(Rating {
            source: "imdb".into(),
            value,
            scale: 10.0,
            votes,
        });
    }
    if !out.iter().any(|r| r.source == "metacritic")
        && let Some(value) = response.metascore.as_deref().and_then(parse_number)
    {
        out.push(Rating {
            source: "metacritic".into(),
            value,
            scale: 100.0,
            votes: None,
        });
    }
    out.retain(|r| r.scale > 0.0 && (0.0..=r.scale).contains(&r.value));
    Ok(Answer::Ratings(out))
}

/// NoFlex's slug for one of OMDb's source names. A source this doesn't know
/// yet gets its name in slug form, so it still shows.
fn source_slug(name: &str) -> String {
    match name.trim() {
        "Internet Movie Database" => "imdb".into(),
        "Rotten Tomatoes" => "rotten-tomatoes".into(),
        "Metacritic" => "metacritic".into(),
        other => {
            let mut slug = String::new();
            for c in other.chars() {
                if c.is_ascii_alphanumeric() {
                    slug.push(c.to_ascii_lowercase());
                } else if !slug.is_empty() && !slug.ends_with('-') {
                    slug.push('-');
                }
            }
            slug.trim_end_matches('-').to_string()
        }
    }
}

/// "7.8/10" is 7.8 of 10, "87%" 87 of 100. Anything else, "N/A" included,
/// is no rating.
fn parse_value(value: &str) -> Option<(f32, f32)> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return Some((parse_number(percent)?, 100.0));
    }
    let (value, scale) = value.split_once('/')?;
    Some((parse_number(value)?, parse_number(scale)?))
}

fn parse_number(value: &str) -> Option<f32> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|number| number.is_finite())
}

/// "1,234,567".
fn parse_votes(votes: &str) -> Option<u64> {
    let digits: String = votes.chars().filter(|c| *c != ',').collect();
    digits.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Neither is recorded: there's no key to record with. This one is built
    // from OMDb's documented response shape with the ratings of the
    // `i=tt3896198` answer quoted in omdbapi/OMDb-API#102.
    const MOVIE: &[u8] = include_bytes!("../fixtures/movie_tt3896198.json");
    // An episode in the same shape, with illustrative numbers: OMDb rates
    // one with IMDb's top-level fields and an empty `Ratings`.
    const EPISODE: &[u8] = include_bytes!("../fixtures/episode_tt1480055.json");

    fn rating(source: &str, value: f32, scale: f32, votes: Option<u64>) -> Rating {
        Rating {
            source: source.into(),
            value,
            scale,
            votes,
        }
    }

    #[test]
    fn urls_need_an_imdb_id_and_escape_the_key() {
        assert_eq!(
            lookup_url("tt3896198", "ab12cd34").as_deref(),
            Some("https://www.omdbapi.com/?apikey=ab12cd34&i=tt3896198")
        );
        assert_eq!(
            lookup_url("tt0133093", "a&b=c").as_deref(),
            Some("https://www.omdbapi.com/?apikey=a%26b%3Dc&i=tt0133093")
        );
        for id in [
            "3896198",
            "tt123",
            "tt38961980x",
            "nm0000206",
            "tt3896198&r=xml",
        ] {
            assert_eq!(lookup_url(id, "key"), None, "{id}");
        }
    }

    #[test]
    fn a_movie_has_every_source_on_its_own_scale() {
        assert_eq!(
            parse(MOVIE).unwrap(),
            Answer::Ratings(vec![
                rating("imdb", 7.7, 10.0, Some(416_247)),
                rating("rotten-tomatoes", 83.0, 100.0, None),
                rating("metacritic", 67.0, 100.0, None),
            ])
        );
    }

    #[test]
    fn an_episode_is_rated_from_the_top_level_fields() {
        assert_eq!(
            parse(EPISODE).unwrap(),
            Answer::Ratings(vec![rating("imdb", 8.9, 10.0, Some(52_118))])
        );
    }

    #[test]
    fn missing_and_unknown_values_are_skipped_or_kept_by_name() {
        let body = br#"{"Response":"True","Ratings":[
            {"Source":"Rotten Tomatoes","Value":"N/A"},
            {"Source":"Some New Site","Value":"4.5/5"},
            {"Source":"Metacritic","Value":"120/100"}
        ],"imdbRating":"N/A","imdbVotes":"N/A","Metascore":"N/A"}"#;
        assert_eq!(
            parse(body).unwrap(),
            Answer::Ratings(vec![rating("some-new-site", 4.5, 5.0, None)])
        );
    }

    #[test]
    fn refusals_say_what_went_wrong() {
        let refusal = |error: &str| {
            parse(format!(r#"{{"Response":"False","Error":"{error}"}}"#).as_bytes()).unwrap()
        };
        assert_eq!(refusal("Movie not found!"), Answer::NotFound);
        assert_eq!(refusal("Series or season not found!"), Answer::NotFound);
        assert_eq!(refusal("Incorrect IMDb ID."), Answer::NotFound);
        assert_eq!(
            refusal("Invalid API key!"),
            Answer::KeyRefused("Invalid API key!".into())
        );
        assert_eq!(
            refusal("No API key provided."),
            Answer::KeyRefused("No API key provided.".into())
        );
        assert_eq!(
            refusal("Request limit reached!"),
            Answer::LimitReached("Request limit reached!".into())
        );
        assert_eq!(
            refusal("Daily request limit reached!"),
            Answer::LimitReached("Daily request limit reached!".into())
        );
        assert_eq!(
            refusal("Something went wrong."),
            Answer::Refused("Something went wrong.".into())
        );
        assert!(parse(b"<html>Service Unavailable</html>").is_err());
    }
}
