// SPDX-License-Identifier: AGPL-3.0-only

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use noflex_core::{ChannelId, LiveTvSourceId, RecordingId, RecordingScheduleId, UserId};
use noflex_graphql::{LiveTvService, LiveTvServiceError, PlaybackSession};
use noflex_storage::{Recording, RecordingStatus, Storage, StorageError};
use time::OffsetDateTime;
use tokio::{
    process::Command,
    sync::{Mutex, Semaphore, watch},
};

use crate::{
    live_tv::{LiveTvError, LiveTvIngest},
    stream::ServerPlaybackSessions,
};

#[derive(Debug, thiserror::Error)]
enum RecordingPathError {
    #[error("recording has no file")]
    Missing,
    #[error("recording path is outside the recordings directory")]
    OutsideRoot,
}

pub struct ServerLiveTv {
    storage: Arc<dyn Storage>,
    ingest: LiveTvIngest,
    playback: Arc<ServerPlaybackSessions>,
    ffmpeg: PathBuf,
    recordings_dir: PathBuf,
    running: Arc<Mutex<HashMap<RecordingScheduleId, watch::Sender<bool>>>>,
    recording_slots: Arc<Semaphore>,
}

impl ServerLiveTv {
    pub fn new(
        storage: Arc<dyn Storage>,
        playback: Arc<ServerPlaybackSessions>,
        ffmpeg: PathBuf,
        data_dir: PathBuf,
        recording_concurrency: usize,
    ) -> Result<Self, LiveTvError> {
        Ok(Self {
            ingest: LiveTvIngest::new(storage.clone())?,
            storage,
            playback,
            ffmpeg,
            recordings_dir: data_dir.join("recordings"),
            running: Arc::new(Mutex::new(HashMap::new())),
            recording_slots: Arc::new(Semaphore::new(recording_concurrency.max(1))),
        })
    }

    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Err(error) = tokio::fs::create_dir_all(&self.recordings_dir).await {
                tracing::error!(%error,"create recordings directory failed");
                return;
            }
            if let Err(error) = self
                .storage
                .fail_running_recordings(
                    OffsetDateTime::now_utc(),
                    "server restarted while recording",
                )
                .await
            {
                tracing::error!(%error,"recording restart reconciliation failed")
            }
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let Err(error) = self.poll_once().await {
                    tracing::error!(%error,"recording worker poll failed")
                }
            }
        })
    }

    pub fn spawn_guide_refresh(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
            loop {
                interval.tick().await;
                match self.storage.list_live_tv_sources().await {
                    Ok(sources) => {
                        for source in sources.into_iter().filter(|s| s.enabled) {
                            if let Err(error) = self.refresh_source(source.id).await {
                                tracing::warn!(source=%source.id,error=?error,"live TV guide refresh failed")
                            }
                        }
                    }
                    Err(error) => tracing::warn!(%error,"listing live TV sources failed"),
                }
            }
        })
    }

    #[doc(hidden)]
    pub async fn poll_once(&self) -> Result<(), StorageError> {
        while let Some((schedule, recording)) = self
            .storage
            .claim_due_recording(OffsetDateTime::now_utc())
            .await?
        {
            let Some(channel) = self
                .storage
                .get_live_tv_channel(schedule.channel_id)
                .await?
            else {
                // Already claimed as Running; without a terminal state it
                // would never be retried or reconciled until restart.
                self.storage
                    .finish_recording(
                        recording.id,
                        RecordingStatus::Failed,
                        None,
                        Some("channel not found"),
                        None,
                        OffsetDateTime::now_utc(),
                    )
                    .await?;
                continue;
            };
            let ffmpeg = self.ffmpeg.clone();
            let dir = self.recordings_dir.clone();
            let storage = self.storage.clone();
            let running = self.running.clone();
            let recording_slots = self.recording_slots.clone();
            tokio::spawn(async move {
                run_recording(
                    storage,
                    running,
                    recording_slots,
                    (ffmpeg, dir),
                    recording,
                    schedule,
                    channel.stream_url,
                )
                .await;
            });
        }
        Ok(())
    }

    async fn owner_for_schedule(
        &self,
        principal: UserId,
        id: RecordingScheduleId,
    ) -> Result<UserId, LiveTvServiceError> {
        let schedule = self
            .storage
            .get_recording_schedule(id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("schedule"))?;
        let admin = self
            .storage
            .get_user(principal)
            .await
            .map_err(LiveTvServiceError::internal)?
            .is_some_and(|u| u.is_admin);
        if !admin && schedule.owner_id != principal {
            return Err(LiveTvServiceError::NotFound("schedule"));
        }
        Ok(schedule.owner_id)
    }
    async fn owner_for_recording(
        &self,
        principal: UserId,
        id: RecordingId,
    ) -> Result<UserId, LiveTvServiceError> {
        let recording = self
            .storage
            .get_recording(id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("recording"))?;
        let admin = self
            .storage
            .get_user(principal)
            .await
            .map_err(LiveTvServiceError::internal)?
            .is_some_and(|u| u.is_admin);
        if !admin && recording.owner_id != principal {
            return Err(LiveTvServiceError::NotFound("recording"));
        }
        Ok(recording.owner_id)
    }

    fn recording_path(&self, recording: &Recording) -> Result<PathBuf, RecordingPathError> {
        let path = recording
            .path
            .as_deref()
            .ok_or(RecordingPathError::Missing)?;
        let expected = self.recordings_dir.join(format!("{}.mkv", recording.id));
        if std::path::Path::new(path) != expected.as_path() {
            return Err(RecordingPathError::OutsideRoot);
        }
        Ok(expected)
    }
}

