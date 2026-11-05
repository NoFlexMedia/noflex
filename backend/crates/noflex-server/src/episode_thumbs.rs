// SPDX-License-Identifier: AGPL-3.0-only
//
// Fallback thumbnails for episodes the scanner found no local `Thumb`
// for: one frame grabbed ~20% into the runtime, written under the image
// cache as a `Generated`-source row. Runs as a background sweep (after
// each scan and hourly) so it never slows a scan. Episodes holding a
// thumb from any source are skipped; a local or provider thumb found
// later outranks the generated one, which the scanner never deletes.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use noflex_core::{DomainEvent, DomainEventBus, ImageKind, WorkId, WorkKind};
use noflex_storage::{
    Storage,
    domain::{Image, ImageSource, ImageTarget, MediaFile, Work},
};
use tokio::{
    sync::{broadcast::error::RecvError, mpsc},
    task::JoinHandle,
    time::{Duration, Instant, interval_at},
};

use crate::subprocess::{CommandError, command_output};

const SWEEP_PAGE: u32 = 32;
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// A single-frame grab after an input seek decodes one GOP at most.
const GRAB_TIMEOUT: Duration = Duration::from_secs(2 * 60);
/// The listing has no failure state, so failures are remembered in
/// memory to avoid re-running ffmpeg on them every sweep.
const FAILURE_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60);
const THUMB_WIDTH: u32 = 640;
/// Directory under the image cache root holding `<work id>.jpg`.
pub const THUMB_DIR: &str = "episode-thumbs";

