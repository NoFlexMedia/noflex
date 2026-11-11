// SPDX-License-Identifier: AGPL-3.0-only
//
// Online subtitle search over the `subtitle` hook. A search asks every
// subtitle plugin at once and keeps whatever each one found, so a provider
// that's down or out of quota hides nothing the others offer; downloading
// goes to the one provider that offered the subtitle. `SubtitleService`
// keeps what viewers download in the server's own cache, shared by everyone
// who can see the title, until nobody has used it for a while.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use noflex_core::{
    DownloadedSubtitleId, MediaFileId, PluginError, SubtitleCodec, UserId,
    model::{ExternalId, ExternalIdKind, WorkKind},
    providers::{SubtitleFile, SubtitleMatch, SubtitleQuery},
    text::canonical_language,
};
use noflex_storage::{
    Storage,
    domain::{DownloadedSubtitle, MediaFile},
};
use noflex_transcoder::subtitles;
use time::OffsetDateTime;

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

/// How long a search's matches can be downloaded by handle. A viewer picks
/// from what they just searched; past this they search again.
const OFFER_TTL: Duration = Duration::from_secs(60 * 60);

/// Most matches remembered at once, across every search.
const MAX_OFFERS: usize = 10_000;

/// A file in the cache with no record is left this long before the sweep
/// takes it, so a download being written isn't.
const ORPHAN_GRACE: Duration = Duration::from_secs(10 * 60);

/// OpenSubtitles' moviehash reads this much from each end of the file.
const HASH_CHUNK: u64 = 64 * 1024;