#[async_trait::async_trait]
impl LiveTvService for ServerLiveTv {
    async fn refresh_source(&self, source: LiveTvSourceId) -> Result<(), LiveTvServiceError> {
        let now = OffsetDateTime::now_utc();
        self.ingest
            .refresh(
                source,
                now - Duration::from_secs(24 * 60 * 60),
                now + Duration::from_secs(14 * 24 * 60 * 60),
            )
            .await
            .map_err(LiveTvServiceError::internal)?;
        Ok(())
    }
    async fn start_channel(
        &self,
        owner: UserId,
        channel: ChannelId,
    ) -> Result<PlaybackSession, LiveTvServiceError> {
        let channel = self
            .storage
            .get_live_tv_channel(channel)
            .await
            .map_err(LiveTvServiceError::internal)?
            .filter(|c| c.enabled)
            .ok_or(LiveTvServiceError::NotFound("channel"))?;
        self.playback
            .start_live(owner, channel.id, &channel.stream_url)
            .await
            .map_err(LiveTvServiceError::internal)
    }
    async fn cancel_schedule(
        &self,
        principal: UserId,
        id: RecordingScheduleId,
    ) -> Result<noflex_storage::RecordingSchedule, LiveTvServiceError> {
        let owner = self.owner_for_schedule(principal, id).await?;
        let schedule = self
            .storage
            .cancel_recording_schedule(id, owner, OffsetDateTime::now_utc())
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("schedule"))?;
        if let Some(sender) = self.running.lock().await.get(&id) {
            let _ = sender.send(true);
        }
        Ok(schedule)
    }
    async fn delete_recording(
        &self,
        principal: UserId,
        id: RecordingId,
        delete_file: bool,
    ) -> Result<Recording, LiveTvServiceError> {
        let owner = self.owner_for_recording(principal, id).await?;
        let recording = self
            .storage
            .get_recording(id)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("recording"))?;
        if matches!(
            recording.status,
            RecordingStatus::Scheduled | RecordingStatus::Running
        ) {
            return Err(LiveTvServiceError::RecordingActive);
        }
        if delete_file && recording.path.is_some() {
            let path = self
                .recording_path(&recording)
                .map_err(LiveTvServiceError::internal)?;
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(LiveTvServiceError::internal(error)),
            }
        }
        self.storage
            .delete_recording(id, owner)
            .await
            .map_err(LiveTvServiceError::internal)?
            .ok_or(LiveTvServiceError::NotFound("recording"))
    }
}

