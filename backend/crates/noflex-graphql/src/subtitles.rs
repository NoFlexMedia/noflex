// SPDX-License-Identifier: AGPL-3.0-only
//
// Online subtitle search for a media file. Anyone who can play a title can
// search for its subtitles and download one, which everyone who can see the
// title then shares as one of its tracks; whoever downloaded it, or an
// administrator, can remove it. The work happens in
// `noflex_plugin_host::SubtitleService`.

use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions, ID, Object, SimpleObject};
use noflex_core::{DownloadedSubtitleId, MediaFileId, UserId, providers::SubtitleMatch};
use noflex_plugin_host::{
    PluginRegistry, ProviderError, ProviderFailure, SubtitleError, SubtitleService,
};
use noflex_storage::{
    Storage,
    domain::{DownloadedSubtitle, MediaFile, SubtitleTrack},
};

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn service<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<SubtitleService>> {
    ctx.data::<Arc<SubtitleService>>()
        .map_err(|_| async_graphql::Error::new("subtitle search not configured"))
}

/// A provider's name as its manifest gives it, else its id.
fn provider_name(ctx: &Context<'_>, provider: &str) -> String {
    ctx.data::<Arc<PluginRegistry>>()
        .ok()
        .and_then(|registry| registry.get(&noflex_plugin_host::PluginId(provider.into())))
        .map_or_else(|| provider.to_string(), |plugin| plugin.display_name)
}

fn parse_id<T>(id: &ID, from: impl Fn(uuid::Uuid) -> T) -> async_graphql::Result<T> {
    id.parse::<uuid::Uuid>()
        .map(from)
        .map_err(|_| async_graphql::Error::new("invalid id"))
}

/// The media file, when the caller can play it.
async fn playable_media(
    ctx: &Context<'_>,
    media: &ID,
) -> async_graphql::Result<(MediaFile, UserId)> {
    let storage = storage(ctx)?;
    let user = crate::access::request_user(ctx)?;
    let id = parse_id(media, MediaFileId::from_uuid)?;
    let not_found = || async_graphql::Error::new("media file not found");
    if !crate::access::can_media(storage, user, id).await? {
        return Err(not_found());
    }
    let media = storage
        .media_file_by_id(id)
        .await
        .map_err(|e| {
            tracing::error!(media = %id, error = %e, "subtitles: media file lookup failed");
            async_graphql::Error::new("storage error")
        })?
        .ok_or_else(not_found)?;
    Ok((media, user.user_id))
}

/// "In about 3 hours" for a wait in seconds.
fn wait_for(seconds: u32) -> String {
    match seconds {
        0..=89 => "in a minute or so".into(),
        90..=5399 => format!("in about {} minutes", seconds.div_ceil(60)),
        _ => format!("in about {} hours", seconds.div_ceil(3600)),
    }
}

/// What a person reads when a provider fails them.
fn readable(name: &str, error: &ProviderError) -> String {
    match error {
        ProviderError::RateLimited { retry_after_secs } => format!(
            "{name} won't take more requests for now. Try again {}.",
            wait_for(*retry_after_secs)
        ),
        ProviderError::Unavailable(why) => format!("{name} is unavailable: {why}"),
        ProviderError::NotFound => format!("{name} no longer has that subtitle."),
        ProviderError::Failed(_) => format!("{name} couldn't answer."),
    }
}

fn provider_error(ctx: &Context<'_>, failure: &ProviderFailure) -> async_graphql::Error {
    let name = provider_name(ctx, &failure.provider);
    let code = match failure.error {
        ProviderError::RateLimited { .. } => "RATE_LIMITED",
        ProviderError::Unavailable(_) => "PROVIDER_UNAVAILABLE",
        ProviderError::NotFound => "NOT_FOUND",
        ProviderError::Failed(_) => "PROVIDER_FAILED",
    };
    let retry = match failure.error {
        ProviderError::RateLimited { retry_after_secs } => Some(retry_after_secs),
        _ => None,
    };
    async_graphql::Error::new(readable(&name, &failure.error)).extend_with(|_, e| {
        e.set("code", code);
        if let Some(retry) = retry {
            e.set("retryAfterSeconds", retry);
        }
    })
}

