// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use noflex_core::{DomainEvent, DomainEventBus, EditionId, WorkKind};
use noflex_storage::{
    Storage,
    domain::{Edition, MediaFile, TrickplayAsset, TrickplayChapterCue, TrickplaySheet},
};
use time::OffsetDateTime;
use tokio::{
    sync::{broadcast::error::RecvError, mpsc},
    task::JoinHandle,
    time::{Duration, Instant, interval_at},
};

use crate::subprocess::{CommandError, command_output};

const INTERVAL_MS: u64 = 10_000;
const THUMB_WIDTH: u32 = 160;
const TILE_COLUMNS: u32 = 10;
const TILE_ROWS: u32 = 10;
const SWEEP_PAGE: u32 = 32;
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// One sheet decodes at most `TILE_COLUMNS * TILE_ROWS * INTERVAL_MS`
/// (~17 min) of source, so a fixed bound is generous even for slow
/// software decodes of 4K HEVC; past it the decode is hung.
const SHEET_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2 * 60);
/// The storage listing has no failure state, so the sweep remembers
/// failures in memory to avoid re-running ffmpeg on them every hour.
const FAILURE_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum TrickplayError {
    #[error("storage: {0}")]
    Storage(#[from] noflex_storage::StorageError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("probe output: {0}")]
    ProbeJson(#[from] serde_json::Error),
    #[error("media has no usable video duration")]
    MissingDuration,
    #[error("media has no usable video dimensions")]
    MissingDimensions,
    #[error("trickplay is only generated for movies and episodes")]
    NotVideo,
    #[error(transparent)]
    Command(#[from] CommandError),
}

#[derive(Clone)]
pub struct TrickplayWorker {
    storage: Arc<dyn Storage>,
    ffmpeg_path: PathBuf,
    ffprobe_path: PathBuf,
    cache_root: PathBuf,
    /// Edition → earliest next sweep attempt. Non-video editions are
    /// parked for the process lifetime.
    backoff: Arc<std::sync::Mutex<std::collections::HashMap<EditionId, Option<Instant>>>>,
}

impl TrickplayWorker {
    pub fn new(storage: Arc<dyn Storage>, ffmpeg_path: PathBuf, cache_root: PathBuf) -> Self {
        let ffprobe_path = ffmpeg_path.with_file_name(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        Self {
            storage,
            ffmpeg_path,
            ffprobe_path,
            cache_root,
            backoff: Arc::default(),
        }
    }

    pub fn with_ffprobe_path(mut self, path: PathBuf) -> Self {
        self.ffprobe_path = path;
        self
    }

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

    pub async fn run_sweep(&self) {
        let mut cursor = None;
        loop {
            let page = match self
                .storage
                .editions_missing_trickplay(SWEEP_PAGE, cursor)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "trickplay sweep listing failed");
                    return;
                }
            };
            if page.items.is_empty() {
                return;
            }
            let next = page.next_cursor;
            for edition in page.items {
                if self.backed_off(edition.id) {
                    continue;
                }
                match self.generate(&edition).await {
                    Ok(_) => {}
                    Err(TrickplayError::NotVideo) => self.park(edition.id, None),
                    Err(error) => {
                        tracing::warn!(edition = %edition.id, %error, "trickplay generation failed");
                        self.park(edition.id, Some(Instant::now() + FAILURE_BACKOFF));
                    }
                }
            }
            match next {
                Some(next) => cursor = Some(next),
                None => return,
            }
        }
    }

    fn backed_off(&self, edition: EditionId) -> bool {
        let Ok(mut backoff) = self.backoff.lock() else {
            return false;
        };
        match backoff.get(&edition) {
            Some(None) => true,
            Some(Some(until)) if *until > Instant::now() => true,
            Some(Some(_)) => {
                backoff.remove(&edition);
                false
            }
            None => false,
        }
    }

    fn park(&self, edition: EditionId, until: Option<Instant>) {
        if let Ok(mut backoff) = self.backoff.lock() {
            backoff.insert(edition, until);
        }
    }

    /// Render and persist trickplay for `edition`.
    ///
    /// # Errors
    /// [`TrickplayError::NotVideo`] unless the edition's work is a movie
    /// or episode; ffmpeg/ffprobe failures and timeouts otherwise.
    pub async fn generate(&self, edition: &Edition) -> Result<TrickplayAsset, TrickplayError> {
        let is_video = self
            .storage
            .get_work(edition.work_id)
            .await?
            .is_some_and(|work| matches!(work.kind, WorkKind::Movie | WorkKind::Episode));
        if !is_video {
            return Err(TrickplayError::NotVideo);
        }
        let Some(media) = self
            .storage
            .media_files_for_edition(edition.id)
            .await?
            .into_iter()
            .min_by_key(|media| media.part_index)
        else {
            return Err(TrickplayError::MissingDuration);
        };
        let stored_probe = stored_probe_info(&media);
        let needs_probe = edition.runtime_ms.is_none() && stored_probe.duration_ms.is_none()
            || stored_probe.width.is_none()
            || stored_probe.height.is_none();
        let probe = if needs_probe {
            self.probe(&media).await?
        } else {
            ProbeInfo::default()
        };
        let duration_ms = edition
            .runtime_ms
            .and_then(|value| u64::try_from(value).ok())
            .or(stored_probe.duration_ms)
            .or(probe.duration_ms)
            .filter(|duration| *duration > 0)
            .ok_or(TrickplayError::MissingDuration)?;
        let source_width = stored_probe
            .width
            .or(probe.width)
            .filter(|width| *width > 0);
        let source_height = stored_probe
            .height
            .or(probe.height)
            .filter(|height| *height > 0);
        let (source_width, source_height) = source_width
            .zip(source_height)
            .ok_or(TrickplayError::MissingDimensions)?;
        let thumb_height = scaled_height(source_width, source_height);
        let frame_count = duration_ms.div_ceil(INTERVAL_MS).max(1);
        let tiles_per_sheet = u64::from(TILE_COLUMNS * TILE_ROWS);
        let sheet_count = frame_count.div_ceil(tiles_per_sheet);

        let root = self.cache_root.join("trickplay");
        tokio::fs::create_dir_all(&root).await?;
        let edition_dir = root.join(edition.id.to_string());
        tokio::fs::create_dir_all(&edition_dir).await?;
        let nonce = uuid::Uuid::now_v7();
        let temp_dir = edition_dir.join(format!(".{nonce}.tmp"));
        tokio::fs::create_dir(&temp_dir).await?;
        let generated = self
            .render_sheets(
                &media,
                edition.id,
                &temp_dir,
                frame_count,
                sheet_count,
                tiles_per_sheet,
            )
            .await;
        if let Err(error) = generated {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(error);
        }

        let vtt = build_vtt(
            edition.id,
            duration_ms,
            frame_count,
            thumb_height,
            tiles_per_sheet,
        );
        tokio::fs::write(temp_dir.join("index.vtt"), vtt).await?;
        // Versioned directories keep the previously published manifest
        // readable until the replacement is committed to storage.
        let final_dir = edition_dir.join(nonce.to_string());
        if let Err(error) = tokio::fs::rename(&temp_dir, &final_dir).await {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(error.into());
        }

        let sheets = (0..sheet_count)
            .map(|index| TrickplaySheet {
                edition_id: edition.id,
                index: index as u32,
                path: final_dir
                    .join(format!("sheet-{index:04}.jpg"))
                    .to_string_lossy()
                    .into_owned(),
                tile_count: (frame_count - index * tiles_per_sheet).min(tiles_per_sheet) as u32,
            })
            .collect();
        let asset = TrickplayAsset {
            edition_id: edition.id,
            vtt_path: final_dir.join("index.vtt").to_string_lossy().into_owned(),
            thumb_width: THUMB_WIDTH,
            thumb_height,
            tile_columns: TILE_COLUMNS,
            tile_rows: TILE_ROWS,
            interval_ms: INTERVAL_MS,
            source_media_mtime: media.mtime,
            source_fingerprint: source_fingerprint(&media),
            generated_at: OffsetDateTime::now_utc(),
            sheets,
            chapter_cues: chapter_cues(
                edition,
                frame_count,
                duration_ms,
                thumb_height,
                tiles_per_sheet,
            ),
        };
        if let Err(error) = self.storage.upsert_trickplay(&asset).await {
            let _ = tokio::fs::remove_dir_all(&final_dir).await;
            return Err(error.into());
        }
        cleanup_version_dirs(&root, &edition_dir, &final_dir).await;
        Ok(asset)
    }

    async fn render_sheets(
        &self,
        media: &MediaFile,
        edition: EditionId,
        output_dir: &Path,
        frame_count: u64,
        sheet_count: u64,
        tiles_per_sheet: u64,
    ) -> Result<(), TrickplayError> {
        for index in 0..sheet_count {
            let tile_count = (frame_count - index * tiles_per_sheet).min(tiles_per_sheet);
            let start_ms = index * tiles_per_sheet * INTERVAL_MS;
            let output = output_dir.join(format!("sheet-{index:04}.jpg"));
            let filter = format!(
                "fps=1/{}:start_time=0,scale={}:-2,tile={}x{}:nb_frames={}:padding=0:margin=0",
                INTERVAL_MS / 1000,
                THUMB_WIDTH,
                TILE_COLUMNS,
                TILE_ROWS,
                tile_count
            );
            let args = vec![
                "-hide_banner".to_string(),
                "-loglevel".to_string(),
                "error".to_string(),
                "-ss".to_string(),
                format_seconds(start_ms),
                "-i".to_string(),
                media.path.clone(),
                "-an".to_string(),
                "-sn".to_string(),
                "-vf".to_string(),
                filter,
                "-frames:v".to_string(),
                "1".to_string(),
                "-q:v".to_string(),
                "3".to_string(),
                "-y".to_string(),
                output.to_string_lossy().into_owned(),
            ];
            run_command(&self.ffmpeg_path, &args, SHEET_TIMEOUT)
                .await
                .inspect_err(|_| {
                    tracing::debug!(%edition, sheet = index, "ffmpeg trickplay sheet failed");
                })?;
            tokio::fs::metadata(output).await?;
        }
        Ok(())
    }

    async fn probe(&self, media: &MediaFile) -> Result<ProbeInfo, TrickplayError> {
        let args = vec![
            "-v".to_string(),
            "error".to_string(),
            "-select_streams".to_string(),
            "v:0".to_string(),
            "-show_entries".to_string(),
            "format=duration:stream=width,height".to_string(),
            "-of".to_string(),
            "json".to_string(),
            media.path.clone(),
        ];
        let output = command_output(&self.ffprobe_path, &args, PROBE_TIMEOUT).await?;
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        Ok(ProbeInfo {
            duration_ms: value
                .pointer("/format/duration")
                .and_then(json_number)
                .map(|seconds| (seconds * 1000.0).ceil() as u64),
            width: value
                .pointer("/streams/0/width")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            height: value
                .pointer("/streams/0/height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
        })
    }
}

#[derive(Default)]
struct ProbeInfo {
    duration_ms: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
}

async fn cleanup_version_dirs(root: &Path, edition_dir: &Path, retained: &Path) {
    let (Ok(root), Ok(edition_dir), Ok(retained)) = (
        tokio::fs::canonicalize(root).await,
        tokio::fs::canonicalize(edition_dir).await,
        tokio::fs::canonicalize(retained).await,
    ) else {
        return;
    };
    if edition_dir.parent() != Some(root.as_path())
        || retained.parent() != Some(edition_dir.as_path())
    {
        tracing::warn!(path = %edition_dir.display(), "refusing to clean trickplay cache outside root");
        return;
    }
    let mut entries = match tokio::fs::read_dir(&edition_dir).await {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, path = %edition_dir.display(), "trickplay cache listing failed");
            return;
        }
    };
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, path = %edition_dir.display(), "trickplay cache listing failed");
                break;
            }
        };
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        let path = entry.path();
        if !file_type.is_dir() || path == retained {
            continue;
        }
        if let Err(error) = tokio::fs::remove_dir_all(&path).await {
            tracing::warn!(%error, path = %path.display(), "trickplay cache cleanup failed");
        }
    }
}

