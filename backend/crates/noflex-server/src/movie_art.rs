// SPDX-License-Identifier: AGPL-3.0-only
//
// Fallback art for films that nothing else has art for: ffmpeg's `thumbnail`
// filter picks the most representative of a run of keyframes, and that frame
// becomes a `Generated` poster and backdrop under the image cache. Both are
// generated apart from each other's presence, since a provider's poster with no
// backdrop would leave the hero banner empty. Generated art ranks last, so a
// local or provider image found later wins without anything being deleted.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use image::{RgbImage, codecs::jpeg::JpegEncoder, imageops};
use noflex_core::{DomainEvent, DomainEventBus, ImageKind, TonemapCurve, WorkId, WorkKind};
use noflex_storage::{
    Storage,
    domain::{Image, ImageSource, ImageTarget, MediaFile, Work},
};
use noflex_transcoder::ffmpeg_argv::cpu_tonemap_filter;
use time::OffsetDateTime;
use tokio::{
    sync::{broadcast::error::RecvError, mpsc},
    task::JoinHandle,
    time::{Duration, Instant, interval_at, sleep_until},
};

use crate::subprocess::{CommandError, command_output};

const SWEEP_PAGE: u32 = 32;
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// The metadata worker gets this long at a new film first. A first scan of a big
/// library would otherwise decode a frame of every film for art that's about to
/// be outranked.
const METADATA_GRACE: Duration = Duration::from_secs(10 * 60);
const GRAB_TIMEOUT: Duration = Duration::from_secs(2 * 60);
/// The listing has no failure state, so failures are remembered in memory to
/// avoid re-running ffmpeg on them every sweep.
const FAILURE_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60);

/// Percentages into the runtime to look. The first skips cold opens and logos,
/// the next two are for a first look that came out dark, and the start of the file
/// is the one place a clip with sparse keyframes is sure to have one.
const LOOK_AT_PERCENT: [u64; 4] = [20, 45, 70, 0];
/// Bounds what's read from a high-bitrate remux, whose keyframes are far apart in
/// bytes even when skipping everything between them.
const WINDOW_SECONDS: u32 = 180;
/// `thumbnail` holds every candidate in memory until it chooses, about 90 MB here
/// at the width below.
const CANDIDATES: u32 = 30;
const FRAME_WIDTH: u32 = 1920;
const JPEG_QUALITY: u8 = 85;
/// Below this average luma a frame is a fade or a night scene nobody could read
/// at poster size.
const DARK_LUMA: u64 = 24;
/// A row or column of a bar has almost nothing above this luma, leaving room for
/// JPEG ringing and stray specks.
const BAR_LUMA: u32 = 32;
const BAR_SPECKS_PER_MILLE: usize = 10;
/// Directory under the image cache root holding `<work id>-<kind>.jpg`.
pub const ART_DIR: &str = "movie-art";

