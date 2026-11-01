// SPDX-License-Identifier: AGPL-3.0-only

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::Context as _;
use async_trait::async_trait;
use noflex_core::ids::MediaFileId;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStderr, ChildStdout},
    sync::{Mutex, watch},
};

use crate::{JobHandle, TranscodeRequest, Transcoder};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
}

/// Latest ffmpeg `-progress` block.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JobProgress {
    /// Output timestamp reached.
    pub position_secs: f64,
    /// Share of the media processed. Only computable when the
    /// request carries a duration — ffmpeg reports the output
    /// timestamp, not the source length.
    pub percent: Option<f64>,
    /// Encoded frames per second; `None` before the first frame or
    /// for audio-only output.
    pub fps: Option<f64>,
    /// Realtime multiple (`1.0` = realtime).
    pub speed: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct TranscodeJob {
    pub id: String,
    pub media: MediaFileId,
    pub request: TranscodeRequest,
    pub status: JobStatus,
    pub progress: Option<JobProgress>,
    pub error: Option<String>,
    pub output_dir: PathBuf,
}

#[derive(Default, Clone)]
pub struct TranscodeJobRegistry {
    inner: Arc<Mutex<HashMap<String, TranscodeJob>>>,
}

impl TranscodeJobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, job: TranscodeJob) {
        let mut g = self.inner.lock().await;
        g.insert(job.id.clone(), job);
    }

    pub async fn set_progress(&self, id: &str, progress: JobProgress) {
        let mut g = self.inner.lock().await;
        if let Some(j) = g.get_mut(id) {
            j.progress = Some(progress);
        }
    }

    /// Terminal transition: Running → Completed/Failed/Cancelled.
    /// Never applied over an already-terminal job.
    pub async fn finish(&self, id: &str, status: JobStatus, error: Option<String>) {
        let mut g = self.inner.lock().await;
        if let Some(j) = g.get_mut(id)
            && j.status == JobStatus::Running
        {
            j.status = status;
            j.error = error;
        }
    }

    pub async fn get(&self, id: &str) -> Option<TranscodeJob> {
        let g = self.inner.lock().await;
        g.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) -> Option<TranscodeJob> {
        let mut g = self.inner.lock().await;
        g.remove(id)
    }
}

pub struct FfmpegJobManager {
    ffmpeg_path: PathBuf,
    cache_dir: PathBuf,
    registry: TranscodeJobRegistry,
    cancellations: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

impl FfmpegJobManager {
    pub fn new(ffmpeg_path: impl Into<PathBuf>, cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg_path: ffmpeg_path.into(),
            cache_dir: cache_dir.into(),
            registry: TranscodeJobRegistry::new(),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn registry(&self) -> &TranscodeJobRegistry {
        &self.registry
    }

    /// The ffmpeg binary jobs spawn; other one-shot ffmpeg work (e.g.
    /// subtitle extraction) reuses it so overrides apply everywhere.
    pub fn ffmpeg_path(&self) -> &std::path::Path {
        &self.ffmpeg_path
    }

    fn next_id() -> String {
        uuid::Uuid::now_v7().to_string()
    }
}

#[async_trait]
impl Transcoder for FfmpegJobManager {
    async fn start_job(
        &self,
        media: MediaFileId,
        request: TranscodeRequest,
    ) -> anyhow::Result<JobHandle> {
        let id = Self::next_id();
        let output_dir = self.cache_dir.join(&id);
        tokio::fs::create_dir_all(&output_dir)
            .await
            .with_context(|| format!("create job dir {}", output_dir.display()))?;
        // A relative output resolves inside the job's own dir so
        // stream endpoints can pass "index.m3u8" without knowing
        // the cache layout; absolute paths stay caller-owned.
        let mut request = request;
        if request.output.is_relative() {
            request.output = output_dir.join(&request.output);
        }
        self.registry
            .register(TranscodeJob {
                id: id.clone(),
                media,
                request: request.clone(),
                status: JobStatus::Running,
                progress: None,
                error: None,
                output_dir: output_dir.clone(),
            })
            .await;
        let handle = JobHandle {
            id: id.clone(),
            output_dir,
        };
        let ffmpeg = self.ffmpeg_path.to_string_lossy().into_owned();
        let spawned = match crate::spawn::spawn(&request, &ffmpeg, handle.clone()).await {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("spawn {}: {e}", self.ffmpeg_path.display());
                self.registry
                    .finish(&id, JobStatus::Failed, Some(msg.clone()))
                    .await;
                return Err(anyhow::anyhow!(msg));
            }
        };
        if let Some(child) = spawned.into_child() {
            let (cancel_tx, cancel_rx) = watch::channel(false);
            self.cancellations
                .lock()
                .await
                .insert(id.clone(), cancel_tx);
            tokio::spawn(run_job(
                child,
                self.registry.clone(),
                Arc::clone(&self.cancellations),
                id,
                request.duration_secs,
                cancel_rx,
            ));
        }
        Ok(handle)
    }