async fn run_recording(
    storage: Arc<dyn Storage>,
    running: Arc<Mutex<HashMap<RecordingScheduleId, watch::Sender<bool>>>>,
    recording_slots: Arc<Semaphore>,
    paths: (PathBuf, PathBuf),
    recording: Recording,
    schedule: noflex_storage::RecordingSchedule,
    input: String,
) {
    let (ffmpeg, dir) = paths;
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    running.lock().await.insert(schedule.id, cancel_tx);
    let final_path = dir.join(format!("{}.mkv", recording.id));
    let temp_path = dir.join(format!("{}.part.mkv", recording.id));
    let _permit = tokio::select! {
        permit = recording_slots.acquire_owned() => match permit {
            Ok(permit) => permit,
            Err(_) => {
                running.lock().await.remove(&schedule.id);
                return;
            }
        },
        _ = cancel_rx.changed() => {
            finish_cancelled(&running, schedule.id, &temp_path, &final_path).await;
            return;
        }
    };
    if !storage
        .get_recording(recording.id)
        .await
        .ok()
        .flatten()
        .is_some_and(|row| row.status == RecordingStatus::Running)
    {
        finish_cancelled(&running, schedule.id, &temp_path, &final_path).await;
        return;
    }
    let now = OffsetDateTime::now_utc();
    let start = schedule.start_at - Duration::from_secs(u64::from(schedule.padding_before_secs));
    if start > now {
        tokio::select! {_ = tokio::time::sleep((start-now).unsigned_abs())=>{},_ = cancel_rx.changed()=>{finish_cancelled(&running,schedule.id,&temp_path,&final_path).await;return}}
    }
    let end = schedule.end_at + Duration::from_secs(u64::from(schedule.padding_after_secs));
    let duration = (end - OffsetDateTime::now_utc()).as_seconds_f64().max(0.0);
    if duration <= 0.0 {
        let _ = storage
            .finish_recording(
                recording.id,
                RecordingStatus::Failed,
                None,
                Some("recording window elapsed before start"),
                None,
                OffsetDateTime::now_utc(),
            )
            .await;
        running.lock().await.remove(&schedule.id);
        return;
    }
    let mut child = match Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            // Tuner URLs are admin-configured but resolved by ffmpeg;
            // never let a playlist pivot into file:, concat:, etc.
            "-protocol_whitelist",
            noflex_transcoder::NETWORK_PROTOCOL_WHITELIST,
            "-i",
            &input,
            "-t",
            &duration.to_string(),
            "-map",
            "0",
            "-c",
            "copy",
        ])
        .arg(&temp_path)
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("spawn {}: {error}", ffmpeg.display()))
    {
        Ok(child) => child,
        Err(error) => {
            let _ = storage
                .finish_recording(
                    recording.id,
                    RecordingStatus::Failed,
                    None,
                    Some(&error.to_string()),
                    None,
                    OffsetDateTime::now_utc(),
                )
                .await;
            running.lock().await.remove(&schedule.id);
            return;
        }
    };
    let cancelled;
    let result = tokio::select! {result=child.wait()=>{cancelled=false;result},_=cancel_rx.changed()=>{cancelled=true;let _=child.start_kill();child.wait().await}};
    if cancelled {
        finish_cancelled(&running, schedule.id, &temp_path, &final_path).await;
        return;
    }
    match result {
        Ok(status) if status.success() => match tokio::fs::rename(&temp_path, &final_path).await {
            Ok(()) => {
                let size = tokio::fs::metadata(&final_path).await.ok().map(|m| m.len());
                let path = final_path.to_string_lossy();
                let completed = storage
                    .finish_recording(
                        recording.id,
                        RecordingStatus::Completed,
                        Some(&path),
                        None,
                        size,
                        OffsetDateTime::now_utc(),
                    )
                    .await
                    .unwrap_or(false);
                if !completed {
                    let _ = tokio::fs::remove_file(&final_path).await;
                }
            }
            Err(error) => {
                let _ = storage
                    .finish_recording(
                        recording.id,
                        RecordingStatus::Failed,
                        None,
                        Some(&error.to_string()),
                        None,
                        OffsetDateTime::now_utc(),
                    )
                    .await;
            }
        },
        Ok(status) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            let _ = storage
                .finish_recording(
                    recording.id,
                    RecordingStatus::Failed,
                    None,
                    Some(&format!("ffmpeg exited with {status}")),
                    None,
                    OffsetDateTime::now_utc(),
                )
                .await;
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            let _ = storage
                .finish_recording(
                    recording.id,
                    RecordingStatus::Failed,
                    None,
                    Some(&error.to_string()),
                    None,
                    OffsetDateTime::now_utc(),
                )
                .await;
        }
    }
    running.lock().await.remove(&schedule.id);
}

async fn finish_cancelled(
    running: &Mutex<HashMap<RecordingScheduleId, watch::Sender<bool>>>,
    schedule: RecordingScheduleId,
    temp: &std::path::Path,
    final_path: &std::path::Path,
) {
    let _ = tokio::fs::remove_file(temp).await;
    let _ = tokio::fs::remove_file(final_path).await;
    running.lock().await.remove(&schedule);
}