fn subtitle_error(ctx: &Context<'_>, error: SubtitleError) -> async_graphql::Error {
    let coded = |message: &str, code: &str| {
        let code = code.to_string();
        async_graphql::Error::new(message).extend_with(move |_, e| e.set("code", code.clone()))
    };
    match error {
        SubtitleError::NotFound => coded("media file not found", "NOT_FOUND"),
        SubtitleError::NotOffered => coded(
            "That subtitle is no longer on offer. Search again.",
            "NOT_OFFERED",
        ),
        SubtitleError::Provider(failure) => provider_error(ctx, &failure),
        SubtitleError::UnsupportedFormat(format) => coded(
            &format!("That subtitle is in a format NoFlex can't play ({format})."),
            "UNSUPPORTED_FORMAT",
        ),
        SubtitleError::Empty(_) => coded("That subtitle file has nothing in it.", "EMPTY"),
        other => {
            tracing::error!(error = %other, "subtitles: failed");
            async_graphql::Error::new("subtitle storage error")
        }
    }
}

/// One subtitle a provider offers for a media file.
#[derive(SimpleObject, Debug, Clone)]
pub struct SubtitleOffer {
    /// The plugin offering it; pass it to `downloadSubtitle`.
    pub provider: String,
    pub provider_name: String,
    /// Pass it to `downloadSubtitle`.
    pub handle: String,
    /// ISO 639-2/T.
    pub language: String,
    /// BCP 47 where the provider tells regions apart (`pt-BR`).
    pub locale: Option<String>,
    /// `srt`, `vtt`, `ass` or `ssa`.
    pub format: String,
    /// The release it was timed for.
    pub release: Option<String>,
    pub hearing_impaired: bool,
    pub forced: bool,
    /// Found by the file's hash, so timed for this exact file.
    pub hash_match: bool,
    pub download_count: Option<u64>,
    /// The provider's own rating; higher is better.
    pub score: Option<f32>,
}

impl SubtitleOffer {
    fn new(ctx: &Context<'_>, offer: SubtitleMatch) -> Self {
        Self {
            provider_name: provider_name(ctx, &offer.provider),
            provider: offer.provider,
            handle: offer.handle,
            language: offer.language,
            locale: offer.locale,
            format: offer.format,
            release: offer.release,
            hearing_impaired: offer.hearing_impaired,
            forced: offer.forced,
            hash_match: offer.hash_match,
            download_count: offer.download_count,
            score: offer.score,
        }
    }
}

/// A provider that couldn't answer a search.
#[derive(SimpleObject, Debug, Clone)]
pub struct SubtitleProviderFailure {
    pub provider: String,
    pub provider_name: String,
    /// What happened, to show as it is.
    pub message: String,
    /// When it's out of quota, the seconds until it resets.
    pub retry_after_seconds: Option<u32>,
}

/// What a subtitle search found.
#[derive(SimpleObject, Debug, Clone)]
pub struct SubtitleSearchResult {
    /// Best first: matches by the file's hash, then the most downloaded.
    pub offers: Vec<SubtitleOffer>,
    /// The providers that couldn't answer. The others' offers stand.
    pub failures: Vec<SubtitleProviderFailure>,
}

/// A downloaded subtitle as the media file now lists it.
#[derive(SimpleObject, Debug, Clone)]
pub struct DownloadedSubtitleTrack {
    /// Its position in `mediaFile.subtitleTracks`, the index `burnSubtitle`
    /// and `subtitles` use.
    pub index: u32,
    pub track: SubtitleTrack,
    /// With its tracks as they now are.
    pub media_file: MediaFile,
}

#[derive(Default)]
pub struct SubtitlesQuery;

#[Object]
impl SubtitlesQuery {
    /// Search every subtitle plugin for a media file's subtitles in
    /// `languages` (ISO 639 codes), or in the caller's preferred subtitle
    /// languages when it's left out; any language when there are none.
    /// What's found can be downloaded for the next hour. A provider that
    /// fails is in `failures` and hides nothing the others found.
    async fn search_subtitles(
        &self,
        ctx: &Context<'_>,
        media_file_id: ID,
        languages: Option<Vec<String>>,
    ) -> async_graphql::Result<SubtitleSearchResult> {
        let (media, user) = playable_media(ctx, &media_file_id).await?;
        let languages = match languages {
            Some(languages) => languages,
            None => preferred_subtitle_languages(storage(ctx)?, user).await,
        };
        let search = service(ctx)?
            .search(&media, &languages)
            .await
            .map_err(|e| subtitle_error(ctx, e))?;
        Ok(SubtitleSearchResult {
            offers: search
                .matches
                .into_iter()
                .map(|offer| SubtitleOffer::new(ctx, offer))
                .collect(),
            failures: search
                .failures
                .into_iter()
                .map(|failure| {
                    let provider_name = provider_name(ctx, &failure.provider);
                    SubtitleProviderFailure {
                        message: readable(&provider_name, &failure.error),
                        retry_after_seconds: match failure.error {
                            ProviderError::RateLimited { retry_after_secs } => {
                                Some(retry_after_secs)
                            }
                            _ => None,
                        },
                        provider: failure.provider,
                        provider_name,
                    }
                })
                .collect(),
        })
    }
}

