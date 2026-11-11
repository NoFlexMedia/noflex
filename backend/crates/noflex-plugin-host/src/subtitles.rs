// SPDX-License-Identifier: AGPL-3.0-only
//
// Online subtitle search over the `subtitle` hook. A search asks every
// subtitle plugin at once and keeps whatever each one found, so a provider
// that's down or out of quota hides nothing the others offer; downloading
// goes to the one provider that offered the subtitle.

use noflex_core::{
    PluginError,
    model::{ExternalId, ExternalIdKind, WorkKind},
    providers::{SubtitleFile, SubtitleMatch, SubtitleQuery},
    text::canonical_language,
};

use crate::{
    HookKind, HostError, PluginId, PluginRegistry,
    bindings::subtitle::{
        exports::noflex::plugin::subtitle as wit, noflex::plugin::types as wit_types,
    },
};

/// What a search across every subtitle plugin found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubtitleSearch {
    /// Best first: matches by the file's hash, then the most downloaded.
    pub matches: Vec<SubtitleMatch>,
    /// The providers that couldn't answer, and why.
    pub failures: Vec<ProviderFailure>,
}

/// A subtitle provider that failed a search or a download.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("subtitle provider {provider}: {error}")]
pub struct ProviderFailure {
    /// The plugin's id.
    pub provider: String,
    /// What it answered, or [`ProviderError::Unavailable`] when it couldn't be
    /// reached at all.
    pub error: ProviderError,
}

/// Why a subtitle provider couldn't answer: [`noflex_core::PluginError`], in
/// a form that can be cloned and compared.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// Out of requests or downloads until its quota resets.
    #[error("rate limited for {retry_after_secs}s")]
    RateLimited { retry_after_secs: u32 },
    /// Not set up (no API key), down, or busy.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// It no longer has what was asked for.
    #[error("not found")]
    NotFound,
    /// Anything else: a bad answer, a trap, a deadline overrun.
    #[error("{0}")]
    Failed(String),
}

impl From<PluginError> for ProviderError {
    fn from(error: PluginError) -> Self {
        match error {
            PluginError::RateLimited { retry_after_secs } => Self::RateLimited { retry_after_secs },
            PluginError::Unavailable(m) => Self::Unavailable(m),
            PluginError::NotFound => Self::NotFound,
            PluginError::BadInput(m) | PluginError::Internal(m) => Self::Failed(m),
        }
    }
}

impl From<wit_types::PluginError> for ProviderError {
    fn from(error: wit_types::PluginError) -> Self {
        use wit_types::PluginError as E;
        match error {
            E::RateLimited(retry_after_secs) => Self::RateLimited { retry_after_secs },
            E::Unavailable(m) => Self::Unavailable(m),
            E::NotFound => Self::NotFound,
            E::BadInput(m) | E::Internal(m) | E::Trapped(m) | E::CapabilityDenied(m) => {
                Self::Failed(m)
            }
        }
    }
}

impl From<HostError> for ProviderError {
    fn from(error: HostError) -> Self {
        match error {
            HostError::Overloaded { .. } => Self::Unavailable("busy".into()),
            other => Self::Failed(other.to_string()),
        }
    }
}

impl PluginRegistry {
    /// Ask every loaded subtitle plugin but those in `skip` for `query`, all
    /// at once. Matches come tagged with their provider, in the wanted
    /// languages only (any when `query.languages` is empty), spelled as
    /// [`canonical_language`] spells them, and ranked: hash matches first,
    /// then by downloads and the provider's score.
    pub async fn search_subtitles(&self, query: &SubtitleQuery, skip: &[String]) -> SubtitleSearch {
        let targets: Vec<_> = self
            .exporters_of(HookKind::Subtitle)
            .into_iter()
            .filter(|id| !skip.contains(&id.0))
            .filter_map(|id| self.get_plugin(&id).map(|plugin| (id, plugin)))
            .collect();
        let wanted: Vec<String> = query
            .languages
            .iter()
            .map(|l| canonical_language(l))
            .collect();
        let wit_query = to_wit_query(query, &wanted);
        let calls = targets.into_iter().map(|(id, plugin)| {
            let wit_query = wit_query.clone();
            async move { (id, plugin.call_subtitle_search(wit_query).await) }
        });
        let mut search = SubtitleSearch::default();
        for (id, outcome) in futures::future::join_all(calls).await {
            let error = match outcome {
                Ok(Ok(found)) => {
                    search.matches.extend(
                        found
                            .into_iter()
                            .map(|m| from_wit_match(&id, m))
                            .filter(|m| wanted.is_empty() || wanted.contains(&m.language)),
                    );
                    continue;
                }
                Ok(Err(e)) => ProviderError::from(e),
                Err(e) => ProviderError::from(e),
            };
            tracing::warn!(plugin = %id, %error, "subtitle search failed");
            search.failures.push(ProviderFailure {
                provider: id.0,
                error,
            });
        }
        rank(&mut search.matches);
        search
    }

