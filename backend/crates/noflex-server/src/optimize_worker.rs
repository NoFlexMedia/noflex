// SPDX-License-Identifier: AGPL-3.0-only
//
// Prepares optimized versions in the background: works out what each user is
// likely to watch next, plans versions for their devices, and encodes them one
// at a time while nobody is watching anything.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use noflex_core::{MediaFileId, OptimizedVersionId, UserId};
use noflex_graphql::{
    PlaybackSessionService,
    smart::{LikelyOptions, likely_watched, marked_works, playable_sources},
};
use noflex_plugin_host::PluginRegistry;
use noflex_storage::{
    RequestUser, Storage,
    domain::{
        Device, MediaFile, OptimizedVersion, OptimizedVersionStatus, SmartTranscodeMark,
        SmartTranscodeOutcome, User,
    },
};
use noflex_transcoder::{
    ClientCapabilities, DecideOptions, HwBackend, TonemapOptions, VideoPlan,
    optimize::{DeviceTarget, OptimizeTarget, build_optimize_argv, plan_versions},
    package::finalize,
    probe::ProbeReport,
};
use time::OffsetDateTime;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, Command},
    task::JoinHandle,
};

use crate::{
    config::{LadderEntry, OptimizeConfig},
    stream::pick_encoder,
};

const TICK: Duration = Duration::from_secs(30);
/// How often a running job looks for playback and reports progress.
const JOB_POLL: Duration = Duration::from_secs(2);
/// A paused encoder still holds memory and hardware sessions, so a long
/// pause gives them back and starts over later.
const MAX_PAUSE: Duration = Duration::from_secs(30 * 60);
const MAX_ATTEMPTS: u32 = 3;
const MIB: u64 = 1024 * 1024;
/// Where marked titles start in the queue: ahead of every guess, whose priorities
/// count up from 0, and in the order they were marked.
const MARKED_PRIORITY: i32 = i32::MIN / 2;
/// How long a finished copy stays once nothing wants it and nobody's played it.
/// Predictions shift as people watch, so anything shorter would remake the same
/// titles, and a day is longer than a viewer stays in the middle of one.
const SWEEP_AFTER: time::Duration = time::Duration::days(1);

/// Everything the worker depends on but doesn't own.
pub struct OptimizeWorker {
    pub storage: Arc<dyn Storage>,
    pub playback: Arc<dyn PlaybackSessionService>,
    pub plugins: Option<Arc<PluginRegistry>>,
    pub config: OptimizeConfig,
    pub ladder: Vec<LadderEntry>,
    pub ffmpeg_path: PathBuf,
    pub backend: HwBackend,
    pub vaapi_device: Option<PathBuf>,
    /// Tone-map HDR on the GPU; see [`noflex_transcoder::hwdetect::gpu_tonemap_works`].
    pub gpu_tonemap: bool,
    /// The configured curve, until an administrator picks one.
    pub tonemap_curve: noflex_core::TonemapCurve,
    /// Finished versions live directly in here, as `{id}.mp4`, until an
    /// administrator picks somewhere else.
    pub dir: PathBuf,
    /// Woken when a viewer changes a smart transcode mark.
    pub wake: Arc<tokio::sync::Notify>,
}

/// The limits in force: an administrator's settings over the configuration.
struct Limits {
    dir: PathBuf,
    quota_bytes: u64,
    version_cap_bytes: Option<u64>,
}

/// How a job ended, when not in error.
enum JobEnd {
    Finished,
    /// Paused too long, or something marked needs the encoder; back in the queue.
    Requeued,
    /// Nobody wants it any more; gone.
    Dropped,
}

/// The planner's verdict on one source's versions.
type Keep = HashSet<(MediaFileId, String)>;