#[derive(Debug, thiserror::Error)]
pub enum MovieArtError {
    #[error("storage: {0}")]
    Storage(#[from] noflex_storage::StorageError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("art is only generated for movies")]
    NotMovie,
    #[error("movie has no media file")]
    NoMedia,
    #[error("media has no usable duration")]
    MissingDuration,
    #[error("no frame came out of the source")]
    NoFrame,
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("image task: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Command(#[from] CommandError),
}

#[derive(Clone)]
pub struct MovieArtWorker {
    storage: Arc<dyn Storage>,
    ffmpeg_path: PathBuf,
    cache_root: PathBuf,
    backoff: Arc<std::sync::Mutex<HashMap<WorkId, Instant>>>,
}

impl MovieArtWorker {
    /// `cache_root` is the image cache dir the image route serves from.
    pub fn new(storage: Arc<dyn Storage>, ffmpeg_path: PathBuf, cache_root: PathBuf) -> Self {
        Self {
            storage,
            ffmpeg_path,
            cache_root,
            backoff: Arc::default(),
        }
    }

    /// Sweep once now, then [`METADATA_GRACE`] after a `ScanCompleted`, and hourly.
    pub fn spawn(self: Arc<Self>, events: &DomainEventBus) -> JoinHandle<()> {
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(forward_events(events.clone(), tx.clone()));
        tokio::spawn(self.run(rx, tx))
    }

    async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<()>, _keepalive: mpsc::Sender<()>) {
        let mut tick = interval_at(Instant::now() + SWEEP_INTERVAL, SWEEP_INTERVAL);
        self.run_sweep().await;
        let mut after_scan = None;
        loop {
            tokio::select! {
                message = rx.recv() => match message {
                    // Set once rather than pushed back, so a server that's always
                    // scanning something still gets here.
                    Some(()) => {
                        after_scan.get_or_insert_with(|| Instant::now() + METADATA_GRACE);
                    }
                    None => break,
                },
                () = sleep_until(after_scan.unwrap_or_else(Instant::now)), if after_scan.is_some() => {
                    after_scan = None;
                    self.run_sweep().await;
                }
                _ = tick.tick() => self.run_sweep().await,
            }
        }
    }

    /// Generate what's missing for every film past its grace period. Films that
    /// failed within the last day are skipped.
    pub async fn run_sweep(&self) {
        for kind in [ImageKind::Primary, ImageKind::Backdrop] {
            let mut cursor = None;
            loop {
                let page = match self
                    .storage
                    .works_missing_image(
                        WorkKind::Movie,
                        kind,
                        ImageSource::ALL,
                        SWEEP_PAGE,
                        cursor,
                    )
                    .await
                {
                    Ok(page) => page,
                    Err(error) => {
                        tracing::warn!(%error, "movie art sweep listing failed");
                        return;
                    }
                };
                for work in &page.items {
                    if self.backed_off(work.id) || within_grace(work) {
                        continue;
                    }
                    if let Err(error) = self.generate(work).await {
                        tracing::warn!(work = %work.id, %error, "movie art generation failed");
                        self.park(work.id);
                    }
                }
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
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

    /// Record a poster and backdrop for whichever of the two `movie` lacks, and
    /// return what was written. No partial file is left behind on error.
    ///
    /// # Errors
    /// [`MovieArtError::NotMovie`] for other work kinds; [`MovieArtError::NoMedia`]
    /// or [`MovieArtError::MissingDuration`] with nothing to grab from; ffmpeg,
    /// decoding and storage failures otherwise.
    pub async fn generate(&self, movie: &Work) -> Result<Vec<Image>, MovieArtError> {
        if movie.kind != WorkKind::Movie {
            return Err(MovieArtError::NotMovie);
        }
        let target = ImageTarget::Work(movie.id);
        let mut missing = Vec::new();
        for kind in [ImageKind::Primary, ImageKind::Backdrop] {
            if self
                .storage
                .image(target, kind, ImageSource::ALL)
                .await?
                .is_none()
            {
                missing.push(kind);
            }
        }
        if missing.is_empty() {
            return Ok(Vec::new());
        }

        let (media, duration_ms) = self.source(movie.id).await?;
        let dir = self.cache_root.join(ART_DIR);
        tokio::fs::create_dir_all(&dir).await?;
        // Unique temp siblings + rename: concurrent sweeps never publish a partial
        // file, and a failed grab never clobbers a previous one.
        let temp_id = uuid::Uuid::now_v7();
        let frame_path = dir.join(format!(".{}.{temp_id}.jpg", movie.id));
        let frame = self.pick_frame(&media, duration_ms, &frame_path).await;
        let _ = tokio::fs::remove_file(&frame_path).await;
        let frame = frame?;

        let mut written = Vec::new();
        for kind in missing {
            let rendered = tmp_and_dest(&dir, movie.id, kind, temp_id);
            let image = self
                .publish(target, kind, Arc::clone(&frame), &rendered)
                .await;
            if image.is_err() {
                let _ = tokio::fs::remove_file(&rendered.0).await;
            }
            written.extend(image?);
        }
        Ok(written)
    }

    /// The first frame bright enough to read, or the last one looked at if none is.
    async fn pick_frame(
        &self,
        media: &MediaFile,
        duration_ms: u64,
        out: &Path,
    ) -> Result<Arc<RgbImage>, MovieArtError> {
        let filter = frame_filter(&media.probe.0);
        let mut last = None;
        for percent in LOOK_AT_PERCENT {
            let at_ms = seek_ms(duration_ms, percent);
            self.grab(&media.path, at_ms, &filter, out).await?;
            // ffmpeg exits 0 without writing when no keyframe falls in the window.
            if tokio::fs::metadata(out).await.is_err() {
                continue;
            }
            let path = out.to_path_buf();
            let frame = tokio::task::spawn_blocking(move || {
                image::open(path).map(image::DynamicImage::into_rgb8)
            })
            .await??;
            if !is_dark(&frame) {
                return Ok(Arc::new(frame));
            }
            last = Some(frame);
        }
        last.map(Arc::new).ok_or(MovieArtError::NoFrame)
    }

    /// Render `kind` from `frame` and record it, unless one appeared meanwhile
    /// (a scan found a local one, say).
    async fn publish(
        &self,
        target: ImageTarget,
        kind: ImageKind,
        frame: Arc<RgbImage>,
        (tmp, dest): &(PathBuf, PathBuf),
    ) -> Result<Option<Image>, MovieArtError> {
        let path = tmp.clone();
        let (width, height) = tokio::task::spawn_blocking(move || {
            let rendered = render(&frame, kind);
            let file = std::io::BufWriter::new(std::fs::File::create(path)?);
            JpegEncoder::new_with_quality(file, JPEG_QUALITY).encode_image(&rendered)?;
            Ok::<_, MovieArtError>(rendered.dimensions())
        })
        .await??;

        if self
            .storage
            .image(target, kind, ImageSource::ALL)
            .await?
            .is_some()
        {
            let _ = tokio::fs::remove_file(tmp).await;
            return Ok(None);
        }
        tokio::fs::rename(tmp, dest).await?;
        let image = Image {
            target,
            kind,
            source: ImageSource::Generated,
            path: dest.to_string_lossy().into_owned(),
            width: Some(width),
            height: Some(height),
            language: None,
        };
        self.storage.upsert_image(&image).await?;
        Ok(Some(image))
    }

    /// The film's first part, SDR over HDR since that needs no tone-mapping, and
    /// how long that part runs. A part's own duration comes first, as an edition's
    /// runtime spans all its parts and could seek past the end of this one.
    async fn source(&self, movie: WorkId) -> Result<(MediaFile, u64), MovieArtError> {
        let mut editions = self.storage.editions_for_work(movie).await?;
        editions.sort_by_key(|e| e.id.into_uuid());
        for edition in editions {
            let Some(media) = self
                .storage
                .media_files_for_edition(edition.id)
                .await?
                .into_iter()
                .min_by_key(|m| (m.part_index, is_hdr(&m.probe.0)))
            else {
                continue;
            };
            let duration_ms = media
                .probe
                .0
                .pointer("/duration_ms")
                .and_then(|v| v.as_u64())
                .or_else(|| edition.runtime_ms.and_then(|ms| u64::try_from(ms).ok()))
                .filter(|ms| *ms > 0)
                .ok_or(MovieArtError::MissingDuration)?;
            return Ok((media, duration_ms));
        }
        Err(MovieArtError::NoMedia)
    }

    async fn grab(
        &self,
        source: &str,
        at_ms: u64,
        filter: &str,
        out: &Path,
    ) -> Result<(), MovieArtError> {
        // `-discard nokey` drops the rest before decoding, in any codec, which is
        // what makes a few minutes of 4K AV1 take under a second.
        let args = vec![
            "-hide_banner".to_string(),
            "-loglevel".to_string(),
            "error".to_string(),
            "-discard".to_string(),
            "nokey".to_string(),
            "-ss".to_string(),
            format!("{}.{:03}", at_ms / 1000, at_ms % 1000),
            "-t".to_string(),
            WINDOW_SECONDS.to_string(),
            "-i".to_string(),
            source.to_string(),
            "-an".to_string(),
            "-sn".to_string(),
            "-dn".to_string(),
            "-vf".to_string(),
            filter.to_string(),
            "-frames:v".to_string(),
            "1".to_string(),
            "-q:v".to_string(),
            "2".to_string(),
            "-y".to_string(),
            out.to_string_lossy().into_owned(),
        ];
        command_output(&self.ffmpeg_path, &args, GRAB_TIMEOUT).await?;
        Ok(())
    }
}

fn within_grace(work: &Work) -> bool {
    OffsetDateTime::now_utc() - work.added_at < METADATA_GRACE
}

fn tmp_and_dest(
    dir: &Path,
    movie: WorkId,
    kind: ImageKind,
    temp_id: uuid::Uuid,
) -> (PathBuf, PathBuf) {
    let name = kind.as_str();
    (
        dir.join(format!(".{movie}-{name}.{temp_id}.jpg")),
        dir.join(format!("{movie}-{name}.jpg")),
    )
}

fn is_hdr(probe: &serde_json::Value) -> bool {
    matches!(
        probe.pointer("/video/transfer").and_then(|v| v.as_str()),
        Some("smpte2084" | "arib-std-b67")
    )
}

/// Candidates are scaled before `thumbnail` holds on to them. An HDR source is
/// tone-mapped too, as its frames would otherwise come out grey and washed out.
fn frame_filter(probe: &serde_json::Value) -> String {
    let size = |field: &str| {
        probe
            .pointer(&format!("/video/{field}"))
            .and_then(|v| v.as_u64())
            .filter(|v| *v > 0)
    };
    let scale = match (is_hdr(probe), size("width"), size("height")) {
        (true, Some(width), Some(height)) => {
            let scaled_width = width.min(u64::from(FRAME_WIDTH));
            let even = |v: u64| u32::try_from(v & !1).unwrap_or(FRAME_WIDTH);
            cpu_tonemap_filter(
                even(scaled_width),
                even(height * scaled_width / width),
                TonemapCurve::default(),
            )
        }
        _ => format!("scale='min({FRAME_WIDTH},iw)':-2"),
    };
    format!("{scale},thumbnail={CANDIDATES}")
}

/// `percent` into the runtime, kept at least a second before the end so the seek
/// has something to land on.
fn seek_ms(duration_ms: u64, percent: u64) -> u64 {
    (duration_ms * percent / 100).min(duration_ms.saturating_sub(1000))
}

fn luma(pixel: &image::Rgb<u8>) -> u32 {
    let [r, g, b] = pixel.0;
    (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000
}

fn is_dark(frame: &RgbImage) -> bool {
    let pixels = u64::from(frame.width()) * u64::from(frame.height());
    let total: u64 = frame.pixels().map(|p| u64::from(luma(p))).sum();
    pixels == 0 || total / pixels < DARK_LUMA
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// `frame` without the black bars a remux keeps baked into its picture. A dark
/// scene looks like bars from edge to edge, so an axis that would lose over half
/// of itself keeps everything.
fn picture_bounds(frame: &RgbImage) -> Bounds {
    let (width, height) = frame.dimensions();
    let is_bar = |pixels: &mut dyn Iterator<Item = &image::Rgb<u8>>, length: u32| {
        let specks = pixels.filter(|p| luma(p) > BAR_LUMA).count();
        specks * 1000 <= length as usize * BAR_SPECKS_PER_MILLE
    };
    let row_is_bar = |y: u32| is_bar(&mut (0..width).map(|x| frame.get_pixel(x, y)), width);
    let column_is_bar = |x: u32| is_bar(&mut (0..height).map(|y| frame.get_pixel(x, y)), height);
    let (y, picture_height) = trim_axis(height, row_is_bar);
    let (x, picture_width) = trim_axis(width, column_is_bar);
    Bounds {
        x,
        y,
        width: picture_width,
        height: picture_height,
    }
}

fn trim_axis(length: u32, is_bar: impl Fn(u32) -> bool) -> (u32, u32) {
    let start = (0..length).find(|i| !is_bar(*i)).unwrap_or(0);
    let end = (0..length)
        .rev()
        .find(|i| !is_bar(*i))
        .map_or(length, |i| i + 1);
    let kept = end.saturating_sub(start);
    if kept * 2 < length {
        (0, length)
    } else {
        (start, kept)
    }
}

/// The 2:3 middle of `picture`. A frame narrower than that keeps its width and
/// loses height instead.
fn poster_bounds(picture: Bounds) -> Bounds {
    let width = picture.width.min(picture.height * 2 / 3);
    let height = picture.height.min(picture.width * 3 / 2);
    Bounds {
        x: picture.x + (picture.width - width) / 2,
        y: picture.y + (picture.height - height) / 2,
        width,
        height,
    }
}

fn render(frame: &RgbImage, kind: ImageKind) -> RgbImage {
    let picture = picture_bounds(frame);
    let bounds = if kind == ImageKind::Primary {
        poster_bounds(picture)
    } else {
        picture
    };
    imageops::crop_imm(frame, bounds.x, bounds.y, bounds.width, bounds.height).to_image()
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
                tracing::warn!(missed, "movie art worker lagged; events dropped");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;

    const WHITE: Rgb<u8> = Rgb([230, 230, 230]);

    fn letterboxed(width: u32, height: u32, bar: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |_, y| {
            if y < bar || y >= height - bar {
                Rgb([2, 2, 2])
            } else {
                WHITE
            }
        })
    }

    #[test]
    fn bars_are_trimmed_from_the_picture() {
        assert_eq!(
            picture_bounds(&letterboxed(1920, 1080, 140)),
            Bounds {
                x: 0,
                y: 140,
                width: 1920,
                height: 800,
            }
        );
    }

    #[test]
    fn a_frame_without_bars_is_left_alone() {
        assert_eq!(
            picture_bounds(&RgbImage::from_pixel(640, 360, WHITE)),
            Bounds {
                x: 0,
                y: 0,
                width: 640,
                height: 360,
            }
        );
    }

    #[test]
    fn a_dark_scene_is_not_mistaken_for_bars() {
        let night = RgbImage::from_fn(640, 360, |x, y| {
            if (300..320).contains(&x) && (170..190).contains(&y) {
                WHITE
            } else {
                Rgb([4, 4, 4])
            }
        });
        assert_eq!(
            picture_bounds(&night),
            Bounds {
                x: 0,
                y: 0,
                width: 640,
                height: 360,
            }
        );
    }

    #[test]
    fn a_poster_is_the_middle_two_thirds_by_height() {
        let poster = poster_bounds(Bounds {
            x: 0,
            y: 140,
            width: 1920,
            height: 800,
        });
        assert_eq!(
            poster,
            Bounds {
                x: 693,
                y: 140,
                width: 533,
                height: 800,
            }
        );
    }

    #[test]
    fn a_tall_frame_loses_height_for_its_poster() {
        let poster = poster_bounds(Bounds {
            x: 0,
            y: 0,
            width: 400,
            height: 1000,
        });
        assert_eq!(
            poster,
            Bounds {
                x: 0,
                y: 200,
                width: 400,
                height: 600,
            }
        );
    }

    #[test]
    fn hdr_sources_are_tone_mapped_at_the_frame_size() {
        let filter = frame_filter(&serde_json::json!({
            "video": { "transfer": "smpte2084", "width": 3840, "height": 1608 },
        }));
        assert!(filter.contains("tonemap=tonemap=hable"), "{filter}");
        assert!(filter.contains("w=1920:h=804"), "{filter}");
        assert!(
            filter.ends_with(&format!("thumbnail={CANDIDATES}")),
            "{filter}"
        );
    }

    #[test]
    fn sdr_sources_are_only_scaled() {
        let filter = frame_filter(&serde_json::json!({
            "video": { "transfer": "bt709", "width": 1280, "height": 720 },
        }));
        assert_eq!(
            filter,
            format!("scale='min(1920,iw)':-2,thumbnail={CANDIDATES}")
        );
    }

    #[test]
    fn looks_stay_inside_short_files() {
        assert_eq!(seek_ms(7_200_000, 20), 1_440_000);
        assert_eq!(seek_ms(1_100, 70), 100);
        assert_eq!(seek_ms(500, 70), 0);
    }
}