    async fn stop_job(&self, job: &JobHandle) -> anyhow::Result<()> {
        let running = self
            .registry
            .get(&job.id)
            .await
            .is_some_and(|j| j.status == JobStatus::Running);
        if running && let Some(tx) = self.cancellations.lock().await.get(&job.id) {
            // The driver flips the status to Cancelled once the
            // process is actually dead; terminal jobs keep theirs.
            let _ = tx.send(true);
        }
        Ok(())
    }
}

/// Owns the child for the rest of its life: pipes stderr
/// progress into the registry, answers cancellation, and maps
/// the exit status onto the job's terminal state.
async fn run_job(
    mut child: Child,
    registry: TranscodeJobRegistry,
    cancellations: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    job_id: String,
    duration_secs: Option<f64>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let tail = Arc::new(Mutex::new(Vec::new()));
    let reader = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(read_stderr_tail(stderr, Arc::clone(&tail))));
    let progress = child.stdout.take().map(|stdout| {
        tokio::spawn(read_progress(
            stdout,
            registry.clone(),
            job_id.clone(),
            duration_secs,
        ))
    });

    let mut cancelled = false;
    let wait_result = tokio::select! {
        _ = cancel_rx.changed() => {
            cancelled = true;
            // May already have exited between the select and
            // here; the wait below settles it either way.
            let _ = child.start_kill();
            child.wait().await
        }
        result = child.wait() => result,
    };

    if let Some(reader) = reader {
        // stderr hits EOF when the process dies, so the reader
        // ends on its own — but only once every holder of the
        // pipe's write end (grandchildren included) is gone, so
        // don't wait forever. A bounded wait still lets the
        // reader drain the final chunks the error tail below is
        // built from; aborting immediately could preempt that.
        let _ = tokio::time::timeout(STDERR_DRAIN_TIMEOUT, reader).await;
    }
    if let Some(progress) = progress {
        // The final block only matters while the job is live; don't
        // hold the terminal transition hostage to it.
        progress.abort();
    }

    let (status, error) = match wait_result {
        Err(e) => (JobStatus::Failed, Some(format!("wait on ffmpeg: {e}"))),
        Ok(_) if cancelled => (JobStatus::Cancelled, None),
        Ok(exit) if exit.success() => (JobStatus::Completed, None),
        Ok(exit) => {
            let stderr_tail = String::from_utf8_lossy(&tail.lock().await)
                .trim()
                .to_string();
            let msg = if stderr_tail.is_empty() {
                format!("ffmpeg exited with {exit}")
            } else {
                format!("ffmpeg exited with {exit}: {stderr_tail}")
            };
            (JobStatus::Failed, Some(msg))
        }
    };
    registry.finish(&job_id, status, error).await;
    cancellations.lock().await.remove(&job_id);
}

/// How much of ffmpeg's stderr to retain for error reporting.
const STDERR_TAIL_CAP: usize = 8 * 1024;

/// Grace period for the stderr reader to reach EOF after the
/// child exits. Real ffmpeg closes its end at death; shell
/// wrappers can leave grandchildren holding the pipe.
const STDERR_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// Keep a rolling tail of stderr for the failure message.
async fn read_stderr_tail(stderr: ChildStderr, tail: Arc<Mutex<Vec<u8>>>) {
    let mut chunks = BufReader::new(stderr).split(b'\n');
    while let Ok(Some(mut chunk)) = chunks.next_segment().await {
        chunk.push(b'\n');
        let mut t = tail.lock().await;
        t.extend_from_slice(&chunk);
        if t.len() > STDERR_TAIL_CAP {
            let excess = t.len() - STDERR_TAIL_CAP;
            t.drain(..excess);
        }
    }
}