impl OptimizeWorker {
    /// Run forever: re-plan on the configured interval, and start a queued job
    /// whenever the server is idle. Does nothing while disabled.
    pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            if !self.config.enabled {
                tracing::info!("smart transcoding disabled");
                return;
            }
            if let Err(error) = self.storage.requeue_running_optimized_versions().await {
                tracing::warn!(%error, "smart transcoding: requeue after restart failed");
            }
            let mut system = sysinfo::System::new_with_specifics(
                sysinfo::RefreshKind::nothing()
                    .with_cpu(sysinfo::CpuRefreshKind::nothing().with_usage()),
            )
            .ok();
            let mut last_plan: Option<Instant> = None;
            let plan_every = Duration::from_secs(self.config.plan_interval_secs.max(60));
            let mut woken = false;
            loop {
                if woken || last_plan.is_none_or(|at| at.elapsed() >= plan_every) {
                    if let Err(error) = self.plan().await {
                        tracing::warn!(%error, "smart transcoding: planning failed");
                    }
                    last_plan = Some(Instant::now());
                }
                // sysinfo reports usage since the previous refresh, so this
                // averages over the whole tick.
                let cpu = system.as_mut().map_or(0.0, |system| {
                    system.refresh_cpu_usage();
                    system.global_cpu_usage()
                });
                if cpu <= self.config.max_cpu_percent
                    && self.playback_idle().await
                    && let Err(error) = self.run_next().await
                {
                    tracing::warn!(%error, "smart transcoding: job failed");
                }
                woken = tokio::select! {
                    () = tokio::time::sleep(TICK) => false,
                    () = self.wake.notified() => true,
                };
            }
        })
    }

    async fn playback_idle(&self) -> bool {
        match self.playback.active_streams().await {
            Ok(streams) => streams.is_empty(),
            Err(error) => {
                tracing::warn!(%error, "smart transcoding: can't read active streams");
                false
            }
        }
    }

    async fn limits(&self) -> anyhow::Result<Limits> {
        let settings = self.storage.smart_transcoding_settings().await?;
        let gb = |gb: u64| gb.saturating_mul(1024 * MIB);
        Ok(Limits {
            dir: settings.dir.map_or_else(|| self.dir.clone(), PathBuf::from),
            quota_bytes: gb(settings
                .max_storage_gb
                .unwrap_or(self.config.max_storage_gb)),
            version_cap_bytes: settings
                .max_version_gb
                .or(self.config.max_version_gb)
                .map(gb),
        })
    }

    /// Bring the queue in line with what viewers have marked and are likely to
    /// watch: queue what's missing, drop waiting jobs nobody wants now, remove
    /// versions of files that have changed, sweep up copies left unwanted, and
    /// evict over quota. Touches no running job. Returns the versions wanted.
    pub async fn plan(&self) -> anyhow::Result<Keep> {
        let limits = self.limits().await?;
        // Before queuing: a stale row would block its replacement's key.
        self.remove_stale().await?;
        let devices = self.storage.devices_wanting_versions().await?;
        let now = OffsetDateTime::now_utc();
        let namespace = self
            .plugins
            .as_ref()
            .and_then(|registry| registry.embedder_claim())
            .map(|claim| claim.namespace);

        let mut by_user: HashMap<UserId, Vec<Device>> = HashMap::new();
        for device in devices {
            by_user.entry(device.user_id).or_default().push(device);
        }
        // The same title for two users is one plan across all their devices.
        let mut wanted: HashMap<MediaFileId, Want> = HashMap::new();
        let mut targets_by_user: HashMap<UserId, Vec<DeviceTarget>> = HashMap::new();
        for (user, devices) in by_user {
            let Some(account) = self.storage.get_user(user).await? else {
                continue;
            };
            let policy_limit = self.policy_limit(&account).await?;
            let targets = devices
                .iter()
                .filter_map(|device| device_target(device, policy_limit))
                .collect::<Vec<_>>();
            if targets.is_empty() {
                continue;
            }
            targets_by_user.insert(user, targets.clone());
            let likely = likely_watched(
                self.storage.as_ref(),
                namespace.as_deref(),
                RequestUser {
                    user_id: user,
                    is_admin: account.is_admin,
                },
                LikelyOptions {
                    lookahead_episodes: self.config.lookahead_episodes,
                    recommendations: self.config.recommendations,
                },
                now,
            )
            .await?;
            for (priority, work) in likely.iter().enumerate() {
                for media in playable_sources(self.storage.as_ref(), work).await? {
                    wanted
                        .entry(media.id)
                        .or_insert_with(|| Want::new(media))
                        .add(priority as i32, &targets);
                }
            }
        }

        let opts = self.decide_options();
        let marks = self
            .plan_marks(&targets_by_user, &opts, &mut wanted)
            .await?;

        let mut keep: Keep = HashSet::new();
        let mut prepared: HashSet<MediaFileId> = HashSet::new();
        for want in wanted.values() {
            let Ok(probe) = serde_json::from_value::<ProbeReport>(want.media.probe.0.clone())
            else {
                continue;
            };
            for target in plan_versions(&probe, &want.targets, &opts, &self.ladder) {
                let estimate = target
                    .estimated_bitrate_bps(&probe)
                    .saturating_mul(probe.duration_ms.unwrap_or(0))
                    / 8000;
                if limits.version_cap_bytes.is_some_and(|cap| estimate > cap) {
                    continue;
                }
                let key = target.key();
                keep.insert((want.media.id, key.clone()));
                prepared.insert(want.media.id);
                self.storage
                    .enqueue_optimized_version(&OptimizedVersion {
                        id: OptimizedVersionId::new(),
                        media_file_id: want.media.id,
                        variant_key: key,
                        status: OptimizedVersionStatus::Queued,
                        priority: want.priority,
                        plan: serde_json::to_value(&target)?,
                        source_size: want.media.size_bytes,
                        source_mtime: want.media.mtime,
                        progress: 0.0,
                        attempts: 0,
                        path: None,
                        size_bytes: None,
                        index: None,
                        probe: None,
                        error: None,
                        created_at: now,
                        finished_at: None,
                        last_used_at: None,
                        unwanted_since: None,
                    })
                    .await?;
            }
        }

        let mut marked_media: HashSet<MediaFileId> = HashSet::new();
        for (mark, needed) in &marks {
            marked_media.extend(needed.iter().copied());
            let reached = needed
                .iter()
                .copied()
                .filter(|media| prepared.contains(media))
                .collect::<Vec<_>>();
            let outcome = match (needed.is_empty(), reached.is_empty()) {
                (true, _) => SmartTranscodeOutcome::NotNeeded,
                (false, true) => SmartTranscodeOutcome::TooLarge,
                (false, false) => SmartTranscodeOutcome::Planned,
            };
            self.storage
                .record_smart_transcode_plan(mark, outcome, &reached)
                .await?;
        }
        self.prune(&keep, &marked_media, &limits).await?;
        Ok(keep)
    }

    async fn policy_limit(&self, account: &User) -> anyhow::Result<Option<u64>> {
        Ok(if account.is_admin {
            None
        } else {
            self.storage
                .get_user_restrictions(account.id)
                .await?
                .and_then(|policy| policy.max_stream_bitrate_bps)
        })
    }

    /// Add what viewers have marked to `wanted`, ahead of any guess, dropping marks
    /// with nothing left to do. Returns each remaining mark with the sources that
    /// need a version for the devices it's for. A mark with no devices is recorded
    /// here and isn't returned, as there's nothing further to find out about it.
    async fn plan_marks(
        &self,
        targets_by_user: &HashMap<UserId, Vec<DeviceTarget>>,
        opts: &DecideOptions,
        wanted: &mut HashMap<MediaFileId, Want>,
    ) -> anyhow::Result<Vec<(SmartTranscodeMark, Vec<MediaFileId>)>> {
        let mut planned = Vec::new();
        for (position, mark) in self
            .storage
            .list_smart_transcode_marks()
            .await?
            .into_iter()
            .enumerate()
        {
            let remove = async |mark: &SmartTranscodeMark| {
                self.storage
                    .remove_smart_transcode_mark(mark.user_id, mark.work_id, mark.device_id)
                    .await
            };
            let (Some(account), Some(work)) = (
                self.storage.get_user(mark.user_id).await?,
                self.storage.get_work(mark.work_id).await?,
            ) else {
                continue;
            };
            let user = RequestUser {
                user_id: mark.user_id,
                is_admin: account.is_admin,
            };
            if !noflex_storage::access::user_can_access_work(
                self.storage.as_ref(),
                user,
                mark.work_id,
            )
            .await?
            {
                remove(&mark).await?;
                continue;
            }
            let targets = match mark.device_id {
                None => targets_by_user
                    .get(&mark.user_id)
                    .cloned()
                    .unwrap_or_default(),
                Some(id) => match self.storage.get_device(id).await? {
                    Some(device) if device.user_id == mark.user_id => {
                        let limit = self.policy_limit(&account).await?;
                        device_target(&device, limit).into_iter().collect()
                    }
                    _ => {
                        remove(&mark).await?;
                        continue;
                    }
                },
            };
            let works = marked_works(
                self.storage.as_ref(),
                mark.user_id,
                &work,
                mark.created_at,
                self.config.lookahead_episodes,
            )
            .await?;
            if works.is_empty() {
                remove(&mark).await?;
                continue;
            }
            if targets.is_empty() {
                self.storage
                    .record_smart_transcode_plan(&mark, SmartTranscodeOutcome::NoDevices, &[])
                    .await?;
                continue;
            }
            let mut needed = Vec::new();
            for work in &works {
                for media in playable_sources(self.storage.as_ref(), work).await? {
                    let id = media.id;
                    // Alone, because another viewer's device could need a version
                    // of a file that plays as it is on the ones this mark is for.
                    let needs_one = serde_json::from_value::<ProbeReport>(media.probe.0.clone())
                        .is_ok_and(|probe| {
                            !plan_versions(&probe, &targets, opts, &self.ladder).is_empty()
                        });
                    if needs_one {
                        needed.push(id);
                    }
                    wanted
                        .entry(id)
                        .or_insert_with(|| Want::new(media))
                        .add(MARKED_PRIORITY.saturating_add(position as i32), &targets);
                }
            }
            planned.push((mark, needed));
        }
        Ok(planned)
    }

    fn decide_options(&self) -> DecideOptions {
        DecideOptions {
            burn_subtitle: false,
            encode_video: self.config.encode_video.clone(),
            encode_audio: vec![noflex_core::AudioCodec::Aac],
            audio_stream: None,
        }
    }

    /// Remove versions made from an older copy of their source. A running job
    /// is left to finish; the next plan catches it.
    async fn remove_stale(&self) -> anyhow::Result<()> {
        for version in self.storage.list_optimized_versions().await? {
            if version.status == OptimizedVersionStatus::Running {
                continue;
            }
            let stale = match self.storage.media_file_by_id(version.media_file_id).await? {
                Some(media) => {
                    media.size_bytes != version.source_size || media.mtime != version.source_mtime
                }
                None => true,
            };
            if stale {
                self.remove(&version).await?;
            }
        }
        Ok(())
    }

    /// Remove what nobody wants, whether or not there's room: a job at once, and
    /// a finished copy after [`SWEEP_AFTER`]. Then make room. Versions of `marked`
    /// files are never evicted while wanted. A marked job waiting on space can evict what's
    /// only a guess, which a later plan queues again behind it.
    async fn prune(
        &self,
        keep: &Keep,
        marked: &HashSet<MediaFileId>,
        limits: &Limits,
    ) -> anyhow::Result<()> {
        let mut known = HashSet::new();
        let mut evictable = Vec::new();
        let mut guesses = Vec::new();
        let mut marked_waiting = false;
        let mut used = 0_u64;
        let now = OffsetDateTime::now_utc();
        for version in self.storage.list_optimized_versions().await? {
            let wanted = keep.contains(&(version.media_file_id, version.variant_key.clone()));
            let is_marked = wanted && marked.contains(&version.media_file_id);
            // A failed version remembers not to retry only while it's still
            // wanted; dropping it lets a later plan try again.
            let unwanted_job = matches!(
                version.status,
                OptimizedVersionStatus::Queued | OptimizedVersionStatus::Failed
            ) && !wanted;
            if unwanted_job {
                self.remove(&version).await?;
                continue;
            }
            if version.status == OptimizedVersionStatus::Ready && !wanted {
                let since = version.unwanted_since.unwrap_or(now);
                if version.unwanted_since.is_none() {
                    self.storage
                        .mark_optimized_version_unwanted(version.id, now)
                        .await?;
                }
                let played_lately = version
                    .last_used_at
                    .is_some_and(|at| now - at < SWEEP_AFTER);
                if now - since >= SWEEP_AFTER && !played_lately {
                    tracing::info!(version = %version.id, "smart transcoding: sweeping up an unwanted copy");
                    self.remove(&version).await?;
                    continue;
                }
            }
            known.insert(version.id);
            marked_waiting |= is_marked && version.status == OptimizedVersionStatus::Queued;
            if version.status == OptimizedVersionStatus::Ready {
                used += version.size_bytes.unwrap_or(0);
                if !wanted {
                    evictable.push(version);
                } else if !is_marked {
                    guesses.push(version);
                }
            }
        }

        evictable.sort_by_key(|version| version.last_used_at.or(version.finished_at));
        if marked_waiting {
            guesses.sort_by_key(|version| version.last_used_at.or(version.finished_at));
            evictable.append(&mut guesses);
        }
        for version in evictable {
            // A job can't start at the quota, so one waiting needs it under.
            let over = used > limits.quota_bytes || (marked_waiting && used >= limits.quota_bytes);
            if !over {
                break;
            }
            used = used.saturating_sub(version.size_bytes.unwrap_or(0));
            known.remove(&version.id);
            self.remove(&version).await?;
        }
        self.remove_orphan_files(&known, &limits.dir).await
    }

    /// Whether an administrator has removed this since it was claimed.
    async fn removed(&self, version: &OptimizedVersion) -> anyhow::Result<bool> {
        Ok(self
            .storage
            .get_optimized_version(version.id)
            .await?
            .is_none())
    }

    async fn remove(&self, version: &OptimizedVersion) -> anyhow::Result<()> {
        if let Some(path) = &version.path
            && let Err(error) = tokio::fs::remove_file(path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, version = %version.id, "optimized file not removed");
        }
        self.storage.delete_optimized_version(version.id).await?;
        Ok(())
    }

    /// A library removal takes version rows with it but can't reach their
    /// files, and a crash can leave partial output behind. Only files named the
    /// way this worker names them count: the folder is an administrator's
    /// choice, and may well hold things of theirs.
    async fn remove_orphan_files(
        &self,
        known: &HashSet<OptimizedVersionId>,
        dir: &Path,
    ) -> anyhow::Result<()> {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let running = self
            .storage
            .list_optimized_versions()
            .await?
            .into_iter()
            .filter(|version| version.status == OptimizedVersionStatus::Running)
            .map(|version| version.id.to_string())
            .collect::<HashSet<_>>();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some((id, suffix)) = version_file(&name) else {
                continue;
            };
            let belongs =
                running.contains(&id.to_string()) || (known.contains(&id) && suffix != "raw.mp4");
            if !belongs {
                tracing::debug!(file = %name, "removing orphaned optimized file");
                if let Err(error) = tokio::fs::remove_file(entry.path()).await {
                    tracing::warn!(%error, file = %name, "orphaned optimized file not removed");
                }
            }
        }
        Ok(())
    }

    /// Run the most urgent queued job to completion, if the quota has room.
    /// Returns whether there was one to run.
    pub async fn run_next(&self) -> anyhow::Result<bool> {
        let limits = self.limits().await?;
        let used = self.storage.optimized_version_stats().await?.ready_bytes;
        if used >= limits.quota_bytes {
            return Ok(false);
        }
        let Some(mut version) = self.storage.claim_optimized_version().await? else {
            return Ok(false);
        };
        match self.run_job(&mut version, &limits.dir).await {
            Ok(JobEnd::Finished | JobEnd::Requeued | JobEnd::Dropped) => {}
            Err(error) => {
                // The detail stays in the log: it can carry paths, and this
                // field reaches admins' browsers.
                tracing::warn!(%error, version = %version.id, "optimized version failed");
                version.error = Some("preparing this version failed; see the server log".into());
                version.status = if version.attempts >= MAX_ATTEMPTS {
                    OptimizedVersionStatus::Failed
                } else {
                    OptimizedVersionStatus::Queued
                };
                version.finished_at = Some(OffsetDateTime::now_utc());
                self.storage.update_optimized_version(&version).await?;
                tokio::fs::remove_file(raw_path(&limits.dir, version.id))
                    .await
                    .ok();
            }
        }
        Ok(true)
    }

    async fn run_job(&self, version: &mut OptimizedVersion, dir: &Path) -> anyhow::Result<JobEnd> {
        let media = self
            .storage
            .media_file_by_id(version.media_file_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("source file is gone"))?;
        let target = serde_json::from_value::<OptimizeTarget>(version.plan.clone())?;
        let probe = serde_json::from_value::<ProbeReport>(media.probe.0.clone())?;
        tokio::fs::create_dir_all(dir).await?;

        let raw = raw_path(dir, version.id);
        let encoder = match target.video {
            VideoPlan::Encode { codec, .. } => pick_encoder(self.backend, codec),
            _ => pick_encoder(self.backend, noflex_core::VideoCodec::H264),
        };
        let argv = build_optimize_argv(
            Path::new(&media.path),
            &target,
            encoder,
            self.vaapi_device.as_deref(),
            TonemapOptions {
                gpu: self.gpu_tonemap,
                curve: self
                    .storage
                    .tonemap_curve()
                    .await?
                    .unwrap_or(self.tonemap_curve),
            },
            &raw,
        );
        let mut child = Command::new(&self.ffmpeg_path)
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        tracing::info!(version = %version.id, key = %version.variant_key, "preparing optimized version");
        match self.supervise(&mut child, version, &probe, &raw).await? {
            JobEnd::Finished => {}
            end => return Ok(end),
        }
        self.publish(version, &raw, dir).await?;
        tracing::info!(version = %version.id, bytes = version.size_bytes, "optimized version ready");
        Ok(JobEnd::Finished)
    }

    /// Wait for ffmpeg, reporting progress, stepping aside while anyone is
    /// watching. `SIGSTOP` keeps progress that restarting a long encode would
    /// lose.
    async fn supervise(
        &self,
        child: &mut Child,
        version: &mut OptimizedVersion,
        probe: &ProbeReport,
        raw: &Path,
    ) -> anyhow::Result<JobEnd> {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("ffmpeg stdout unavailable"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("ffmpeg stderr unavailable"))?;
        let errors = tokio::spawn(async move {
            let mut buffer = String::new();
            stderr.read_to_string(&mut buffer).await.ok();
            buffer
        });
        let mut lines = BufReader::new(stdout).lines();
        let duration_us = probe.duration_ms.unwrap_or(0).saturating_mul(1000);
        let mut poll = tokio::time::interval(JOB_POLL);
        let mut paused_since: Option<Instant> = None;
        let mut out_time_us = 0_u64;
        let mut output_open = true;

        let status = loop {
            tokio::select! {
                line = lines.next_line(), if output_open => match line {
                    Ok(Some(line)) => {
                        if let Some(value) = line.strip_prefix("out_time_us=") {
                            out_time_us = value.trim().parse().unwrap_or(out_time_us);
                        }
                    }
                    _ => output_open = false,
                },
                status = child.wait() => break status?,
                () = self.wake.notified() => {
                    if self.removed(version).await? {
                        child.kill().await?;
                        tokio::fs::remove_file(raw).await.ok();
                        self.wake.notify_one();
                        return Ok(JobEnd::Dropped);
                    }
                    // A mark changed. The plan touches no running job, so that's
                    // for here to act on.
                    let keep = match Box::pin(self.plan()).await {
                        Ok(keep) => keep,
                        Err(error) => {
                            tracing::warn!(%error, "smart transcoding: planning failed");
                            continue;
                        }
                    };
                    if !keep.contains(&(version.media_file_id, version.variant_key.clone())) {
                        child.kill().await?;
                        tokio::fs::remove_file(raw).await.ok();
                        self.remove(version).await?;
                        // The encoder's free, so don't wait out a tick to use it.
                        self.wake.notify_one();
                        return Ok(JobEnd::Dropped);
                    }
                    let outranked = self
                        .storage
                        .list_optimized_versions()
                        .await?
                        .iter()
                        .any(|other| {
                            other.status == OptimizedVersionStatus::Queued
                                && other.priority < version.priority
                        });
                    if outranked {
                        self.wake.notify_one();
                        return self.requeue(child, version, raw).await;
                    }
                },
                _ = poll.tick() => {
                    let idle = self.playback_idle().await;
                    match (idle, paused_since) {
                        (false, None) => {
                            set_paused(child, true);
                            paused_since = Some(Instant::now());
                            tracing::debug!(version = %version.id, "paused for playback");
                        }
                        (true, Some(_)) => {
                            set_paused(child, false);
                            paused_since = None;
                            tracing::debug!(version = %version.id, "resumed");
                        }
                        (false, Some(since)) if since.elapsed() >= MAX_PAUSE => {
                            return self.requeue(child, version, raw).await;
                        }
                        _ => {}
                    }
                    if duration_us > 0 {
                        version.progress = (out_time_us as f64 / duration_us as f64).min(1.0) as f32;
                        self.storage.update_optimized_version(version).await?;
                    }
                }
            }
        };
        if !status.success() {
            let detail = errors.await.unwrap_or_default();
            anyhow::bail!("ffmpeg exited with {status}: {}", detail.trim());
        }
        Ok(JobEnd::Finished)
    }

    /// Stop a job and put it back in the queue, which isn't a failed attempt.
    async fn requeue(
        &self,
        child: &mut Child,
        version: &mut OptimizedVersion,
        raw: &Path,
    ) -> anyhow::Result<JobEnd> {
        // SIGKILL reaches a stopped process.
        child.kill().await?;
        version.status = OptimizedVersionStatus::Queued;
        version.progress = 0.0;
        version.attempts = version.attempts.saturating_sub(1);
        self.storage.update_optimized_version(version).await?;
        tokio::fs::remove_file(raw).await.ok();
        Ok(JobEnd::Requeued)
    }

    /// Turn ffmpeg's output into a finished version. The final name appears
    /// only once it's complete, so nothing can read a partial package.
    async fn publish(
        &self,
        version: &mut OptimizedVersion,
        raw: &Path,
        dir: &Path,
    ) -> anyhow::Result<()> {
        let staged = dir.join(format!("{}.staged.mp4", version.id));
        let final_path = dir.join(format!("{}.mp4", version.id));
        let (raw_owned, staged_owned) = (raw.to_path_buf(), staged.clone());
        let index =
            tokio::task::spawn_blocking(move || finalize(&raw_owned, &staged_owned)).await??;
        tokio::fs::remove_file(raw).await.ok();
        let probe = noflex_scanner::ffprobe::probe_media_file(&staged, None).await?;
        tokio::fs::rename(&staged, &final_path).await?;

        version.status = OptimizedVersionStatus::Ready;
        version.progress = 1.0;
        version.path = Some(final_path.display().to_string());
        version.size_bytes = Some(index.size_bytes);
        version.index = Some(serde_json::to_value(&index)?);
        version.probe = Some(serde_json::to_value(&probe)?);
        version.error = None;
        version.finished_at = Some(OffsetDateTime::now_utc());
        self.storage.update_optimized_version(version).await?;
        // Removed in the moment it finished, so nothing else knows about this file.
        if self.removed(version).await? {
            tokio::fs::remove_file(&final_path).await.ok();
        }
        Ok(())
    }
}

