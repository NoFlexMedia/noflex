// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration as StdDuration,
};

use noflex_graphql::ExportEventBroadcaster;
use noflex_storage::{
    Storage,
    domain::{ExportFormat, ExportJobStatus},
    error::StorageResult,
    store::ExportJobLease,
};
use time::{Duration, OffsetDateTime};
use tokio::{
    io::{AsyncBufReadExt, BufReader, Lines},
    process::{ChildStderr, Command},
    task::JoinHandle,
};

const LEASE_DURATION: Duration = Duration::seconds(15);
const HEARTBEAT_INTERVAL: StdDuration = StdDuration::from_secs(3);
const PROGRESS_WRITE_INTERVAL: StdDuration = StdDuration::from_secs(1);
const MAX_ATTEMPTS: u32 = 3;

pub struct DownloadWorker {
    storage: Arc<dyn Storage>,
    ffmpeg_path: PathBuf,
    output_dir: PathBuf,
    events: Option<Arc<ExportEventBroadcaster>>,
    worker_id: String,
}

impl DownloadWorker {
    pub fn new(storage: Arc<dyn Storage>, ffmpeg_path: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            storage,
            ffmpeg_path,
            output_dir,
            events: None,
            worker_id: uuid::Uuid::now_v7().to_string(),
        }
    }

    pub fn with_events(mut self, events: Arc<ExportEventBroadcaster>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn spawn(&self) -> JoinHandle<()> {
        let mut worker = Self {
            storage: self.storage.clone(),
            ffmpeg_path: self.ffmpeg_path.clone(),
            output_dir: self.output_dir.clone(),
            events: self.events.clone(),
            worker_id: self.worker_id.clone(),
        };
        tokio::spawn(async move {
            loop {
                match worker.tick().await {
                    Ok(true) => continue,
                    Ok(false) => tokio::time::sleep(StdDuration::from_millis(500)).await,
                    Err(error) => {
                        tracing::warn!(%error, "download worker tick error");
                        tokio::time::sleep(StdDuration::from_secs(1)).await;
                    }
                }
            }
        })
    }

    /// Claims and processes at most one globally queued job.
    pub async fn tick(&mut self) -> StorageResult<bool> {
        let now = OffsetDateTime::now_utc();
        let token = uuid::Uuid::now_v7().to_string();
        let Some(lease) = self
            .storage
            .claim_export_job(
                &self.worker_id,
                &token,
                now,
                now + LEASE_DURATION,
                MAX_ATTEMPTS,
            )
            .await?
        else {
            return Ok(false);
        };
        self.emit_status(lease.job.id, ExportJobStatus::Running, lease.job.progress);
        self.run_job(lease).await?;
        Ok(true)
    }

    async fn run_job(&self, lease: ExportJobLease) -> StorageResult<()> {
        let job = &lease.job;
        let media_files = self.storage.media_files_for_edition(job.edition_id).await?;
        let Some(media_file) = media_files.into_iter().next() else {
            self.fail_job(&lease, "edition has no media file").await?;
            return Ok(());
        };
        let runtime_ms = self
            .storage
            .get_edition(job.edition_id)
            .await?
            .and_then(|edition| edition.runtime_ms);

        if let Err(error) = tokio::fs::create_dir_all(&self.output_dir).await {
            self.fail_job(&lease, &format!("create output directory failed: {error}"))
                .await?;
            return Ok(());
        }
        let extension = match job.format {
            ExportFormat::Mp4 => "mp4",
            ExportFormat::Mkv => "mkv",
        };
        let stem = format!("{}.{}", job.id, lease.lease_token);
        let output = self.output_dir.join(format!("{stem}.{extension}"));
        let temporary = self.output_dir.join(format!(".{stem}.part.{extension}"));

        let mut command = Command::new(&self.ffmpeg_path);
        command
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-nostdin")
            .arg("-y")
            .arg("-i")
            .arg(&media_file.path)
            .arg("-c")
            .arg("copy");
        if job.format == ExportFormat::Mp4 {
            command.arg("-movflags").arg("+faststart");
        }
        command
            .arg("-progress")
            .arg("pipe:2")
            .arg("-nostats")
            .arg(&temporary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            // `?` early-returns from the select loop below would
            // otherwise orphan a running ffmpeg.
            .kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.fail_job(&lease, &format!("ffmpeg spawn failed: {error}"))
                    .await?;
                return Ok(());
            }
        };
        let mut stderr = child
            .stderr
            .take()
            .map(|stderr| BufReader::new(stderr).lines());
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_progress_write = tokio::time::Instant::now() - PROGRESS_WRITE_INTERVAL;
        let mut progress = job.progress;

        let outcome = loop {
            tokio::select! {
                status = child.wait() => break status.map_err(|error| format!("ffmpeg wait failed: {error}")),
                _ = heartbeat.tick() => {
                    match self.renew(&lease, progress).await? {
                        Some(ExportJobStatus::Running) => {}
                        Some(ExportJobStatus::CancelRequested) => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            let _ = tokio::fs::remove_file(&temporary).await;
                            if self.storage.cancel_claimed_export_job(job.id, &lease.lease_token).await? {
                                self.emit_status(job.id, ExportJobStatus::Cancelled, progress);
                            }
                            return Ok(());
                        }
                        _ => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            let _ = tokio::fs::remove_file(&temporary).await;
                            return Ok(());
                        }
                    }
                }
                line = next_progress_line(&mut stderr), if stderr.is_some() => {
                    match line {
                        Ok(Some(line)) => {
                            if let Some(value) = parse_progress(&line, runtime_ms) {
                                progress = value;
                                if last_progress_write.elapsed() >= PROGRESS_WRITE_INTERVAL {
                                    match self.renew(&lease, progress).await? {
                                        Some(ExportJobStatus::Running) => {
                                            self.emit_status(job.id, ExportJobStatus::Running, progress);
                                            last_progress_write = tokio::time::Instant::now();
                                        }
                                        Some(ExportJobStatus::CancelRequested) => {}
                                        _ => {
                                            let _ = child.kill().await;
                                            let _ = child.wait().await;
                                            let _ = tokio::fs::remove_file(&temporary).await;
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) | Err(_) => stderr = None,
                    }
                }
            }
        };

        match outcome {
            Ok(status) if status.success() => {
                if tokio::fs::rename(&temporary, &output).await.is_err() {
                    self.fail_job(&lease, "could not publish ffmpeg output")
                        .await?;
                    let _ = tokio::fs::remove_file(&temporary).await;
                    return Ok(());
                }
                let output_path = output.to_string_lossy();
                if self
                    .storage
                    .complete_export_job(job.id, &lease.lease_token, &output_path)
                    .await?
                {
                    self.emit_status(job.id, ExportJobStatus::Completed, 1.0);
                } else {
                    let _ = tokio::fs::remove_file(&output).await;
                    if self
                        .storage
                        .cancel_claimed_export_job(job.id, &lease.lease_token)
                        .await?
                    {
                        self.emit_status(job.id, ExportJobStatus::Cancelled, progress);
                    }
                }
            }
            Ok(status) => {
                self.fail_job(&lease, &format!("ffmpeg exited with status {status}"))
                    .await?;
                let _ = tokio::fs::remove_file(&temporary).await;
            }
            Err(error) => {
                self.fail_job(&lease, &error).await?;
                let _ = tokio::fs::remove_file(&temporary).await;
            }
        }
        Ok(())
    }

    async fn renew(
        &self,
        lease: &ExportJobLease,
        progress: f32,
    ) -> StorageResult<Option<ExportJobStatus>> {
        self.storage
            .update_export_job_progress(
                lease.job.id,
                &lease.lease_token,
                progress,
                OffsetDateTime::now_utc() + LEASE_DURATION,
            )
            .await
    }

    async fn fail_job(&self, lease: &ExportJobLease, message: &str) -> StorageResult<()> {
        if self
            .storage
            .fail_export_job(lease.job.id, &lease.lease_token, message)
            .await?
        {
            self.emit_status(lease.job.id, ExportJobStatus::Failed, lease.job.progress);
        } else if self
            .storage
            .cancel_claimed_export_job(lease.job.id, &lease.lease_token)
            .await?
        {
            self.emit_status(lease.job.id, ExportJobStatus::Cancelled, lease.job.progress);
        }
        Ok(())
    }

    fn emit_status(&self, id: noflex_core::ExportJobId, status: ExportJobStatus, progress: f32) {
        if let Some(events) = &self.events {
            events.emit_status(id, status, progress, None);
        }
    }

    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

async fn next_progress_line(
    stderr: &mut Option<Lines<BufReader<ChildStderr>>>,
) -> std::io::Result<Option<String>> {
    match stderr {
        Some(lines) => lines.next_line().await,
        None => Ok(None),
    }
}

fn parse_progress(line: &str, runtime_ms: Option<i64>) -> Option<f32> {
    let runtime_ms = u64::try_from(runtime_ms?).ok()?;
    if runtime_ms == 0 {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let elapsed_ms = match key {
        "out_time_ms" | "out_time_us" => value.parse::<u64>().ok()? / 1_000,
        "out_time" => parse_clock_ms(value)?,
        _ => return None,
    };
    Some((elapsed_ms as f64 / runtime_ms as f64).clamp(0.0, 0.99) as f32)
}

fn parse_clock_ms(value: &str) -> Option<u64> {
    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<u64>().ok()?;
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(((hours * 3_600 + minutes * 60) as f64 * 1_000.0 + seconds * 1_000.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_progress() {
        assert_eq!(
            parse_progress("out_time_us=5000000", Some(10_000)),
            Some(0.5)
        );
        assert_eq!(parse_progress("progress=continue", Some(10_000)), None);
    }
}