/// Forward ffmpeg's `-progress` blocks into the registry: `key=value`
/// lines, each block terminated by `progress=continue|end`.
async fn read_progress(
    stdout: ChildStdout,
    registry: TranscodeJobRegistry,
    job_id: String,
    duration_secs: Option<f64>,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut parser = ProgressParser::default();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(mut progress) = parser.feed(&line) {
            progress.percent = duration_secs
                .filter(|d| *d > 0.0)
                .map(|d| (progress.position_secs / d * 100.0).clamp(0.0, 100.0));
            registry.set_progress(&job_id, progress).await;
        }
    }
}

#[derive(Default)]
struct ProgressParser {
    current: JobProgress,
}

impl ProgressParser {
    /// Feed one line; returns the completed block on its `progress=` line.
    fn feed(&mut self, line: &str) -> Option<JobProgress> {
        let (key, value) = line.trim().split_once('=')?;
        let value = value.trim();
        match key {
            "fps" => self.current.fps = value.parse().ok().filter(|f: &f64| *f > 0.0),
            "speed" => {
                self.current.speed = value.trim_end_matches('x').trim().parse().ok();
            }
            // Despite the name, `out_time_ms` is microseconds too.
            "out_time_us" | "out_time_ms" => {
                if let Ok(us) = value.parse::<i64>()
                    && us >= 0
                {
                    self.current.position_secs = us as f64 / 1_000_000.0;
                }
            }
            "out_time" => {
                if let Some(secs) = parse_timestamp(value) {
                    self.current.position_secs = secs;
                }
            }
            "progress" => return Some(std::mem::take(&mut self.current)),
            _ => {}
        }
        None
    }
}