/// The version and stage a file in a versions folder is named for, when it's
/// one this worker wrote: `{id}.mp4`, or `{id}.raw.mp4` or `{id}.staged.mp4` on
/// the way there. Anything else in the folder is someone else's.
pub(crate) fn version_file(name: &str) -> Option<(OptimizedVersionId, &str)> {
    let (id, suffix) = name.split_once('.')?;
    let id = uuid::Uuid::parse_str(id).ok()?;
    matches!(suffix, "mp4" | "raw.mp4" | "staged.mp4")
        .then(|| (OptimizedVersionId::from_uuid(id), suffix))
}

fn raw_path(dir: &Path, id: OptimizedVersionId) -> PathBuf {
    dir.join(format!("{id}.raw.mp4"))
}

/// A source and the devices that want it.
struct Want {
    media: MediaFile,
    priority: i32,
    targets: Vec<DeviceTarget>,
}

impl Want {
    fn new(media: MediaFile) -> Self {
        Self {
            media,
            priority: i32::MAX,
            targets: Vec::new(),
        }
    }

    /// Keep the most urgent priority, and each device once: one viewer's guess and
    /// mark can both name a device.
    fn add(&mut self, priority: i32, targets: &[DeviceTarget]) {
        self.priority = self.priority.min(priority);
        for target in targets {
            if !self.targets.iter().any(|known| known.id == target.id) {
                self.targets.push(target.clone());
            }
        }
    }
}

