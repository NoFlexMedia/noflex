// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    model::{ExternalId, WorkKind},
    providers::metadata::Attribution,
};

/// The sources a work's single rating is taken from when nothing configures
/// them, first preferred. Community scores only: a critics' aggregate or a
/// share of positive reviews is not the same measure.
pub const DEFAULT_RATING_SOURCES: &[&str] = &["imdb", "tmdb", "tvdb", "musicbrainz"];

/// Longest source slug accepted.
const MAX_SOURCE_LEN: usize = 64;

/// One source's rating of a work, on that source's own scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    /// Whose rating it is, as a stable slug: `imdb`, `tmdb`, `tvdb`,
    /// `rotten-tomatoes`, `metacritic`, ... Not the plugin that reported it.
    pub source: String,
    pub value: f32,
    /// The best `value` can be: 10 for "7.8/10", 100 for "87%".
    pub scale: f32,
    /// How many ratings `value` summarises, when the source says.
    pub votes: Option<u64>,
}

impl Rating {
    /// Whether this can be kept: a source of lowercase ASCII letters, digits
    /// and inner hyphens, a positive finite scale, and a value within it.
    pub fn is_valid(&self) -> bool {
        is_source_slug(&self.source)
            && self.scale.is_finite()
            && self.scale > 0.0
            && self.value.is_finite()
            && (0.0..=self.scale).contains(&self.value)
    }

    /// `value` on a 0–10 scale, so sources can be compared and sorted on.
    pub fn normalized(&self) -> f32 {
        self.value * 10.0 / self.scale
    }
}

/// Whether `source` is a slug a [`Rating`] may name.
pub fn is_source_slug(source: &str) -> bool {
    !source.is_empty()
        && source.len() <= MAX_SOURCE_LEN
        && !source.starts_with('-')
        && !source.ends_with('-')
        && source
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// What to call `source` where its rating shows. A slug nobody named here
/// is spelled out from its words ("anime-planet" is "Anime Planet").
pub fn rating_source_name(source: &str) -> String {
    let known = match source {
        "imdb" => "IMDb",
        "tmdb" => "TMDb",
        "tvdb" => "TheTVDB",
        "musicbrainz" => "MusicBrainz",
        "rotten-tomatoes" => "Rotten Tomatoes",
        "metacritic" => "Metacritic",
        "trakt" => "Trakt",
        "letterboxd" => "Letterboxd",
        "anidb" => "AniDB",
        "myanimelist" => "MyAnimeList",
        "anilist" => "AniList",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    source
        .split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A provider that only adds ratings to works another provider described
/// (OMDb, ...). The host asks every one after applying a work's metadata,
/// with the ids that metadata gave, and keeps each provider's answer apart.
#[async_trait]
pub trait RatingsProvider: Send + Sync {
    fn id(&self) -> &str;

    /// How to credit this provider. Default asks for no credit.
    fn attribution(&self) -> Option<Attribution> {
        None
    }

    /// The ratings this provider has for the `kind` work `ids` name. Empty
    /// when it has none, or none of `ids` is one it can look up.
    async fn fetch(&self, ids: &[ExternalId], kind: WorkKind) -> PluginResult<Vec<Rating>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rating(source: &str, value: f32, scale: f32) -> Rating {
        Rating {
            source: source.into(),
            value,
            scale,
            votes: None,
        }
    }

    #[test]
    fn only_sensible_ratings_are_valid() {
        assert!(rating("imdb", 7.8, 10.0).is_valid());
        assert!(rating("rotten-tomatoes", 0.0, 100.0).is_valid());
        assert!(rating("metacritic", 100.0, 100.0).is_valid());
        assert!(!rating("imdb", 10.5, 10.0).is_valid(), "above its scale");
        assert!(!rating("imdb", -1.0, 10.0).is_valid());
        assert!(!rating("imdb", 5.0, 0.0).is_valid());
        assert!(!rating("imdb", f32::NAN, 10.0).is_valid());
        assert!(!rating("imdb", 5.0, f32::INFINITY).is_valid());
        for source in ["", "IMDb", "rotten tomatoes", "-imdb", "imdb-", "imdb/x"] {
            assert!(!rating(source, 5.0, 10.0).is_valid(), "{source:?}");
        }
    }

    #[test]
    fn normalizing_puts_every_scale_out_of_ten() {
        assert_eq!(rating("rotten-tomatoes", 87.0, 100.0).normalized(), 8.7);
        assert_eq!(rating("musicbrainz", 4.0, 5.0).normalized(), 8.0);
        assert_eq!(rating("imdb", 7.5, 10.0).normalized(), 7.5);
    }

    #[test]
    fn unknown_sources_are_named_from_their_slug() {
        assert_eq!(rating_source_name("imdb"), "IMDb");
        assert_eq!(rating_source_name("rotten-tomatoes"), "Rotten Tomatoes");
        assert_eq!(rating_source_name("anime-planet"), "Anime Planet");
        assert_eq!(rating_source_name("kinopoisk"), "Kinopoisk");
    }
}
