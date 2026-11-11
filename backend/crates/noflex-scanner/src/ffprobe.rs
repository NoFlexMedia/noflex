// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::Path, process::Stdio, time::Duration};

use noflex_transcoder::probe::{ProbeError, ProbeReport};
use tokio::process::Command;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const ENV_BINARY: &str = "FFPROBE_BIN";
const ENV_TIMEOUT: &str = "PROBE_TIMEOUT_SECS";

/// Run `ffprobe -v error -show_streams -show_format -show_chapters -of json <path>`
/// and return a typed [`ProbeReport`].
///
/// `binary` overrides the default `ffprobe` lookup:
/// - `Some(path)` → use that path verbatim
/// - `None` → honour `FFPROBE_BIN` env var if set, else `ffprobe`
///   from `PATH`.
///
/// The 30s default timeout is also overridable via
/// `PROBE_TIMEOUT_SECS` (so the test suite can use a shorter
/// ceiling). On timeout we return [`ProbeError::Timeout`].
///
/// On a real ffprobe process failure (non-zero exit, malformed
/// JSON) we map back into [`ProbeError::Parse`].
pub async fn probe_media_file(
    path: &Path,
    binary: Option<&str>,
) -> Result<ProbeReport, ProbeError> {
    let binary = resolve_binary(binary);
    let timeout = resolve_timeout();

    let mut cmd = Command::new(&binary);
    cmd.arg("-v")
        .arg("error")
        .arg("-show_streams")
        .arg("-show_format")
        .arg("-show_chapters")
        .arg("-of")
        .arg("json")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(err)) => return Err(ProbeError::Io(err)),
        Err(_) => {
            return Err(ProbeError::Timeout {
                secs: timeout.as_secs(),
            });
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ProbeError::Parse(format!(
            "ffprobe exited with status {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ProbeError::Parse(format!("invalid ffprobe JSON: {e}")))?;
    ProbeReport::from_ffprobe_json(json)
}

fn resolve_binary(override_path: Option<&str>) -> String {
    if let Some(p) = override_path {
        return p.to_string();
    }
    noflex_core::bundle::ffmpeg_tool("ffprobe", ENV_BINARY)
        .to_string_lossy()
        .into_owned()
}

fn resolve_timeout() -> Duration {
    if let Ok(s) = std::env::var(ENV_TIMEOUT)
        && let Ok(secs) = s.parse::<u64>()
    {
        return Duration::from_secs(secs);
    }
    DEFAULT_TIMEOUT
}

/// How far back from the target to look for a keyframe. Real-world GOPs
/// are a few seconds; a stream with none in this window just isn't snapped.
const KEYFRAME_LOOKBACK_SECS: f64 = 30.0;

/// Source time (seconds) of the last video keyframe at or before `secs`,
/// or `None` when the window before it has none. A stream-copy seek can
/// only start on a keyframe, so this is where such a transcode really
/// begins. Reads only the window before `secs`, not the whole file.
pub async fn keyframe_at_or_before(
    path: &Path,
    secs: f64,
    binary: Option<&str>,
) -> Result<Option<f64>, ProbeError> {
    let binary = resolve_binary(binary);
    let from = (secs - KEYFRAME_LOOKBACK_SECS).max(0.0);
    let mut cmd = Command::new(&binary);
    cmd.args(["-v", "error", "-protocol_whitelist", "file"])
        .args(["-select_streams", "v:0", "-skip_frame", "nokey"])
        .args(["-show_entries", "frame=pts_time", "-of", "csv=p=0"])
        .arg("-read_intervals")
        .arg(format!("{from}%{}", secs + 0.001))
        .arg(format!("file:{}", path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(resolve_timeout(), cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(err)) => return Err(ProbeError::Io(err)),
        Err(_) => {
            return Err(ProbeError::Timeout {
                secs: resolve_timeout().as_secs(),
            });
        }
    };
    if !output.status.success() {
        return Err(ProbeError::Parse(format!(
            "ffprobe exited with status {}",
            output.status
        )));
    }
    Ok(last_at_or_before(
        &String::from_utf8_lossy(&output.stdout),
        secs,
    ))
}

/// The largest timestamp in ffprobe's one-per-line output that is
/// `<= secs`; unparsable lines (`N/A`, trailing commas) are skipped.
fn last_at_or_before(output: &str, secs: f64) -> Option<f64> {
    output
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse::<f64>().ok())
        .filter(|t| *t <= secs + 1e-6)
        .reduce(f64::max)
}

#[cfg(test)]
mod tests {
    #[test]
    fn last_keyframe_at_or_before_target() {
        let out = "0.000000\n4.004000\n8.008000,\nN/A\n12.012000\n";
        assert_eq!(super::last_at_or_before(out, 10.0), Some(8.008));
        assert_eq!(super::last_at_or_before(out, 8.008), Some(8.008));
        assert_eq!(super::last_at_or_before(out, 3.0), Some(0.0));
        assert_eq!(super::last_at_or_before("", 3.0), None);
    }

    use std::path::PathBuf;

    use super::*;

    // Env-var tests share a single env var (`FFPROBE_BIN`); serialize
    // them so the parallel test runner doesn't see a partially-mutated
    // env. The guard is held across the probe await — dropping it after
    // set_var lets a sibling test clear FFPROBE_BIN mid-probe, which
    // flakes to a bare-`ffprobe` PATH lookup. Async mutex because the
    // guard crosses an await point.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn write_mock_ffprobe(script_body: &str) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("noflex-mock-ffprobe-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("ffprobe");
        let argv_log = dir.join("argv.log");
        let argv_str = argv_log.to_string_lossy();
        let script = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {argv_str}\n{script_body}\n");
        std::fs::write(&bin, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&bin, perms).unwrap();
        }
        (bin, argv_log)
    }

    #[tokio::test]
    async fn mock_binary_receives_expected_argv() {
        let (bin, argv_log) = write_mock_ffprobe(r#"printf '{"streams":[]}\n'"#);
        let lock = ENV_LOCK.lock().await;
        // SAFETY: serialized by `ENV_LOCK` against other env-mutating
        // tests running in parallel.
        unsafe {
            std::env::set_var(ENV_BINARY, &bin);
        }
        let res = probe_media_file(Path::new("/data/sample.mkv"), None).await;
        unsafe {
            std::env::remove_var(ENV_BINARY);
        }
        drop(lock);
        let r = res.expect("probe");
        assert!(r.video.is_none());
        let args: Vec<String> = std::fs::read_to_string(&argv_log)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(args[0], "-v");
        assert_eq!(args[1], "error");
        assert_eq!(args[2], "-show_streams");
        assert_eq!(args[3], "-show_format");
        assert_eq!(args[4], "-show_chapters");
        assert_eq!(args[5], "-of");
        assert_eq!(args[6], "json");
        assert_eq!(args[7], "/data/sample.mkv");
    }

    #[tokio::test]
    async fn binary_arg_overrides_env() {
        let (bin, _argv_log) = write_mock_ffprobe(r#"printf '{"streams":[]}\n'"#);
        // See mock_binary_receives_expected_argv: hold the guard across
        // the await so the env mutation stays serialized.
        let lock = ENV_LOCK.lock().await;
        // SAFETY: serialized by `ENV_LOCK` against other env-mutating
        // tests running in parallel.
        unsafe {
            std::env::set_var(ENV_BINARY, "/should/be/ignored");
        }
        let r = probe_media_file(Path::new("/data/x.mp4"), Some(&bin.to_string_lossy()))
            .await
            .expect("probe");
        unsafe {
            std::env::remove_var(ENV_BINARY);
        }
        drop(lock);
        assert!(r.video.is_none());
    }

    #[tokio::test]
    async fn non_zero_exit_maps_to_parse_error() {
        let (bin, _argv_log) = write_mock_ffprobe("echo 'fake stderr' >&2; exit 1");
        let err = probe_media_file(Path::new("/data/x.mp4"), Some(&bin.to_string_lossy()))
            .await
            .expect_err("should fail");
        assert!(matches!(err, ProbeError::Parse(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn missing_binary_maps_to_io_error() {
        let err = probe_media_file(
            Path::new("/data/x.mp4"),
            Some("/this/does/not/exist/anywhere"),
        )
        .await
        .expect_err("should fail");
        assert!(matches!(err, ProbeError::Io(_)), "got {err:?}");
    }
}