/// The languages a user's playback preference names for subtitles, as the
/// web's Account → Profile writes them (`playback.subtitles.languages`).
async fn preferred_subtitle_languages(storage: &Arc<dyn Storage>, user: UserId) -> Vec<String> {
    let preferences = match storage.list_user_preferences(user).await {
        Ok(preferences) => preferences,
        Err(error) => {
            tracing::warn!(%user, %error, "subtitle search: preferences unreadable");
            return Vec::new();
        }
    };
    preferences
        .into_iter()
        .find(|p| p.key == "playback")
        .and_then(|p| {
            p.value.0["subtitles"]["languages"]
                .as_array()
                .map(|languages| {
                    languages
                        .iter()
                        .filter_map(|l| l.as_str().map(str::to_string))
                        .collect()
                })
        })
        .unwrap_or_default()
}

#[derive(Default)]
pub struct SubtitlesMutation;

#[Object]
impl SubtitlesMutation {
    /// Download what `provider` offered as `handle` in a recent
    /// `searchSubtitles` for this media file. It becomes one of the file's
    /// tracks for everyone who can see the title, kept in the server's own
    /// cache, never in the media folder. Downloading one the file already
    /// has returns it. A provider out of quota fails with code
    /// `RATE_LIMITED` and `retryAfterSeconds`; one no longer on offer with
    /// `NOT_OFFERED`.
    async fn download_subtitle(
        &self,
        ctx: &Context<'_>,
        media_file_id: ID,
        provider: String,
        handle: String,
    ) -> async_graphql::Result<DownloadedSubtitleTrack> {
        let (media, user) = playable_media(ctx, &media_file_id).await?;
        let kept = service(ctx)?
            .download(&media, &provider, &handle, Some(user))
            .await
            .map_err(|e| subtitle_error(ctx, e))?;
        let downloaded = storage(ctx)?
            .downloaded_subtitles_for_media(media.id)
            .await
            .map_err(|e| {
                tracing::error!(media = %media.id, error = %e, "subtitles: listing failed");
                async_graphql::Error::new("storage error")
            })?;
        let position = downloaded
            .iter()
            .position(|s| s.id == kept.id)
            .ok_or_else(|| async_graphql::Error::new("subtitle not found"))?;
        let index = u32::try_from(media.subtitle_tracks.len() + position)
            .map_err(|_| async_graphql::Error::new("too many subtitle tracks"))?;
        Ok(DownloadedSubtitleTrack {
            index,
            track: SubtitleTrack::from(&kept),
            media_file: media,
        })
    }

    /// Remove a downloaded subtitle from its media file, for everyone.
    /// Whoever downloaded it or an administrator may. Returns what was
    /// removed, or null when there was nothing to remove.
    async fn delete_downloaded_subtitle(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> async_graphql::Result<Option<DownloadedSubtitle>> {
        let storage = storage(ctx)?;
        let user = crate::access::request_user(ctx)?;
        let id = parse_id(&id, DownloadedSubtitleId::from_uuid)?;
        let Some(subtitle) = storage.get_downloaded_subtitle(id).await.map_err(|e| {
            tracing::error!(subtitle = %id, error = %e, "subtitles: lookup failed");
            async_graphql::Error::new("storage error")
        })?
        else {
            return Ok(None);
        };
        let visible = crate::access::can_media(storage, user, subtitle.media_file_id).await?;
        if !visible {
            return Ok(None);
        }
        if !user.is_admin && subtitle.downloaded_by != Some(user.user_id) {
            return Err(async_graphql::Error::new(
                "only whoever downloaded a subtitle, or an administrator, can remove it",
            )
            .extend_with(|_, e| e.set("code", "FORBIDDEN")));
        }
        service(ctx)?
            .delete(id)
            .await
            .map_err(|e| subtitle_error(ctx, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_read_as_people_say_them() {
        assert_eq!(wait_for(20), "in a minute or so");
        assert_eq!(wait_for(600), "in about 10 minutes");
        assert_eq!(wait_for(3 * 3600 - 60), "in about 3 hours");
        assert_eq!(
            readable(
                "OpenSubtitles",
                &ProviderError::RateLimited {
                    retry_after_secs: 7200
                }
            ),
            "OpenSubtitles won't take more requests for now. Try again in about 2 hours."
        );
    }
}