    /// Download what `provider` offered as `handle`. A provider that isn't
    /// loaded or doesn't serve subtitles is [`ProviderError::NotFound`].
    pub async fn download_subtitle(
        &self,
        provider: &str,
        handle: &str,
    ) -> Result<SubtitleFile, ProviderFailure> {
        let fail = |error| ProviderFailure {
            provider: provider.to_string(),
            error,
        };
        let plugin = self
            .get_plugin(&PluginId(provider.to_string()))
            .filter(|plugin| plugin.exports(HookKind::Subtitle))
            .ok_or_else(|| fail(ProviderError::NotFound))?;
        match plugin.call_subtitle_download(handle.to_string()).await {
            Ok(Ok((format, bytes))) => Ok(SubtitleFile {
                format: format.to_ascii_lowercase(),
                bytes: bytes.into(),
            }),
            Ok(Err(e)) => Err(fail(e.into())),
            Err(e) => Err(fail(e.into())),
        }
    }
}

/// Hash matches first, as they're timed for the very file; then the most
/// downloaded and best rated. Ties keep a stable order by provider and handle.
fn rank(matches: &mut [SubtitleMatch]) {
    matches.sort_by(|a, b| {
        b.hash_match
            .cmp(&a.hash_match)
            .then_with(|| b.download_count.cmp(&a.download_count))
            .then_with(|| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)))
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.handle.cmp(&b.handle))
    });
}

fn to_wit_query(query: &SubtitleQuery, languages: &[String]) -> wit::SubtitleQuery {
    wit::SubtitleQuery {
        moviehash: query.moviehash.clone(),
        file_size: query.file_size,
        file_name: query.file_name.clone(),
        title: query.title.clone(),
        year: query.year,
        kind: to_wit_kind(query.kind),
        external_ids: to_wit_ids(&query.external_ids),
        series_external_ids: to_wit_ids(&query.series_external_ids),
        season: query.season,
        episode: query.episode,
        languages: languages.to_vec(),
    }
}

/// Scanner keys (`custom`) name files and folders; providers get the rest.
fn to_wit_ids(ids: &[ExternalId]) -> Vec<wit_types::ExternalId> {
    use wit_types::ExternalIdKind as W;
    ids.iter()
        .filter_map(|id| {
            let kind = match id.kind {
                ExternalIdKind::Tmdb => W::Tmdb,
                ExternalIdKind::Tvdb => W::Tvdb,
                ExternalIdKind::Imdb => W::Imdb,
                ExternalIdKind::MusicBrainzReleaseGroup => W::MusicBrainzReleaseGroup,
                ExternalIdKind::MusicBrainzArtist => W::MusicBrainzArtist,
                ExternalIdKind::MusicBrainzRecording => W::MusicBrainzRecording,
                ExternalIdKind::AniDb => W::AniDb,
                ExternalIdKind::Custom => return None,
            };
            Some(wit_types::ExternalId {
                kind,
                value: id.value.clone(),
            })
        })
        .collect()
}

fn to_wit_kind(kind: WorkKind) -> wit_types::WorkKind {
    use wit_types::WorkKind as W;
    match kind {
        WorkKind::Movie => W::Movie,
        WorkKind::Series => W::Series,
        WorkKind::Season => W::Season,
        WorkKind::Episode => W::Episode,
        WorkKind::Album => W::Album,
        WorkKind::Track => W::Track,
        WorkKind::Artist => W::Artist,
        WorkKind::AudioBook => W::AudioBook,
        WorkKind::Photo => W::Photo,
    }
}

fn from_wit_match(provider: &PluginId, m: wit::SubtitleMatch) -> SubtitleMatch {
    SubtitleMatch {
        provider: provider.0.clone(),
        handle: m.handle,
        language: canonical_language(&m.language),
        locale: m.locale,
        format: m.format.to_ascii_lowercase(),
        release: m.release.filter(|r| !r.trim().is_empty()),
        hearing_impaired: m.hearing_impaired,
        forced: m.forced,
        hash_match: m.hash_match,
        download_count: m.download_count,
        score: m.score.filter(|s| s.is_finite()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(provider: &str, handle: &str, hash: bool, downloads: Option<u64>) -> SubtitleMatch {
        SubtitleMatch {
            provider: provider.into(),
            handle: handle.into(),
            language: "eng".into(),
            locale: None,
            format: "srt".into(),
            release: None,
            hearing_impaired: false,
            forced: false,
            hash_match: hash,
            download_count: downloads,
            score: None,
        }
    }

    #[test]
    fn hash_matches_lead_then_downloads() {
        let mut matches = vec![
            offered("b", "popular", false, Some(9000)),
            offered("a", "unknown", false, None),
            offered("b", "hash-few", true, Some(3)),
            offered("a", "hash-many", true, Some(40)),
            offered("a", "some", false, Some(10)),
        ];
        rank(&mut matches);
        let order: Vec<_> = matches.iter().map(|m| m.handle.as_str()).collect();
        assert_eq!(
            order,
            ["hash-many", "hash-few", "popular", "some", "unknown"]
        );
    }

    #[test]
    fn scanner_keys_never_reach_providers() {
        let ids = to_wit_ids(&[
            ExternalId {
                kind: ExternalIdKind::Custom,
                value: "/media/Films/Alien (1979)".into(),
            },
            ExternalId {
                kind: ExternalIdKind::Imdb,
                value: "tt0078748".into(),
            },
        ]);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].value, "tt0078748");
    }
}