/// `device` as the planner sees it, with its limits and the user's policy
/// applied. `None` when its capabilities can't be read, which only an old or
/// buggy client produces.
fn device_target(device: &Device, policy_limit: Option<u64>) -> Option<DeviceTarget> {
    let mut capabilities =
        serde_json::from_value::<ClientCapabilities>(device.capabilities.0.clone()).ok()?;
    capabilities.max_bitrate_bps = [
        capabilities.max_bitrate_bps,
        device.max_bitrate_bps,
        policy_limit,
    ]
    .into_iter()
    .flatten()
    .min();
    let allowed = &device.allowed_video_codecs;
    if !allowed.is_empty() {
        capabilities
            .video_codecs
            .retain(|support| allowed.contains(&support.codec));
        for hls in &mut capabilities.hls {
            hls.video_codecs.retain(|codec| allowed.contains(codec));
        }
        for profile in &mut capabilities.direct_play {
            profile.video_codecs.retain(|codec| allowed.contains(codec));
        }
    }
    Some(DeviceTarget {
        id: device.id,
        capabilities,
    })
}

/// Holds a running ffmpeg where it is, or lets it go on, without ending it.
fn set_paused(child: &Child, paused: bool) {
    #[cfg(unix)]
    {
        let Some(pid) = child.id() else {
            return;
        };
        let signal = if paused { libc::SIGSTOP } else { libc::SIGCONT };
        // SAFETY: kill(2) takes plain integers and touches no memory. `pid` is our
        // own child, not yet reaped because this runs before `wait` resolves.
        let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
        if result != 0 {
            tracing::warn!(pid, signal, "signalling ffmpeg failed");
        }
    }
    #[cfg(windows)]
    {
        // Windows has no stop signal. These suspend every thread of a process
        // at once; they're ntdll's, undocumented but stable since XP, and what
        // Task Manager and Process Explorer use.
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtSuspendProcess(process: std::os::windows::raw::HANDLE) -> i32;
            fn NtResumeProcess(process: std::os::windows::raw::HANDLE) -> i32;
        }
        let Some(handle) = child.raw_handle() else {
            return;
        };
        // SAFETY: the handle is our own child's, with every access right, and
        // stays open until `wait` resolves, which this runs before.
        let status = unsafe {
            if paused {
                NtSuspendProcess(handle)
            } else {
                NtResumeProcess(handle)
            }
        };
        // An NTSTATUS is an error when negative.
        if status < 0 {
            tracing::warn!(status, paused, "suspending ffmpeg failed");
        }
    }
}