fn parse_timestamp(s: &str) -> Option<f64> {
    let (hours, rest) = s.split_once(':')?;
    let (minutes, seconds) = rest.split_once(':')?;
    Some(
        hours.trim().parse::<f64>().ok()? * 3600.0
            + minutes.parse::<f64>().ok()? * 60.0
            + seconds.parse::<f64>().ok()?,
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use noflex_core::{AudioCodec, VideoCodec};

    use super::*;
    use crate::{
        Encoder,
        decision::{AudioPlan, HlsSegmentContainer, TranscodePlan, VideoPlan},
        ffmpeg_argv::{LadderEntry, TranscodeInput},
    };

    fn make_request(input: PathBuf, output: PathBuf) -> TranscodeRequest {
        TranscodeRequest::new(
            TranscodeInput::File(input),
            output,
            TranscodePlan {
                segment_container: HlsSegmentContainer::Ts,
                video: VideoPlan::Encode {
                    codec: VideoCodec::H264,
                    max_height: 720,
                    tonemap: false,
                    profile: noflex_core::VideoProfile::H264High,
                },
                audio: AudioPlan::Encode {
                    codec: AudioCodec::Aac,
                    channels: 2,
                },
            },
            Encoder::Software264,
            LadderEntry {
                max_height: 720,
                video_codec: VideoCodec::H264,
                audio_codec: AudioCodec::Aac,
                container: "ts".to_string(),
                bitrate_kbps: 4000,
            },
        )
    }

    /// A shell script standing in for the ffmpeg binary.
    /// Returns (binary path, scratch dir).
    fn make_mock_ffmpeg(script_body: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("noflex-job-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let mock = dir.join("ffmpeg");
        std::fs::write(&mock, format!("#!/bin/sh\n{script_body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&mock).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&mock, perms).unwrap();
        }
        (mock, dir)
    }

    async fn wait_terminal(mgr: &FfmpegJobManager, id: &str, timeout: Duration) -> TranscodeJob {
        let deadline = Instant::now() + timeout;
        loop {
            let job = mgr.registry().get(id).await.expect("job registered");
            if job.status != JobStatus::Running || Instant::now() >= deadline {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    #[tokio::test]
    async fn registry_records_started_jobs() {
        let (mock, dir) = make_mock_ffmpeg("sleep 5");
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let request = make_request(dir.join("in.mp4"), dir.join("out.m3u8"));
        let handle = mgr.start_job(media, request).await.expect("start_job");
        let stored = mgr.registry().get(&handle.id).await.expect("stored");
        assert_eq!(stored.status, JobStatus::Running);
        assert_eq!(stored.media, media);
        assert!(stored.output_dir.starts_with(&dir));
        // Reap the sleeping mock so the test doesn't leak it.
        mgr.stop_job(&handle).await.expect("stop_job");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(10)).await;
        assert_eq!(job.status, JobStatus::Cancelled);
    }

    #[tokio::test]
    async fn stop_job_marks_cancelled() {
        let (mock, dir) = make_mock_ffmpeg("sleep 5");
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let handle = mgr
            .start_job(
                media,
                make_request(dir.join("in.mp4"), dir.join("out.m3u8")),
            )
            .await
            .expect("start_job");
        mgr.stop_job(&handle).await.expect("stop_job");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(10)).await;
        assert_eq!(job.status, JobStatus::Cancelled);
    }

    #[tokio::test]
    async fn stop_unknown_job_is_a_noop() {
        let (mock, dir) = make_mock_ffmpeg("exit 0");
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let bogus = JobHandle {
            id: "does-not-exist".to_string(),
            output_dir: dir.clone(),
        };
        mgr.stop_job(&bogus).await.expect("stop_job noop");
    }

    #[tokio::test]
    async fn stop_terminal_job_keeps_its_status() {
        let (mock, dir) = make_mock_ffmpeg("exit 0");
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let handle = mgr
            .start_job(
                media,
                make_request(dir.join("in.mp4"), dir.join("out.m3u8")),
            )
            .await
            .expect("start_job");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(10)).await;
        assert_eq!(job.status, JobStatus::Completed);
        mgr.stop_job(&handle).await.expect("stop_job");
        let job = mgr.registry().get(&handle.id).await.expect("stored");
        assert_eq!(job.status, JobStatus::Completed);
    }

    #[tokio::test]
    async fn progress_flows_from_stdout_into_registry() {
        let (mock, dir) = make_mock_ffmpeg(
            "printf 'frame=42\\nfps=23.5\\nout_time_us=1500000\\nspeed=1.25x\\nprogress=continue\\n'\nsleep 5",
        );
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let mut request = make_request(dir.join("in.mp4"), dir.join("out.m3u8"));
        request.duration_secs = Some(3.0);
        let handle = mgr.start_job(media, request).await.expect("start_job");
        let deadline = Instant::now() + Duration::from_secs(10);
        let progress = loop {
            let job = mgr.registry().get(&handle.id).await.expect("stored");
            if let Some(p) = job.progress {
                break p;
            }
            assert!(Instant::now() < deadline, "progress never arrived");
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert_eq!(progress.position_secs, 1.5);
        assert_eq!(progress.percent, Some(50.0));
        assert_eq!(progress.fps, Some(23.5));
        assert_eq!(progress.speed, Some(1.25));
        mgr.stop_job(&handle).await.expect("cleanup stop");
        wait_terminal(&mgr, &handle.id, Duration::from_secs(10)).await;
    }

    #[tokio::test]
    async fn failed_exit_records_stderr_tail() {
        let (mock, dir) = make_mock_ffmpeg("echo 'something exploded' >&2\nexit 1");
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let handle = mgr
            .start_job(
                media,
                make_request(dir.join("in.mp4"), dir.join("out.m3u8")),
            )
            .await
            .expect("start_job");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(10)).await;
        assert_eq!(job.status, JobStatus::Failed);
        let error = job.error.expect("error message");
        assert!(error.contains("something exploded"), "error: {error}");
    }

    #[tokio::test]
    async fn spawn_failure_marks_job_failed() {
        let dir = std::env::temp_dir().join(format!("noflex-job-test-{}", uuid::Uuid::now_v7()));
        let mgr = FfmpegJobManager::new(dir.join("missing-ffmpeg"), &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let result = mgr
            .start_job(
                media,
                make_request(dir.join("in.mp4"), dir.join("out.m3u8")),
            )
            .await;
        let err = result.expect_err("spawn should fail");
        assert!(err.to_string().contains("missing-ffmpeg"), "err: {err}");
    }

    #[tokio::test]
    async fn relative_output_resolves_into_job_dir() {
        let (mock, dir) = make_mock_ffmpeg("sleep 5");
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let request = make_request(dir.join("in.mp4"), PathBuf::from("index.m3u8"));
        let handle = mgr.start_job(media, request).await.expect("start_job");
        let stored = mgr.registry().get(&handle.id).await.expect("stored");
        assert_eq!(stored.request.output, handle.output_dir.join("index.m3u8"));
        mgr.stop_job(&handle).await.expect("cleanup stop");
        wait_terminal(&mgr, &handle.id, Duration::from_secs(10)).await;
    }

    #[tokio::test]
    async fn stop_job_kills_the_subprocess() {
        let dir = std::env::temp_dir().join(format!("noflex-job-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("ffmpeg.pid");
        let (mock, _) = make_mock_ffmpeg(&format!("echo $$ > {}\nsleep 60", pidfile.display()));
        // make_mock_ffmpeg made its own scratch dir; use it for the
        // cache but keep the pidfile in `dir` for a stable path.
        let mgr = FfmpegJobManager::new(&mock, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let handle = mgr
            .start_job(
                media,
                make_request(dir.join("in.mp4"), dir.join("out.m3u8")),
            )
            .await
            .expect("start_job");
        let pid: i32 = {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                // The shell creates the pidfile before writing it, so an
                // empty or partial read just means "not yet".
                if let Some(pid) = std::fs::read_to_string(&pidfile)
                    .ok()
                    .and_then(|s| s.trim().parse().ok())
                {
                    break pid;
                }
                assert!(Instant::now() < deadline, "pidfile never appeared");
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        mgr.stop_job(&handle).await.expect("stop_job");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(10)).await;
        assert_eq!(job.status, JobStatus::Cancelled);
        let kill = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .expect("kill binary");
        assert!(!kill.success(), "pid {pid} should be dead");
    }

    #[test]
    fn progress_parser_emits_one_block_per_progress_line() {
        let mut p = ProgressParser::default();
        for line in [
            "frame=100",
            "fps=0.00",
            "out_time=00:01:02.500000",
            "speed=N/A",
        ] {
            assert_eq!(p.feed(line), None);
        }
        let block = p.feed("progress=continue").expect("block");
        assert_eq!(block.position_secs, 62.5);
        assert_eq!(block.fps, None);
        assert_eq!(block.speed, None);
        assert_eq!(p.feed("out_time_us=-9223372036854775807"), None);
        assert_eq!(p.feed("progress=end").expect("block").position_secs, 0.0);
        assert_eq!(p.feed("garbage"), None);
    }

    fn find_ffmpeg() -> Option<PathBuf> {
        let paths = std::env::var_os("PATH")?;
        std::env::split_paths(&paths)
            .map(|p| p.join("ffmpeg"))
            .find(|p| p.is_file())
    }

    #[tokio::test]
    async fn real_ffmpeg_hls_end_to_end() {
        let Some(ffmpeg) = find_ffmpeg() else {
            eprintln!("ffmpeg not on PATH; skipping real_ffmpeg_hls_end_to_end");
            return;
        };
        let dir = std::env::temp_dir().join(format!("noflex-job-e2e-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.mp4");
        let gen_output = tokio::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2:size=320x240:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&input)
            .output()
            .await
            .expect("generate test input");
        assert!(
            gen_output.status.success(),
            "input generation failed: {}",
            String::from_utf8_lossy(&gen_output.stderr)
        );

        let mgr = FfmpegJobManager::new(&ffmpeg, &dir);
        let media = MediaFileId::from_uuid(uuid::Uuid::now_v7());
        let output = dir.join("out.m3u8");
        let mut request = make_request(input, output.clone());
        request.duration_secs = Some(2.0);
        let handle = mgr.start_job(media, request).await.expect("start_job");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(60)).await;
        assert_eq!(job.status, JobStatus::Completed, "error: {:?}", job.error);
        assert!(output.exists(), "playlist missing");
        let has_segment = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.path().extension().is_some_and(|ext| ext == "ts"));
        assert!(has_segment, "no .ts segments in {}", dir.display());
    }

    /// Generate a 2s 10-bit HDR10-tagged HEVC + stereo AAC Matroska
    /// file; `None` when this ffmpeg lacks libx265.
    async fn hevc_hdr_input(ffmpeg: &PathBuf, path: &std::path::Path) -> Option<()> {
        let out = tokio::process::Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2:size=320x240:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx265",
                "-pix_fmt",
                "yuv420p10le",
                "-x265-params",
                // libx265 only writes colour VUI from its own params.
                "log-level=error:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc",
                "-c:a",
                "aac",
                "-ac",
                "2",
            ])
            .arg(path)
            .output()
            .await
            .ok()?;
        out.status.success().then_some(())
    }

    #[tokio::test]
    async fn real_ffmpeg_fmp4_copy_and_tonemap_encode() {
        let Some(ffmpeg) = find_ffmpeg() else {
            eprintln!("ffmpeg not on PATH; skipping real_ffmpeg_fmp4_copy_and_tonemap_encode");
            return;
        };
        let dir = std::env::temp_dir().join(format!("noflex-job-fmp4-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        // A `:` in the name must not be read as a protocol prefix.
        let input = dir.join("hdr:movie.mkv");
        if hevc_hdr_input(&ffmpeg, &input).await.is_none() {
            eprintln!("ffmpeg lacks libx265; skipping");
            return;
        }
        let mgr = FfmpegJobManager::new(&ffmpeg, &dir);
        let ladder = LadderEntry {
            max_height: 240,
            video_codec: VideoCodec::H264,
            audio_codec: AudioCodec::Aac,
            container: "mp4".to_string(),
            bitrate_kbps: 500,
        };

        let copy = TranscodePlan {
            segment_container: HlsSegmentContainer::Fmp4,
            video: VideoPlan::Copy {
                codec: VideoCodec::H265,
                range: crate::decision::VideoRange::Hdr10,
            },
            audio: AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: 2,
            },
        };
        let mut request = TranscodeRequest::new(
            TranscodeInput::File(input.clone()),
            "index.m3u8",
            copy,
            Encoder::Software264,
            ladder.clone(),
        );
        request.duration_secs = Some(2.0);
        let handle = mgr
            .start_job(MediaFileId::new(), request)
            .await
            .expect("start copy");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(60)).await;
        assert_eq!(job.status, JobStatus::Completed, "error: {:?}", job.error);
        assert!(
            handle.output_dir.join("init.mp4").exists(),
            "no init segment"
        );
        let playlist = std::fs::read_to_string(handle.output_dir.join("index.m3u8")).unwrap();
        assert!(
            playlist.contains("#EXT-X-MAP:URI=\"init.mp4\""),
            "{playlist}"
        );
        assert!(playlist.contains(".m4s"), "{playlist}");
        assert!(
            job.progress.is_some_and(|p| p.position_secs > 0.0),
            "{:?}",
            job.progress
        );

        let encode = TranscodePlan {
            segment_container: HlsSegmentContainer::Ts,
            video: VideoPlan::Encode {
                codec: VideoCodec::H264,
                max_height: 240,
                tonemap: true,
                profile: noflex_core::VideoProfile::H264High,
            },
            audio: AudioPlan::Copy {
                codec: AudioCodec::Aac,
            },
        };
        let request = TranscodeRequest::new(
            TranscodeInput::File(input),
            "index.m3u8",
            encode,
            Encoder::Software264,
            ladder,
        );
        let handle = mgr
            .start_job(MediaFileId::new(), request)
            .await
            .expect("start encode");
        let job = wait_terminal(&mgr, &handle.id, Duration::from_secs(60)).await;
        assert_eq!(job.status, JobStatus::Completed, "error: {:?}", job.error);
        assert!(handle.output_dir.join("segment_00000.ts").exists());
    }
}