/// Why a subtitle couldn't be found, fetched, kept or removed.
#[derive(Debug, thiserror::Error)]
pub enum SubtitleError {
    /// No such media file or downloaded subtitle.
    #[error("not found")]
    NotFound,
    /// The match wasn't among a recent search's; search again.
    #[error("that subtitle is no longer on offer; search again")]
    NotOffered,
    #[error(transparent)]
    Provider(#[from] ProviderFailure),
    /// The provider sent a format the subtitle pipeline can't read.
    #[error("unsupported subtitle format `{0}`")]
    UnsupportedFormat(String),
    /// The provider's file has no cues in it.
    #[error("the provider's {0} file holds no subtitles")]
    Empty(&'static str),
    #[error("storage: {0}")]
    Storage(#[from] noflex_storage::StorageError),
    #[error("subtitle cache: {0}")]
    Io(#[from] std::io::Error),
}

/// What recent searches offered, by media file, provider and handle, with
/// when they offered it.
type Offers = HashMap<(MediaFileId, String, String), (Instant, SubtitleMatch)>;

/// Searching, downloading and keeping subtitles for media files. Downloads
/// are files in `dir`, the server's own subtitle cache, named for their
/// record; nothing is ever written beside the media.
pub struct SubtitleService {
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    dir: PathBuf,
    disabled: Vec<String>,
    offers: Mutex<Offers>,
}

impl SubtitleService {
    /// A service keeping downloads in `dir`, which it creates when it first
    /// writes one.
    pub fn new(registry: Arc<PluginRegistry>, storage: Arc<dyn Storage>, dir: PathBuf) -> Self {
        Self {
            registry,
            storage,
            dir,
            disabled: Vec::new(),
            offers: Mutex::new(HashMap::new()),
        }
    }

    /// Leave these plugins out of every search, as the server config's
    /// `disabled` overrides ask.
    pub fn with_disabled(mut self, disabled: Vec<String>) -> Self {
        self.disabled = disabled;
        self
    }

    /// Where `subtitle`'s text is.
    pub fn path(&self, subtitle: &DownloadedSubtitle) -> PathBuf {
        self.dir.join(&subtitle.file_name)
    }

    /// Search every subtitle plugin for `media`'s subtitles in `languages`
    /// (ISO 639 in any spelling; any language when empty). What's found can
    /// be downloaded for the next hour.
    ///
    /// # Errors
    ///
    /// [`SubtitleError::NotFound`] when the media file has no work, and
    /// storage errors. A provider failing isn't one: it's in
    /// [`SubtitleSearch::failures`].
    pub async fn search(
        &self,
        media: &MediaFile,
        languages: &[String],
    ) -> Result<SubtitleSearch, SubtitleError> {
        let query = self.query_for(media, languages).await?;
        let search = self.registry.search_subtitles(&query, &self.disabled).await;
        let now = Instant::now();
        let mut offers = self.offers.lock().unwrap_or_else(PoisonError::into_inner);
        offers.retain(|_, (at, _)| now.duration_since(*at) < OFFER_TTL);
        for found in &search.matches {
            if offers.len() >= MAX_OFFERS {
                break;
            }
            offers.insert(
                (media.id, found.provider.clone(), found.handle.clone()),
                (now, found.clone()),
            );
        }
        Ok(search)
    }

    /// Fetch what `provider` offered as `handle` in a recent search for
    /// `media`, and keep it for everyone who can see the title. Fetching one
    /// it already has returns that one without asking the provider again.
    ///
    /// # Errors
    ///
    /// [`SubtitleError::NotOffered`] for a match no recent search found,
    /// the provider's failure, a format or file the pipeline can't use, and
    /// storage or cache errors. Nothing is kept on an error.
    pub async fn download(
        &self,
        media: &MediaFile,
        provider: &str,
        handle: &str,
        user: Option<UserId>,
    ) -> Result<DownloadedSubtitle, SubtitleError> {
        let kept = self
            .storage
            .downloaded_subtitles_for_media(media.id)
            .await?;
        if let Some(existing) = kept
            .into_iter()
            .find(|s| s.provider == provider && s.handle == handle)
        {
            return Ok(existing);
        }
        let offer = {
            let offers = self.offers.lock().unwrap_or_else(PoisonError::into_inner);
            offers
                .get(&(media.id, provider.to_string(), handle.to_string()))
                .filter(|(at, _)| at.elapsed() < OFFER_TTL)
                .map(|(_, offer)| offer.clone())
        }
        .ok_or(SubtitleError::NotOffered)?;
        let file = self.registry.download_subtitle(provider, handle).await?;
        let (format, text) = normalize(&file.format, &file.bytes)?;

        let id = DownloadedSubtitleId::new();
        let file_name = format!("{id}.{}", extension(format));
        tokio::fs::create_dir_all(&self.dir).await?;
        let path = self.dir.join(&file_name);
        let partial = self.dir.join(format!("{file_name}.part"));
        tokio::fs::write(&partial, text.as_bytes()).await?;
        tokio::fs::rename(&partial, &path).await?;

        let now = OffsetDateTime::now_utc();
        let record = DownloadedSubtitle {
            id,
            media_file_id: media.id,
            provider: offer.provider,
            handle: offer.handle,
            language: offer.language,
            format,
            release: offer.release,
            hearing_impaired: offer.hearing_impaired,
            forced: offer.forced,
            hash_match: offer.hash_match,
            file_name,
            downloaded_by: user,
            created_at: now,
            last_used_at: now,
        };
        match self.storage.insert_downloaded_subtitle(&record).await {
            Ok(stored) => {
                // Someone else's download of the same one landed first.
                if stored.id != id {
                    remove_quietly(&path).await;
                }
                Ok(stored)
            }
            Err(error) => {
                remove_quietly(&path).await;
                Err(match error {
                    noflex_storage::StorageError::NotFound => SubtitleError::NotFound,
                    other => other.into(),
                })
            }
        }
    }

    /// Remove a downloaded subtitle and its file. Returns what was removed.
    ///
    /// # Errors
    ///
    /// Storage errors; a file already gone is none.
    pub async fn delete(
        &self,
        id: DownloadedSubtitleId,
    ) -> Result<Option<DownloadedSubtitle>, SubtitleError> {
        let removed = self.storage.delete_downloaded_subtitle(id).await?;
        if let Some(subtitle) = &removed {
            remove_quietly(&self.path(subtitle)).await;
        }
        Ok(removed)
    }

    /// Note that `subtitle` was just served or burned in, which keeps the
    /// sweep from it. Failing to note it costs nothing but that, so it's
    /// logged, not returned.
    pub async fn record_use(&self, subtitle: &DownloadedSubtitle) {
        if let Err(error) = self
            .storage
            .touch_downloaded_subtitle(subtitle.id, OffsetDateTime::now_utc())
            .await
        {
            tracing::warn!(subtitle = %subtitle.id, %error, "downloaded subtitle: use not recorded");
        }
    }

    /// Remove every downloaded subtitle nobody has used for `unused_for`,
    /// and any file in the cache left without a record (its media file went,
    /// or a download died half way). Returns how many subtitles went.
    ///
    /// # Errors
    ///
    /// Storage errors, and failing to read the cache directory.
    pub async fn sweep(&self, unused_for: Duration) -> Result<usize, SubtitleError> {
        let cutoff = OffsetDateTime::now_utc() - unused_for;
        let unused = self
            .storage
            .delete_downloaded_subtitles_unused_since(cutoff)
            .await?;
        for subtitle in &unused {
            remove_quietly(&self.path(subtitle)).await;
        }
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(unused.len()),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let fresh = entry
                .metadata()
                .await
                .and_then(|m| m.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_none_or(|age| age < ORPHAN_GRACE);
            if fresh {
                continue;
            }
            let id = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.split('.').next())
                .and_then(|stem| stem.parse::<uuid::Uuid>().ok())
                .map(DownloadedSubtitleId::from_uuid);
            let kept = match id {
                Some(id) if path.extension().is_some_and(|ext| ext != "part") => {
                    self.storage.get_downloaded_subtitle(id).await?.is_some()
                }
                _ => false,
            };
            if !kept {
                remove_quietly(&path).await;
            }
        }
        Ok(unused.len())
    }

    /// Everything the host knows about `media` that a provider might match
    /// on: the file's moviehash, name and size, its work's title, year and
    /// ids, and for an episode its series' ids and numbers.
    async fn query_for(
        &self,
        media: &MediaFile,
        languages: &[String],
    ) -> Result<SubtitleQuery, SubtitleError> {
        let work = self
            .storage
            .work_for_media_file(media.id)
            .await?
            .ok_or(SubtitleError::NotFound)?;
        let path = Path::new(&media.path);
        let (file_size, moviehash) = match moviehash(path).await {
            Ok(hashed) => hashed,
            Err(error) => {
                tracing::warn!(media = %media.id, %error, "subtitle search: file not hashed");
                (media.size_bytes, None)
            }
        };
        let mut query = SubtitleQuery {
            moviehash,
            file_size,
            file_name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            title: work.title.clone(),
            year: work.year,
            kind: work.kind,
            external_ids: work.external_ids.clone(),
            series_external_ids: Vec::new(),
            season: None,
            episode: None,
            languages: languages.iter().map(|l| canonical_language(l)).collect(),
        };
        if work.kind == WorkKind::Episode {
            query.episode = work.index_number.and_then(|n| u32::try_from(n).ok());
            let season = match work.parent_id {
                Some(id) => self.storage.get_work(id).await?,
                None => None,
            };
            query.season = work
                .parent_index
                .or_else(|| season.as_ref().and_then(|s| s.index_number))
                .and_then(|n| u32::try_from(n).ok());
            let series = match season.as_ref().and_then(|s| s.parent_id) {
                Some(id) => self.storage.get_work(id).await?,
                None => None,
            };
            if let Some(series) = series {
                query.series_external_ids = series.external_ids;
                query.year = query.year.or(series.year);
            }
        }
        Ok(query)
    }
}

/// Keep downloaded subtitles from outliving their use: sweep at start and
/// every six hours, removing those unused for `unused_for`.
pub fn spawn_subtitle_sweeper(
    service: Arc<SubtitleService>,
    unused_for: Duration,
) -> tokio::task::JoinHandle<()> {
    const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
    tokio::spawn(async move {
        loop {
            match service.sweep(unused_for).await {
                Ok(0) => {}
                Ok(removed) => tracing::info!(removed, "swept unused downloaded subtitles"),
                Err(error) => tracing::warn!(%error, "downloaded subtitle sweep failed"),
            }
            tokio::time::sleep(EVERY).await;
        }
    })
}

/// The file's size, and OpenSubtitles' moviehash of it: the size plus the
/// wrapping sum of the first and last 64 KiB read as little-endian `u64`s,
/// as 16 lowercase hex digits. No hash for a file under 64 KiB.
///
/// # Errors
///
/// Failing to open or read the file.
pub async fn moviehash(path: &Path) -> std::io::Result<(u64, Option<String>)> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();
    if size < HASH_CHUNK {
        return Ok((size, None));
    }
    let mut hash = size;
    let mut chunk = vec![0_u8; HASH_CHUNK as usize];
    for offset in [0, size - HASH_CHUNK] {
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        file.read_exact(&mut chunk).await?;
        for word in chunk.as_chunks::<8>().0 {
            hash = hash.wrapping_add(u64::from_le_bytes(*word));
        }
    }
    Ok((size, Some(format!("{hash:016x}"))))
}

/// A provider's subtitle as the pipeline keeps it: one of the text formats
/// it converts and burns in, decoded to UTF-8 with `\n` line ends.
///
/// # Errors
///
/// [`SubtitleError::UnsupportedFormat`] for anything but SubRip, WebVTT,
/// ASS and SSA, and [`SubtitleError::Empty`] for a file without a cue.
fn normalize(format: &str, bytes: &[u8]) -> Result<(SubtitleCodec, String), SubtitleError> {
    let codec = match format {
        "srt" | "subrip" => SubtitleCodec::SubRip,
        "vtt" | "webvtt" => SubtitleCodec::WebVtt,
        "ass" => SubtitleCodec::Ass,
        "ssa" => SubtitleCodec::Ssa,
        other => return Err(SubtitleError::UnsupportedFormat(other.to_string())),
    };
    let text = subtitles::decode_text(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if !subtitles::to_webvtt(codec, &text).is_some_and(|vtt| vtt.contains("-->")) {
        return Err(SubtitleError::Empty(extension(codec)));
    }
    Ok((codec, text))
}

/// The extension ffmpeg recognizes the format by when burning it in.
fn extension(codec: SubtitleCodec) -> &'static str {
    match codec {
        SubtitleCodec::WebVtt => "vtt",
        SubtitleCodec::Ass => "ass",
        SubtitleCodec::Ssa => "ssa",
        _ => "srt",
    }
}

async fn remove_quietly(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "subtitle cache: file not removed");
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

    async fn hash_of(bytes: &[u8]) -> (u64, Option<String>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("film.mkv");
        std::fs::write(&path, bytes).expect("write");
        moviehash(&path).await.expect("hash")
    }

    #[tokio::test]
    async fn moviehash_sums_both_ends_little_endian_onto_the_size() {
        let size = 2 * HASH_CHUNK as usize;
        let mut bytes = vec![0_u8; size];
        bytes[0] = 1; // 1 as the first little-endian word
        bytes[size - 8 + 1] = 1; // 256 as the last
        assert_eq!(
            hash_of(&bytes).await,
            (size as u64, Some(format!("{:016x}", size as u64 + 1 + 256)))
        );

        // The ends overlap in a file under 128 KiB; each is still read whole.
        let mut short = vec![0_u8; HASH_CHUNK as usize + 8];
        short[8] = 2;
        assert_eq!(
            hash_of(&short).await.1,
            Some(format!("{:016x}", short.len() as u64 + 2 + 2))
        );

        // 2 × 8192 words of u64::MAX wrap round to minus 16384.
        let wraps = vec![0xff_u8; HASH_CHUNK as usize];
        assert_eq!(hash_of(&wraps).await.1.as_deref(), Some("000000000000c000"));

        assert_eq!(hash_of(b"tiny").await, (4, None), "too small to hash");
    }

    #[test]
    fn normalize_decodes_to_utf8_and_keeps_only_readable_formats() {
        let (codec, text) =
            normalize("srt", b"1\r\n00:00:01,000 --> 00:00:02,000\r\nCaf\xe9\r\n").expect("SubRip");
        assert_eq!(codec, SubtitleCodec::SubRip);
        assert_eq!(text, "1\n00:00:01,000 --> 00:00:02,000\nCafé\n");

        let (codec, _) = normalize("vtt", b"WEBVTT\n\n00:01.000 --> 00:02.000\nHi\n").expect("vtt");
        assert_eq!(codec, SubtitleCodec::WebVtt);

        assert!(matches!(
            normalize("sub", b"{1}{25}Hi"),
            Err(SubtitleError::UnsupportedFormat(f)) if f == "sub"
        ));
        assert!(matches!(
            normalize("srt", b"<html>quota exceeded</html>"),
            Err(SubtitleError::Empty("srt"))
        ));
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