#[derive(Debug, thiserror::Error)]
pub enum EpisodeThumbError {
    #[error("storage: {0}")]
    Storage(#[from] noflex_storage::StorageError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("thumbnails are only generated for episodes")]
    NotEpisode,
    #[error("episode has no media file")]
    NoMedia,
    #[error("media has no usable duration")]
    MissingDuration,
    #[error(transparent)]
    Command(#[from] CommandError),
}

#[derive(Clone)]
pub struct EpisodeThumbWorker {
    storage: Arc<dyn Storage>,
    ffmpeg_path: PathBuf,
    cache_root: PathBuf,
    backoff: Arc<std::sync::Mutex<HashMap<WorkId, Instant>>>,
}

impl EpisodeThumbWorker {
    /// `cache_root` is the image cache dir the image route serves from.
    pub fn new(storage: Arc<dyn Storage>, ffmpeg_path: PathBuf, cache_root: PathBuf) -> Self {
        Self {
            storage,
            ffmpeg_path,
            cache_root,
            backoff: Arc::default(),
        }
    }

    /// Sweep once now, then after every `ScanCompleted` and hourly.
    pub fn spawn(self: Arc<Self>, events: &DomainEventBus) -> JoinHandle<()> {
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(forward_events(events.clone(), tx.clone()));
        tokio::spawn(self.run(rx, tx))
    }

    async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<()>, _keepalive: mpsc::Sender<()>) {
        let mut tick = interval_at(Instant::now() + SWEEP_INTERVAL, SWEEP_INTERVAL);
        self.run_sweep().await;
        loop {
            tokio::select! {
                message = rx.recv() => match message {
                    Some(()) => self.run_sweep().await,
                    None => break,
                },
                _ = tick.tick() => self.run_sweep().await,
            }
        }
    }

    /// Generate a thumb for every episode still lacking one. Episodes
    /// that failed within the last day are skipped.
    pub async fn run_sweep(&self) {
        let mut cursor = None;
        loop {
            let page = match self
                .storage
                .works_missing_image(
                    WorkKind::Episode,
                    ImageKind::Thumb,
                    ImageSource::ALL,
                    SWEEP_PAGE,
                    cursor,
                )
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "episode thumb sweep listing failed");
                    return;
                }
            };
            for work in &page.items {
                if self.backed_off(work.id) {
                    continue;
                }
                if let Err(error) = self.generate(work).await {
                    tracing::warn!(work = %work.id, %error, "episode thumb generation failed");
                    self.park(work.id);
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return,
            }
        }
    }

    fn backed_off(&self, work: WorkId) -> bool {
        let Ok(mut backoff) = self.backoff.lock() else {
            return false;
        };
        match backoff.get(&work) {
            Some(until) if *until > Instant::now() => true,
            Some(_) => {
                backoff.remove(&work);
                false
            }
            None => false,
        }
    }

    fn park(&self, work: WorkId) {
        if let Ok(mut backoff) = self.backoff.lock() {
            backoff.insert(work, Instant::now() + FAILURE_BACKOFF);
        }
    }

    /// Grab a frame of `episode`'s first media part into the image cache
    /// and record it as the episode's `Thumb`. Returns `None` without
    /// writing a row when a `Thumb` appeared meanwhile (e.g. a scan found
    /// a local one).
    ///
    /// # Errors
    /// [`EpisodeThumbError::NotEpisode`] for other work kinds;
    /// [`EpisodeThumbError::MissingDuration`] when neither the edition
    /// nor the stored probe knows the runtime; ffmpeg failures otherwise.
    /// No partial file is left behind on error.
    pub async fn generate(&self, episode: &Work) -> Result<Option<Image>, EpisodeThumbError> {
        if episode.kind != WorkKind::Episode {
            return Err(EpisodeThumbError::NotEpisode);
        }
        let (media, duration_ms) = self.source(episode.id).await?;

        let dir = self.cache_root.join(THUMB_DIR);
        tokio::fs::create_dir_all(&dir).await?;
        let dest = dir.join(format!("{}.jpg", episode.id));
        // Unique temp sibling + rename: concurrent sweeps never publish a
        // partial file, and a failed grab never clobbers a previous one.
        let tmp = dir.join(format!(".{}.{}.jpg", episode.id, uuid::Uuid::now_v7()));
        let grabbed = self.grab(&media.path, seek_ms(duration_ms), &tmp).await;
        if let Err(error) = grabbed {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }

        let target = ImageTarget::Work(episode.id);
        if self
            .storage
            .image(target, ImageKind::Thumb, ImageSource::ALL)
            .await?
            .is_some()
        {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Ok(None);
        }
        if let Err(error) = tokio::fs::rename(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error.into());
        }
        let image = Image {
            target,
            kind: ImageKind::Thumb,
            source: ImageSource::Generated,
            path: dest.to_string_lossy().into_owned(),
            width: None,
            height: None,
            language: None,
            provider: None,
        };
        self.storage.upsert_image(&image).await?;
        Ok(Some(image))
    }

    /// The episode's first media part and its runtime.
    async fn source(&self, episode: WorkId) -> Result<(MediaFile, u64), EpisodeThumbError> {
        let mut editions = self.storage.editions_for_work(episode).await?;
        editions.sort_by_key(|e| e.id.into_uuid());
        for edition in editions {
            let Some(media) = self
                .storage
                .media_files_for_edition(edition.id)
                .await?
                .into_iter()
                .min_by_key(|m| m.part_index)
            else {
                continue;
            };
            let duration_ms = edition
                .runtime_ms
                .and_then(|ms| u64::try_from(ms).ok())
                .or_else(|| {
                    media
                        .probe
                        .0
                        .pointer("/duration_ms")
                        .and_then(|v| v.as_u64())
                })
                .filter(|ms| *ms > 0)
                .ok_or(EpisodeThumbError::MissingDuration)?;
            return Ok((media, duration_ms));
        }
        Err(EpisodeThumbError::NoMedia)
    }

    async fn grab(&self, source: &str, at_ms: u64, out: &Path) -> Result<(), EpisodeThumbError> {
        let args = vec![
            "-hide_banner".to_string(),
            "-loglevel".to_string(),
            "error".to_string(),
            "-ss".to_string(),
            format!("{}.{:03}", at_ms / 1000, at_ms % 1000),
            "-i".to_string(),
            source.to_string(),
            "-an".to_string(),
            "-sn".to_string(),
            "-dn".to_string(),
            "-vf".to_string(),
            format!("scale={THUMB_WIDTH}:-2"),
            "-frames:v".to_string(),
            "1".to_string(),
            "-q:v".to_string(),
            "3".to_string(),
            "-y".to_string(),
            out.to_string_lossy().into_owned(),
        ];
        command_output(&self.ffmpeg_path, &args, GRAB_TIMEOUT).await?;
        // ffmpeg exits 0 without writing when the seek lands past the
        // last frame.
        tokio::fs::metadata(out).await?;
        Ok(())
    }
}

/// 20% into the runtime, which skips cold opens and title cards, kept
/// at least a second before the end so the seek always lands on a frame.
pub fn seek_ms(duration_ms: u64) -> u64 {
    (duration_ms / 5).min(duration_ms.saturating_sub(1000))
}

async fn forward_events(bus: DomainEventBus, tx: mpsc::Sender<()>) {
    let mut rx = bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(DomainEvent::ScanCompleted { .. }) => {
                // A full channel already holds a pending sweep.
                let _ = tx.try_send(());
            }
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "episode thumb worker lagged; events dropped");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::seek_ms;

    #[test]
    fn seek_is_a_fifth_in_and_clamped_before_the_end() {
        assert_eq!(seek_ms(2_700_000), 540_000);
        assert_eq!(seek_ms(1_100), 100);
        assert_eq!(seek_ms(1_000), 0);
        assert_eq!(seek_ms(500), 0);
    }
}
