// SPDX-License-Identifier: AGPL-3.0-only
//
// On-demand HLS for a title. A session's media playlists are whole VOD
// playlists on a fixed grid from the start, so a player sees the real
// timeline and seeks anywhere, and a variant is encoded from whichever
// segment a player asks for: a run of ffmpeg starts there and keeps going
// while the player keeps up with it. Asking far ahead of a run, or for
// another variant, starts another run. With more than one variant the master
// playlist is a ladder that the player's own adaptive bitrate moves along
// without a restart.

use std::{
    collections::BTreeSet,
    fmt::Write,
    path::PathBuf,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use noflex_core::{AudioCodec, ids::MediaFileId};
use tokio::{sync::Mutex, time::Instant};

use crate::{
    JobStatus, TranscodeRequest,
    decision::{AudioPlan, HlsSegmentContainer},
    job::Launcher,
};

/// How far ahead of a run's last finished segment a request may be and
/// still be waited for, rather than starting a run where it asked.
const AHEAD: u32 = 5;
/// The longest a request waits for its segment. Players are told to wait
/// longer than this for a segment (see each client's player).
const FETCH_WAIT: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(100);
/// A variant nobody has asked anything of for this long has been left by
/// a player that switched, and its run stops.
const SWITCH_IDLE: Duration = Duration::from_secs(12);
/// A session nobody has asked anything of for this long is paused or gone,
/// and its runs stop until it's asked again.
const PAUSED_IDLE: Duration = Duration::from_secs(120);
const REAP_EVERY: Duration = Duration::from_secs(15);

/// Fixed-length segments over a whole title.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentGrid {
    pub duration_secs: f64,
    pub segment_secs: u32,
}

impl SegmentGrid {
    /// Number of segments; at least one.
    pub fn count(&self) -> u32 {
        let count = (self.duration_secs / f64::from(self.segment_secs.max(1))).ceil();
        (count as u32).max(1)
    }

    pub fn start_secs(&self, index: u32) -> f64 {
        f64::from(index) * f64::from(self.segment_secs)
    }

    /// The segment holding `secs`, clamped to the grid.
    pub fn index_at(&self, secs: f64) -> u32 {
        let index = (secs.max(0.0) / f64::from(self.segment_secs.max(1))).floor() as u32;
        index.min(self.count() - 1)
    }

    /// Length of segment `index`; the last one holds what's left.
    pub fn length_secs(&self, index: u32) -> f64 {
        (self.duration_secs - self.start_secs(index)).clamp(0.0, f64::from(self.segment_secs))
    }

    /// Variant `variant`'s media playlist: every segment, from the start.
    pub fn media_playlist(&self, variant: usize, container: HlsSegmentContainer) -> String {
        let count = self.count();
        let mut out = String::with_capacity(160 + count as usize * 32);
        let _ = write!(
            out,
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n\
             #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n",
            self.segment_secs
        );
        if container == HlsSegmentContainer::Fmp4 {
            let _ = writeln!(out, "#EXT-X-MAP:URI=\"{}\"", init_file(variant));
        }
        for index in 0..count {
            let _ = writeln!(
                out,
                "#EXTINF:{:.6},\n{}",
                self.length_secs(index),
                segment_file(variant, index, container)
            );
        }
        out.push_str("#EXT-X-ENDLIST\n");
        out
    }
}

/// A file a player may ask an on-demand session for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Artifact {
    /// `v<n>.m3u8`, rendered from the grid.
    Playlist(usize),
    /// `v<n>_init.mp4`, the fMP4 init segment.
    Init(usize),
    /// `v<n>_<index>.m4s` or `.ts`.
    Segment(usize, u32),
}

impl Artifact {
    /// `None` for anything else, ffmpeg's own `v<n>_run.m3u8` included.
    pub fn parse(filename: &str) -> Option<Self> {
        let (stem, ext) = filename.rsplit_once('.')?;
        let rest = stem.strip_prefix('v')?;
        let number = |text: &str| text.bytes().all(|b| b.is_ascii_digit()) && !text.is_empty();
        match ext {
            "m3u8" if number(rest) => rest.parse().ok().map(Self::Playlist),
            "mp4" => {
                let variant = rest.strip_suffix("_init").filter(|v| number(v))?;
                variant.parse().ok().map(Self::Init)
            }
            "m4s" | "ts" => {
                let (variant, index) = rest.split_once('_')?;
                if !number(variant) || !number(index) {
                    return None;
                }
                Some(Self::Segment(variant.parse().ok()?, index.parse().ok()?))
            }
            _ => None,
        }
    }
}

pub fn variant_playlist(variant: usize) -> String {
    format!("v{variant}.m3u8")
}

fn stem(variant: usize) -> String {
    format!("v{variant}")
}

