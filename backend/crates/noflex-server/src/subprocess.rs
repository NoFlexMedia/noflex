// SPDX-License-Identifier: AGPL-3.0-only
//
// Bounded external-tool runs (ffmpeg / ffprobe) for the background
// workers.

use std::{path::Path, process::Stdio};

use tokio::{process::Command, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("spawning {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("{program} timed out after {timeout:?}")]
    Timeout { program: String, timeout: Duration },
    #[error("{program} failed with status {status}: {stderr}")]
    Process {
        program: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
}

/// Run `program` to completion with stdin closed, killing it if it
/// outlives `timeout`. A non-zero exit is [`CommandError::Process`]
/// carrying the trimmed stderr.
pub async fn command_output(
    program: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<std::process::Output, CommandError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(timeout, child)
        .await
        .map_err(|_| CommandError::Timeout {
            program: program.display().to_string(),
            timeout,
        })?
        .map_err(|source| CommandError::Spawn {
            program: program.display().to_string(),
            source,
        })?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(CommandError::Process {
            program: program.display().to_string(),
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}
