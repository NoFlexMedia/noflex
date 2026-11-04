// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::PathBuf, process::Stdio};

use tokio::process::{Child, Command};

use crate::{JobHandle, TranscodeRequest, build_ffmpeg_argv};

/// Spawn the ffmpeg subprocess for a transcode request.
/// The `JobHandle` returned by `FfmpegJobManager::start_job`
/// can be passed here together with the original request.
///
/// `binary` lets callers point at a non-default ffmpeg path
/// (the Nix dev shell puts ffmpeg on PATH; the NixOS module
/// exposes the path as `services.noflex.transcoder.ffmpegPath`).
pub async fn spawn(
    request: &TranscodeRequest,
    binary: &str,
    handle: JobHandle,
) -> std::io::Result<SpawnedJob> {
    let argv = build_ffmpeg_argv(request);
    let mut attempt = 0;
    let child = loop {
        let mut cmd = Command::new(binary);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            // stdout carries `-progress` blocks; stderr the error tail.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match cmd.spawn() {
            // Overlay filesystems (Nix store, some CI runners)
            // briefly report ETXTBSY when exec'ing a just-written
            // binary; retry instead of flaking the job.
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 10 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            result => break result?,
        }
    };
    Ok(SpawnedJob {
        handle,
        child: Some(child),
    })
}

/// A running ffmpeg subprocess. Drops the child handle to
/// SIGKILL the process; callers should `await terminate()`
/// for a clean kill.
pub struct SpawnedJob {
    pub handle: JobHandle,
    child: Option<Child>,
}

impl SpawnedJob {
    /// Hand the child over to a long-lived driver (e.g. the
    /// job manager's monitor task). `None` if it was already
    /// taken by `terminate` / `wait_with_stderr`.
    pub fn into_child(self) -> Option<Child> {
        self.child
    }

    /// Send SIGTERM (or `TerminateProcess` on Windows) and
    /// wait for the process to exit. Idempotent.
    pub async fn terminate(&mut self) -> std::io::Result<()> {
        if let Some(mut child) = self.child.take() {
            child.start_kill().ok();
            let _ = child.wait().await;
        }
        Ok(())
    }

    /// Wait for the process to exit on its own and return
    /// the status. Reads stderr in the background; the
    /// collected stderr is returned for the audit log.
    pub async fn wait_with_stderr(
        &mut self,
    ) -> std::io::Result<(std::process::ExitStatus, Vec<u8>)> {
        let mut child = self
            .child
            .take()
            .ok_or_else(|| std::io::Error::other("child already taken"))?;
        // Drain progress so a full pipe can't stall the child.
        if let Some(mut out) = child.stdout.take() {
            tokio::spawn(async move {
                let _ = tokio::io::copy(&mut out, &mut tokio::io::sink()).await;
            });
        }
        let stderr = child.stderr.take();
        let (status, stderr_buf) = if let Some(mut err) = stderr {
            let mut sink = Vec::new();
            let read = tokio::spawn(async move {
                let _ = tokio::io::copy(&mut err, &mut sink).await;
                sink
            });
            let status = child.wait().await?;
            let stderr_buf = read.await.unwrap_or_default();
            (status, stderr_buf)
        } else {
            let status = child.wait().await?;
            (status, Vec::new())
        };
        Ok((status, stderr_buf))
    }

    pub fn id(&self) -> &str {
        &self.handle.id
    }

    pub fn output_dir(&self) -> &PathBuf {
        &self.handle.output_dir
    }
}

#[cfg(test)]
mod tests {
    use noflex_core::{AudioCodec, VideoCodec};

    use super::*;
    use crate::{
        decision::{AudioPlan, HlsSegmentContainer, TranscodePlan, VideoPlan},
        encoder::Encoder,
        ffmpeg_argv::{LadderEntry, TranscodeInput, TranscodeRequest},
    };

    fn make_request() -> TranscodeRequest {
        let input = std::env::temp_dir().join("noflex-spawn-test-input.txt");
        let _ = std::fs::write(&input, b"hello world\n");
        let output = std::env::temp_dir().join("noflex-spawn-test-output.m3u8");
        TranscodeRequest::new(
            TranscodeInput::File(input),
            output,
            TranscodePlan {
                segment_container: HlsSegmentContainer::Ts,
                video: VideoPlan::Encode {
                    codec: VideoCodec::H264,
                    width: 1920,
                    height: 1080,
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
                max_pixels: 2_073_600,
                video_codec: VideoCodec::H264,
                video_profile: noflex_core::VideoProfile::H264High,
                audio_codec: AudioCodec::Aac,
                container: "ts".to_string(),
                bitrate_kbps: 8000,
            },
        )
    }

    /// A shell script that records its argv to a file and
    /// exits 0. Used as a stand-in for the ffmpeg binary in
    /// the subprocess-spawn tests.
    fn make_mock_ffmpeg() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("noflex-mock-ffmpeg-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let mock = dir.join("ffmpeg");
        let log = dir.join("argv.log");
        let log_str = log.to_string_lossy();
        let script = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {log_str}\nexit 0\n");
        std::fs::write(&mock, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&mock).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&mock, perms).unwrap();
        }
        (mock, log)
    }

    #[tokio::test]
    async fn spawn_invokes_binary_with_built_argv() {
        let (mock, log) = make_mock_ffmpeg();
        let request = make_request();
        let handle = JobHandle {
            id: "test-job".to_string(),
            output_dir: std::env::temp_dir(),
        };
        let mut job = spawn(&request, mock.to_str().unwrap(), handle)
            .await
            .expect("spawn");
        let (status, _stderr) = job.wait_with_stderr().await.expect("wait");
        assert!(status.success());
        let args_logged: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        assert!(args_logged.contains(&"-c:v".to_string()));
        assert!(args_logged.contains(&"libx264".to_string()));
        assert!(args_logged.contains(&"-i".to_string()));
    }

    #[tokio::test]
    async fn spawn_returns_handle_id_unchanged() {
        let (mock, _log) = make_mock_ffmpeg();
        let request = make_request();
        let handle = JobHandle {
            id: "preserved-id".to_string(),
            output_dir: std::env::temp_dir(),
        };
        let job = spawn(&request, mock.to_str().unwrap(), handle)
            .await
            .expect("spawn");
        assert_eq!(job.id(), "preserved-id");
    }

    #[tokio::test]
    async fn terminate_is_safe_on_already_completed() {
        let (mock, _log) = make_mock_ffmpeg();
        let request = make_request();
        let handle = JobHandle {
            id: "x".to_string(),
            output_dir: std::env::temp_dir(),
        };
        let mut job = spawn(&request, mock.to_str().unwrap(), handle)
            .await
            .expect("spawn");
        let _ = job.wait_with_stderr().await;
        // Already-completed: terminate() should be a no-op,
        // not a use-after-take.
        job.terminate().await.expect("terminate noop");
    }

    #[tokio::test]
    async fn spawn_missing_binary_returns_io_error() {
        let request = make_request();
        let handle = JobHandle {
            id: "x".to_string(),
            output_dir: std::env::temp_dir(),
        };
        let result = spawn(&request, "/this/does/not/exist/anywhere", handle).await;
        assert!(result.is_err());
    }
}