async fn forward_events(bus: DomainEventBus, tx: mpsc::Sender<()>) {
    let mut rx = bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(DomainEvent::ScanCompleted { .. }) => {
                if let Err(error) = tx.try_send(()) {
                    tracing::warn!(%error, "trickplay sweep trigger dropped");
                }
            }
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "trickplay worker lagged; events dropped");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

async fn run_command(
    program: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<(), TrickplayError> {
    command_output(program, args, timeout).await?;
    Ok(())
}

fn stored_probe_info(media: &MediaFile) -> ProbeInfo {
    ProbeInfo {
        duration_ms: media
            .probe
            .0
            .pointer("/format/duration")
            .and_then(json_number)
            .map(|seconds| (seconds * 1000.0).ceil() as u64),
        width: media
            .probe
            .0
            .pointer("/video/width")
            .or_else(|| media.probe.0.pointer("/streams/0/width"))
            .and_then(|value| value.as_u64())
            .map(|value| value as u32),
        height: media
            .probe
            .0
            .pointer("/video/height")
            .or_else(|| media.probe.0.pointer("/streams/0/height"))
            .and_then(|value| value.as_u64())
            .map(|value| value as u32),
    }
}

fn json_number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn scaled_height(width: u32, height: u32) -> u32 {
    let scaled = (u64::from(height) * u64::from(THUMB_WIDTH)).div_ceil(u64::from(width));
    scaled.div_ceil(2).max(1) as u32 * 2
}

fn format_seconds(milliseconds: u64) -> String {
    format!("{}.{:03}", milliseconds / 1000, milliseconds % 1000)
}

fn source_fingerprint(media: &MediaFile) -> String {
    noflex_storage::canonical::trickplay_source_fingerprint(
        &media.path,
        media.size_bytes,
        media.mtime,
    )
}

fn format_vtt_time(milliseconds: u64) -> String {
    let hours = milliseconds / 3_600_000;
    let minutes = (milliseconds / 60_000) % 60;
    let seconds = (milliseconds / 1000) % 60;
    let millis = milliseconds % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

pub fn build_vtt(
    edition: EditionId,
    duration_ms: u64,
    frame_count: u64,
    thumb_height: u32,
    tiles_per_sheet: u64,
) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for frame in 0..frame_count {
        let start = frame * INTERVAL_MS;
        let end = ((frame + 1) * INTERVAL_MS).min(duration_ms);
        let in_sheet = frame % tiles_per_sheet;
        let sheet = frame / tiles_per_sheet;
        let x = (in_sheet % u64::from(TILE_COLUMNS)) * u64::from(THUMB_WIDTH);
        let y = (in_sheet / u64::from(TILE_COLUMNS)) * u64::from(thumb_height);
        out.push_str(&format!(
            "{} --> {}\n/trickplay/{edition}/sheets/{sheet}#xywh={x},{y},{THUMB_WIDTH},{thumb_height}\n\n",
            format_vtt_time(start),
            format_vtt_time(end),
        ));
    }
    out
}

fn chapter_cues(
    edition: &Edition,
    frame_count: u64,
    duration_ms: u64,
    thumb_height: u32,
    tiles_per_sheet: u64,
) -> Vec<TrickplayChapterCue> {
    edition
        .chapters
        .iter()
        .filter(|chapter| chapter.position_ms >= 0)
        .map(|chapter| {
            let position = chapter.position_ms as u64;
            let frame = (position / INTERVAL_MS).min(frame_count - 1);
            let in_sheet = frame % tiles_per_sheet;
            TrickplayChapterCue {
                edition_id: edition.id,
                position_ms: chapter.position_ms,
                title: chapter.title.clone(),
                cue_start_ms: (frame * INTERVAL_MS) as i64,
                cue_end_ms: ((frame + 1) * INTERVAL_MS).min(duration_ms) as i64,
                sheet_index: (frame / tiles_per_sheet) as u32,
                x: (in_sheet % u64::from(TILE_COLUMNS)) as u32 * THUMB_WIDTH,
                y: (in_sheet / u64::from(TILE_COLUMNS)) as u32 * thumb_height,
                width: THUMB_WIDTH,
                height: thumb_height,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtt_rolls_over_to_the_next_sheet() {
        let edition = EditionId::new();
        let vtt = build_vtt(edition, 1_010_000, 101, 90, 100);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains(&format!(
            "/trickplay/{edition}/sheets/0#xywh=1440,810,160,90"
        )));
        assert!(vtt.contains(&format!("/trickplay/{edition}/sheets/1#xywh=0,0,160,90")));
        assert!(vtt.contains("00:16:40.000 --> 00:16:50.000"));
    }
}