fn init_file(variant: usize) -> String {
    format!("v{variant}_init.mp4")
}

/// What ffmpeg writes for a run: the list of segments it has finished.
fn run_playlist(variant: usize) -> String {
    format!("v{variant}_run.m3u8")
}

fn segment_file(variant: usize, index: u32, container: HlsSegmentContainer) -> String {
    let ext = match container {
        HlsSegmentContainer::Fmp4 => "m4s",
        HlsSegmentContainer::Ts => "ts",
    };
    format!("v{variant}_{index:05}.{ext}")
}

/// The segment indices a run's playlist lists as finished.
fn finished_segments(playlist: &str) -> impl Iterator<Item = u32> + '_ {
    playlist
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| match Artifact::parse(line.trim()) {
            Some(Artifact::Segment(_, index)) => Some(index),
            _ => None,
        })
}

/// `audio` as an on-demand run carries it, from a source track of
/// `channels`. A copied track keeps its own timestamps through the input
/// seek a run starts with, so it wouldn't land on the grid: it's encoded to
/// the same codec where ffmpeg has an encoder, else to stereo AAC.
pub fn audio_for_runs(audio: AudioPlan, channels: u8) -> AudioPlan {
    let AudioPlan::Copy { codec } = audio else {
        return audio;
    };
    let most = match codec {
        AudioCodec::Aac | AudioCodec::Flac | AudioCodec::Opus => 8,
        AudioCodec::Ac3 | AudioCodec::Eac3 => 6,
        AudioCodec::Mp3 => 2,
        AudioCodec::Vorbis | AudioCodec::Truehd | AudioCodec::Dts | AudioCodec::Pcm => {
            return AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: channels.clamp(1, 2),
            };
        }
    };
    AudioPlan::Encode {
        codec,
        channels: channels.clamp(1, most),
    }
}

/// What one on-demand session encodes: a request per variant, best first,
/// whose output, start and naming each run fills in.
#[derive(Debug, Clone)]
pub struct SegmentedRequest {
    pub grid: SegmentGrid,
    pub variants: Vec<TranscodeRequest>,
}

/// What answering a request for an [`Artifact`] needs to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    Ready,
    Wait,
    Start,
}

/// A run that started at `start`, stops before `end`, and has finished
/// every segment before `head`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reach {
    start: u32,
    end: u32,
    head: u32,
}

fn next(done: &BTreeSet<u32>, run: Option<Reach>, index: u32) -> Next {
    if done.contains(&index) {
        return Next::Ready;
    }
    match run {
        Some(run) if run.start <= index && index < run.end && index < run.head + AHEAD => {
            Next::Wait
        }
        _ => Next::Start,
    }
}

#[derive(Debug)]
struct Run {
    id: String,
    reach: Reach,
}

struct Variant {
    template: TranscodeRequest,
    done: BTreeSet<u32>,
    run: Option<Run>,
    last_request: Instant,
}

/// Why an [`Artifact`] can't be served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// Not part of this session.
    NotFound,
    /// The encode failed, or didn't reach it in time.
    Unavailable,
}

/// What a request for an [`Artifact`] is answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    Playlist(String),
    File(PathBuf),
}

/// One on-demand session's variants and the runs encoding them.
pub(crate) struct SegmentedJob {
    id: String,
    media: MediaFileId,
    dir: PathBuf,
    grid: SegmentGrid,
    container: HlsSegmentContainer,
    launcher: Launcher,
    variants: Mutex<Vec<Variant>>,
    /// The segment last asked for, which an init request starts a run at.
    hint: AtomicU32,
    stopped: AtomicBool,
}

impl SegmentedJob {
    /// The session `id` writing under `dir`, with variant 0 already encoding
    /// from segment `first`. Fails only when there is nothing to encode.
    pub(crate) async fn start(
        id: String,
        media: MediaFileId,
        dir: PathBuf,
        request: SegmentedRequest,
        first: u32,
        launcher: Launcher,
    ) -> anyhow::Result<Arc<Self>> {
        let container = request
            .variants
            .first()
            .map(|variant| variant.plan.segment_container)
            .ok_or_else(|| anyhow::anyhow!("an on-demand session needs a variant"))?;
        let now = Instant::now();
        let job = Arc::new(Self {
            id,
            media,
            dir,
            grid: request.grid,
            container,
            launcher,
            variants: Mutex::new(
                request
                    .variants
                    .into_iter()
                    .map(|template| Variant {
                        template,
                        done: BTreeSet::new(),
                        run: None,
                        last_request: now,
                    })
                    .collect(),
            ),
            hint: AtomicU32::new(first),
            stopped: AtomicBool::new(false),
        });
        {
            let mut variants = job.variants.lock().await;
            job.start_run(&mut variants[0], 0, first.min(job.grid.count() - 1))
                .await?;
        }
        tokio::spawn(reap(Arc::downgrade(&job)));
        Ok(job)
    }

    /// Answer a request for `artifact`, waiting for its encode if need be.
    pub(crate) async fn fetch(&self, artifact: Artifact) -> Result<Fetched, FetchError> {
        let count = self.variants.lock().await.len();
        match artifact {
            Artifact::Playlist(variant) if variant < count => Ok(Fetched::Playlist(
                self.grid.media_playlist(variant, self.container),
            )),
            Artifact::Init(variant)
                if variant < count && self.container == HlsSegmentContainer::Fmp4 =>
            {
                self.wait_for(variant, None).await?;
                Ok(Fetched::File(self.dir.join(init_file(variant))))
            }
            Artifact::Segment(variant, index) if variant < count && index < self.grid.count() => {
                if Artifact::parse(&segment_file(variant, index, self.container)) != Some(artifact)
                {
                    return Err(FetchError::NotFound);
                }
                self.hint.store(index, Ordering::Relaxed);
                self.wait_for(variant, Some(index)).await?;
                Ok(Fetched::File(self.dir.join(segment_file(
                    variant,
                    index,
                    self.container,
                ))))
            }
            _ => Err(FetchError::NotFound),
        }
    }

    /// Wait until segment `index` of `variant` (its init segment for `None`)
    /// is on disk, starting a run for it if none will reach it soon.
    async fn wait_for(&self, variant: usize, index: Option<u32>) -> Result<(), FetchError> {
        let deadline = Instant::now() + FETCH_WAIT;
        let mut started = false;
        loop {
            if self.stopped.load(Ordering::Relaxed) {
                return Err(FetchError::Unavailable);
            }
            {
                let mut variants = self.variants.lock().await;
                let now = Instant::now();
                // A player asking this variant has left any other it was on.
                for (other, state) in variants.iter_mut().enumerate() {
                    if other != variant && now - state.last_request > SWITCH_IDLE {
                        self.stop_run(state).await;
                    }
                }
                let state = &mut variants[variant];
                state.last_request = now;
                self.refresh(variant, state).await;
                let target = index.unwrap_or_else(|| self.hint.load(Ordering::Relaxed));
                let ready = match index {
                    Some(index) => state.done.contains(&index),
                    // ffmpeg writes the init segment before its first segment.
                    None => !state.done.is_empty() && self.dir.join(init_file(variant)).exists(),
                };
                if ready {
                    return Ok(());
                }
                let reach = state.run.as_ref().map(|run| run.reach);
                let wanted = match index {
                    Some(index) => next(&state.done, reach, index),
                    None if reach.is_some() => Next::Wait,
                    None => Next::Start,
                };
                if wanted == Next::Start {
                    // One that died on the way to it won't do better a second time.
                    if started {
                        return Err(FetchError::Unavailable);
                    }
                    self.start_run(state, variant, target)
                        .await
                        .map_err(|error| {
                            tracing::error!(%error, session = %self.id, "on-demand run failed to start");
                            FetchError::Unavailable
                        })?;
                    started = true;
                }
            }
            if Instant::now() >= deadline {
                tracing::warn!(session = %self.id, variant, ?index, "segment didn't arrive in time");
                return Err(FetchError::Unavailable);
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Take in what `state`'s run has finished, and forget the run once it's over.
    async fn refresh(&self, variant: usize, state: &mut Variant) {
        let Some(run) = state.run.as_mut() else {
            return;
        };
        if let Ok(playlist) = tokio::fs::read_to_string(self.dir.join(run_playlist(variant))).await
        {
            for index in finished_segments(&playlist) {
                state.done.insert(index);
                run.reach.head = run.reach.head.max(index + 1);
            }
        }
        match self.launcher.registry.get(&run.id).await {
            Some(job) if job.status == JobStatus::Running => {
                if let Some(progress) = job.progress {
                    self.launcher
                        .registry
                        .set_progress(&self.id, progress)
                        .await;
                }
            }
            job => {
                if let Some(error) = job.and_then(|job| job.error) {
                    tracing::warn!(session = %self.id, variant, %error, "on-demand run failed");
                }
                state.run = None;
            }
        }
    }

    /// Replace `state`'s run with one from segment `index` up to the next
    /// segment already done, so finished work isn't encoded twice.
    async fn start_run(
        &self,
        state: &mut Variant,
        variant: usize,
        index: u32,
    ) -> anyhow::Result<()> {
        self.stop_run(state).await;
        let end = state
            .done
            .range(index + 1..)
            .next()
            .copied()
            .unwrap_or_else(|| self.grid.count());
        let start_secs = self.grid.start_secs(index);
        let mut request = state.template.clone();
        request.output = self.dir.join(run_playlist(variant));
        request.segment.start_number = Some(index);
        request.segment.duration_secs = self.grid.segment_secs;
        request.segment_stem = Some(stem(variant));
        request.start_secs = (index > 0).then_some(start_secs);
        request.duration_secs =
            Some(self.grid.start_secs(end).min(self.grid.duration_secs) - start_secs);
        // The last run's list would read as this one's.
        let _ = tokio::fs::remove_file(&request.output).await;
        let id = uuid::Uuid::now_v7().to_string();
        self.launcher
            .launch(id.clone(), self.media, request, self.dir.clone())
            .await?;
        state.run = Some(Run {
            id,
            reach: Reach {
                start: index,
                end,
                head: index,
            },
        });
        Ok(())
    }

    async fn stop_run(&self, state: &mut Variant) {
        if let Some(run) = state.run.take() {
            self.launcher.cancel(&run.id).await;
        }
    }

    /// Stop every run while nobody is asking for anything.
    async fn pause_if_idle(&self) {
        let mut variants = self.variants.lock().await;
        let now = Instant::now();
        if variants
            .iter()
            .all(|state| now - state.last_request > PAUSED_IDLE)
        {
            for state in variants.iter_mut() {
                self.stop_run(state).await;
            }
        }
    }

    /// Stop for good; the caller removes the directory.
    pub(crate) async fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let mut variants = self.variants.lock().await;
        for state in variants.iter_mut() {
            self.stop_run(state).await;
        }
    }

    pub(crate) fn dir(&self) -> &std::path::Path {
        &self.dir
    }
}

async fn reap(job: Weak<SegmentedJob>) {
    loop {
        tokio::time::sleep(REAP_EVERY).await;
        let Some(job) = job.upgrade() else {
            return;
        };
        if job.stopped.load(Ordering::Relaxed) {
            return;
        }
        job.pause_if_idle().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: SegmentGrid = SegmentGrid {
        duration_secs: 10.5,
        segment_secs: 4,
    };

    #[test]
    fn grid_covers_the_title_with_a_short_last_segment() {
        assert_eq!(GRID.count(), 3);
        assert_eq!(GRID.length_secs(2), 2.5);
        assert_eq!(GRID.index_at(9.0), 2);
        assert_eq!(GRID.index_at(99.0), 2);
        let playlist = GRID.media_playlist(1, HlsSegmentContainer::Fmp4);
        assert!(playlist.contains("#EXT-X-PLAYLIST-TYPE:VOD"));
        assert!(playlist.contains("#EXT-X-MAP:URI=\"v1_init.mp4\""));
        assert!(playlist.contains("#EXTINF:2.500000,\nv1_00002.m4s\n#EXT-X-ENDLIST"));
        assert!(
            !GRID
                .media_playlist(0, HlsSegmentContainer::Ts)
                .contains("EXT-X-MAP")
        );
    }

    #[test]
    fn artifacts_parse_only_what_a_player_may_ask_for() {
        assert_eq!(Artifact::parse("v0.m3u8"), Some(Artifact::Playlist(0)));
        assert_eq!(Artifact::parse("v2_init.mp4"), Some(Artifact::Init(2)));
        assert_eq!(
            Artifact::parse("v1_00042.m4s"),
            Some(Artifact::Segment(1, 42))
        );
        assert_eq!(
            Artifact::parse("v1_123456.ts"),
            Some(Artifact::Segment(1, 123456))
        );
        for other in [
            "v0_run.m3u8",
            "index.m3u8",
            "v+1.m3u8",
            "v1_-2.m4s",
            "v_1.m4s",
            "v1_init.m4s",
        ] {
            assert_eq!(Artifact::parse(other), None, "{other}");
        }
    }

    #[test]
    fn a_run_is_waited_for_only_where_it_will_soon_be() {
        let done: BTreeSet<u32> = [3, 4, 20].into();
        let run = Reach {
            start: 3,
            end: 20,
            head: 5,
        };
        assert_eq!(next(&done, Some(run), 4), Next::Ready);
        assert_eq!(next(&done, Some(run), 7), Next::Wait);
        assert_eq!(next(&done, Some(run), 5 + AHEAD), Next::Start);
        // Before the run, or past where it stops, nothing is coming.
        assert_eq!(next(&done, Some(run), 2), Next::Start);
        assert_eq!(next(&done, Some(Reach { end: 6, ..run }), 6), Next::Start);
        assert_eq!(next(&done, None, 5), Next::Start);
    }

    #[test]
    fn a_runs_playlist_lists_what_it_finished() {
        let playlist = "#EXTM3U\n#EXT-X-MAP:URI=\"v0_init.mp4\"\n#EXTINF:4.004,\nv0_00007.m4s\n#EXTINF:4.004,\nv0_00008.m4s\n";
        assert_eq!(finished_segments(playlist).collect::<Vec<_>>(), [7, 8]);
    }
}
