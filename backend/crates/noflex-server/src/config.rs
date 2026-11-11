// SPDX-License-Identifier: AGPL-3.0-only
//
// Typed runtime configuration. Loaded via figment from
// $XDG_CONFIG_HOME/noflex/config.toml, or the file `NOFLEX_CONFIG_FILE` names
// (with /etc/noflex/config.toml as a system-tier fallback), and overridden by
// `NOFLEX_*` env vars (see [`ENV_KEYS`]). The
// shape here is the single source of truth — main.rs and the rest of
// the server consume the resolved `Config`, not raw env reads.

use std::path::PathBuf;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config parse: {0}")]
    Parse(String),
    #[error("invalid value for `{field}`: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub bind: String,
    /// Public HTTP(S) base URL used for provider callbacks. External
    /// authentication is unavailable when this is absent or invalid.
    #[serde(default)]
    pub public_url: Option<String>,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// Answer cross-origin requests, so clients connected to several servers
    /// can reach this one from another's web page.
    pub cross_origin: bool,
    /// Let the web client load Google's Cast sender script, so it can start a
    /// session on a Cast device. Off by default: the page's policy then admits
    /// scripts from Google, which a server that never casts has no need of.
    #[serde(default)]
    pub cast_sdk: bool,
    pub log: LogConfig,
    pub transcoder: TranscoderConfig,
    pub plugin: PluginConfig,
    pub scanner: ScannerConfig,
    pub search: SearchConfig,
    pub live_tv: LiveTvConfig,
    pub backup: BackupConfig,
    pub cache: CacheConfig,
    #[serde(default)]
    pub external_auth: ExternalAuthConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalAuthConfig {
    /// Create a NoFlex account on first login for a verified external
    /// identity that isn't linked yet. Off: only identities an existing
    /// user has linked may log in.
    #[serde(default)]
    pub auto_provision: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub level: String,
    pub file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscoderConfig {
    pub hwaccel: HwAccel,
    pub fallback: TranscodeFallback,
    pub ladder: Vec<LadderEntry>,
    pub optimize: OptimizeConfig,
    /// The curve HDR is tone-mapped with; administrators can override it.
    pub tonemap_curve: noflex_core::TonemapCurve,
}

/// Preparing versions ahead of time for registered devices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeConfig {
    pub enabled: bool,
    /// Finished versions; unset means `optimized/` under `data_dir`.
    pub dir: Option<PathBuf>,
    pub max_storage_gb: u64,
    /// Skip versions estimated larger than this; unset has no limit.
    pub max_version_gb: Option<u64>,
    /// Jobs start only below this much CPU use. They also wait for playback
    /// to end, whatever this says.
    pub max_cpu_percent: f32,
    /// How often to work out what's likely to be watched next.
    pub plan_interval_secs: u64,
    /// Episodes to prepare beyond each next episode.
    pub lookahead_episodes: u32,
    /// Top recommendations to prepare.
    pub recommendations: u32,
    /// Codecs to encode to, preferred first; a device gets the first it
    /// decodes.
    pub encode_video: Vec<noflex_core::VideoCodec>,
}

impl Default for OptimizeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: None,
            max_storage_gb: 200,
            max_version_gb: None,
            max_cpu_percent: 40.0,
            plan_interval_secs: 900,
            lookahead_episodes: 3,
            recommendations: 5,
            // There's no audience waiting on these, so the slow, small codecs win.
            encode_video: vec![
                noflex_core::VideoCodec::Av1,
                noflex_core::VideoCodec::H265,
                noflex_core::VideoCodec::H264,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HwAccel {
    #[default]
    None,
    Auto,
    Vaapi,
    VideoToolbox,
    Mf,
    Nvenc,
    Qsv,
    Amf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranscodeFallback {
    #[default]
    Auto,
    Software,
}

pub use noflex_transcoder::LadderEntry;

/// The default transcode ladder, in order of preference: what a client decodes
/// best first. Each rung's rate is its codec's rate for that area; hardware or
/// not, see [`crate::hwaccel::live_codec_order`] for what's fast enough to pick.
fn default_ladder() -> Vec<LadderEntry> {
    use noflex_core::{AudioCodec, VideoCodec, VideoProfile};
    const P4K: u64 = 8_294_400;
    const P1440: u64 = 3_686_400;
    const P1080: u64 = 2_073_600;
    const P720: u64 = 921_600;
    const P480: u64 = 409_920;
    [
        (VideoCodec::Av1, VideoProfile::Av1Main, P4K, 12_000),
        (VideoCodec::Av1, VideoProfile::Av1Main, P1440, 6_000),
        (VideoCodec::Av1, VideoProfile::Av1Main, P1080, 3_500),
        (VideoCodec::H265, VideoProfile::HevcMain, P4K, 16_000),
        (VideoCodec::H265, VideoProfile::HevcMain, P1440, 8_000),
        (VideoCodec::H265, VideoProfile::HevcMain, P1080, 5_000),
        (VideoCodec::H264, VideoProfile::H264High, P4K, 24_000),
        (VideoCodec::H264, VideoProfile::H264High, P1440, 12_000),
        (VideoCodec::H264, VideoProfile::H264High, P1080, 8_000),
        (VideoCodec::H264, VideoProfile::H264High, P720, 4_000),
        (VideoCodec::H264, VideoProfile::H264High, P480, 2_000),
    ]
    .map(
        |(video_codec, video_profile, max_pixels, bitrate_kbps)| LadderEntry {
            max_pixels,
            video_codec,
            video_profile,
            audio_codec: AudioCodec::Aac,
            container: "mp4".to_string(),
            bitrate_kbps,
        },
    )
    .to_vec()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub search_path: Vec<PathBuf>,
    /// Root directory the `host-assets.read-file` import serves
    /// (embedding model weights, tokenizers, ...). Unset disables the
    /// import: guests get an `unavailable` plugin error. Defaults to the
    /// release bundle's `assets/`, when there is one.
    #[serde(default)]
    pub assets_dir: Option<PathBuf>,
    pub overrides: std::collections::HashMap<String, PluginOverride>,
    pub per_plugin_config: std::collections::HashMap<String, toml::Table>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PluginOverride {
    pub disabled: bool,
    pub priorities: std::collections::HashMap<String, std::collections::HashMap<String, i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    pub debounce_ms: u64,
    pub concurrency: u32,
}

/// How indexing and semantic search share the host. See
/// `docs/CONFIGURATION.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub device: noflex_plugin_host::InferenceDevice,
    /// CPU threads per inference run; 0 picks half the physical cores.
    pub threads: usize,
    pub batch_size: u32,
    pub busy_percent: f32,
    /// The ONNX Runtime shared library, overriding a bundle's or the system's.
    #[serde(default)]
    pub onnxruntime_library: Option<PathBuf>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        let worker = noflex_plugin_host::embeddings::EmbeddingWorkerConfig::default();
        Self {
            device: noflex_plugin_host::InferenceDevice::default(),
            threads: 0,
            batch_size: worker.batch_size,
            busy_percent: worker.busy_percent,
            onnxruntime_library: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTvConfig {
    pub recording_concurrency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Transcoded segment cache (HLS chunks, ffmpeg output).
    pub transcode_dir: String,
    /// Image cache (posters, backdrops, episode thumbnails).
    pub image_dir: String,
    /// Embedding cache (raw vectors, optional).
    pub embed_dir: String,
    /// Subtitles viewers downloaded from a provider; never the media folder.
    pub subtitle_dir: String,
    /// Days a downloaded subtitle nobody plays is kept.
    pub subtitle_keep_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    /// Directory under which timestamped backup files are written.
    /// Empty string disables the scheduler.
    pub dir: String,
    /// Hours between scheduled backups. Zero = run once at
    /// startup and never again.
    pub interval_hours: u32,
    /// How many old backups to keep. Older ones are pruned at
    /// the start of each run.
    pub keep: u32,
}

impl Default for Config {
    fn default() -> Self {
        let (data_dir, cache_dir) = default_paths();
        Self {
            bind: "127.0.0.1:28793".to_string(),
            public_url: None,
            data_dir,
            cache_dir,
            cross_origin: true,
            cast_sdk: false,
            log: LogConfig {
                level: "info".to_string(),
                file: None,
            },
            transcoder: TranscoderConfig {
                hwaccel: HwAccel::Auto,
                fallback: TranscodeFallback::Auto,
                ladder: default_ladder(),
                optimize: OptimizeConfig::default(),
                tonemap_curve: noflex_core::TonemapCurve::default(),
            },
            plugin: PluginConfig {
                search_path: default_search_path(),
                assets_dir: noflex_core::bundle::dir("assets"),
                overrides: std::collections::HashMap::new(),
                per_plugin_config: std::collections::HashMap::new(),
            },
            scanner: ScannerConfig {
                debounce_ms: 500,
                concurrency: 4,
            },
            search: SearchConfig::default(),
            live_tv: LiveTvConfig {
                recording_concurrency: 2,
            },
            backup: BackupConfig {
                dir: String::new(),
                interval_hours: 0,
                keep: 7,
            },
            cache: CacheConfig {
                transcode_dir: String::new(),
                image_dir: String::new(),
                embed_dir: String::new(),
                subtitle_dir: String::new(),
                subtitle_keep_days: 30,
            },
            external_auth: ExternalAuthConfig::default(),
        }
    }
}

fn default_paths() -> (PathBuf, PathBuf) {
    if let Some(p) = directories::ProjectDirs::from("dev", "noflex", "NoFlex") {
        (p.data_dir().to_path_buf(), p.cache_dir().to_path_buf())
    } else {
        (PathBuf::from("./data"), PathBuf::from("./cache"))
    }
}

fn default_search_path() -> Vec<PathBuf> {
    let mut out = Vec::new();
    // First, so a plugin installed on the host shadows the bundle's copy.
    out.extend(noflex_core::bundle::dir("plugins"));
    out.push(noflex_core::system_dirs::config().join("plugins"));
    out.extend(noflex_core::system_dirs::state().map(|dir| dir.join("plugins")));
    out.extend(user_config_dir().map(|dir| dir.join("plugins")));
    out
}

impl Config {
    /// Load config from the standard search path + env-var overrides.
    /// The system-tier TOML (`/etc/noflex/config.toml`) is consulted
    /// first, then the user-tier
    /// (`$XDG_CONFIG_HOME/noflex/config.toml` or
    /// `~/.config/noflex/config.toml`, or whatever [`CONFIG_FILE_ENV`]
    /// names), then `NOFLEX_*` env vars override.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&default_user_config_path())
    }

    #[allow(dead_code)]
    pub fn user_config_path() -> PathBuf {
        default_user_config_path()
    }

    pub fn load_from(user_path: &std::path::Path) -> Result<Self, ConfigError> {
        let mut fig: Figment = Figment::from(Serialized::defaults(Config::default()));
        let system = noflex_core::system_dirs::config().join("config.toml");
        if system.exists() {
            fig = fig.merge(Toml::file(system));
        }
        if user_path.exists() {
            fig = fig.merge(Toml::file(user_path));
        }
        fig = fig.merge(Env::prefixed("NOFLEX_").filter_map(|key| {
            ENV_KEYS
                .iter()
                .find(|(env, _)| key.as_str().eq_ignore_ascii_case(env))
                .map(|(_, path)| (*path).into())
        }));
        if let Some(list) = std::env::var_os(PLUGIN_SEARCH_PATH_ENV).filter(|v| !v.is_empty()) {
            let paths: Vec<PathBuf> = std::env::split_paths(&list).collect();
            fig = fig.merge(Serialized::default("plugin.search_path", paths));
        }
        let config = fig
            .extract::<Config>()
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// HLS segments and other ffmpeg output: `cache.transcode_dir`, or
    /// `transcode/` under `cache_dir`.
    pub fn transcode_dir(&self) -> PathBuf {
        cache_subdir(&self.cache.transcode_dir, &self.cache_dir, "transcode")
    }

    /// Posters, backdrops, thumbnails, trickplay and channel logos:
    /// `cache.image_dir`, or `image/` under `cache_dir`.
    pub fn image_dir(&self) -> PathBuf {
        cache_subdir(&self.cache.image_dir, &self.cache_dir, "image")
    }

    /// `cache.embed_dir`, or `embed/` under `cache_dir`.
    pub fn embed_dir(&self) -> PathBuf {
        cache_subdir(&self.cache.embed_dir, &self.cache_dir, "embed")
    }

    /// Downloaded subtitles: `cache.subtitle_dir`, or `subtitles/` under
    /// `cache_dir`.
    pub fn subtitle_dir(&self) -> PathBuf {
        cache_subdir(&self.cache.subtitle_dir, &self.cache_dir, "subtitles")
    }

    /// How long a downloaded subtitle nobody plays is kept.
    pub fn subtitle_unused_for(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.cache.subtitle_keep_days) * 24 * 60 * 60)
    }

    /// Finished export copies: the transcode cache when one is configured, or
    /// `downloads/` under `data_dir`.
    pub fn download_dir(&self) -> PathBuf {
        if self.cache.transcode_dir.is_empty() {
            self.data_dir.join("downloads")
        } else {
            PathBuf::from(&self.cache.transcode_dir)
        }
    }

    /// Where optimized versions go until an administrator picks somewhere else.
    pub fn optimized_dir(&self) -> PathBuf {
        self.transcoder
            .optimize
            .dir
            .clone()
            .unwrap_or_else(|| self.data_dir.join("optimized"))
    }

    /// Live TV recordings, always under `data_dir`.
    pub fn recordings_dir(&self) -> PathBuf {
        self.data_dir.join("recordings")
    }

    fn validate(&self) -> Result<(), ConfigError> {
        use noflex_core::{VideoCodec, VideoProfile};
        for rung in &self.transcoder.ladder {
            let matches = match rung.video_codec {
                VideoCodec::H264 => matches!(
                    rung.video_profile,
                    VideoProfile::H264High
                        | VideoProfile::H264Main
                        | VideoProfile::H264Baseline
                        | VideoProfile::H264ConstrainedBaseline
                ),
                VideoCodec::H265 => rung.video_profile == VideoProfile::HevcMain,
                VideoCodec::Av1 => rung.video_profile == VideoProfile::Av1Main,
                _ => {
                    return Err(ConfigError::Invalid {
                        field: "transcoder.ladder",
                        reason: format!("can't encode to {:?}", rung.video_codec),
                    });
                }
            };
            if !matches {
                return Err(ConfigError::Invalid {
                    field: "transcoder.ladder",
                    reason: format!(
                        "{:?} can't encode at {:?}; encodes are 8-bit 4:2:0",
                        rung.video_codec, rung.video_profile
                    ),
                });
            }
            if rung.max_pixels == 0 {
                return Err(ConfigError::Invalid {
                    field: "transcoder.ladder",
                    reason: "max_pixels must be above 0".to_string(),
                });
            }
        }
        Ok(())
    }
}

/// An empty `configured` falls back to `<cache_dir>/<name>`.
fn cache_subdir(configured: &str, cache_dir: &std::path::Path, name: &str) -> PathBuf {
    if configured.is_empty() {
        cache_dir.join(name)
    } else {
        PathBuf::from(configured)
    }
}

/// The config file to read in place of the user-tier one, for a service or
/// container that keeps it elsewhere.
pub const CONFIG_FILE_ENV: &str = "NOFLEX_CONFIG_FILE";

/// `plugin.search_path` as the platform's path list (`:`-separated, `;` on
/// Windows), replacing the default tiers. The one array with a variable, since
/// a service manager sets it.
pub const PLUGIN_SEARCH_PATH_ENV: &str = "NOFLEX_PLUGIN_SEARCH_PATH";

/// `NOFLEX_<NAME>` → config key. Explicit because keys themselves
/// contain `_`, so splitting the name on `_` can't recover the nesting
/// (`LIVE_TV_RECORDING_CONCURRENCY` would become
/// `live.tv.recording.concurrency`). Arrays and maps are TOML-only, but for
/// [`PLUGIN_SEARCH_PATH_ENV`]; unlisted `NOFLEX_*` variables are ignored.
pub const ENV_KEYS: &[(&str, &str)] = &[
    ("BIND", "bind"),
    ("PUBLIC_URL", "public_url"),
    ("DATA_DIR", "data_dir"),
    ("CACHE_DIR", "cache_dir"),
    ("LOG_LEVEL", "log.level"),
    ("LOG_FILE", "log.file"),
    ("CROSS_ORIGIN", "cross_origin"),
    ("CAST_SDK", "cast_sdk"),
    ("TRANSCODER_HWACCEL", "transcoder.hwaccel"),
    ("TRANSCODER_FALLBACK", "transcoder.fallback"),
    ("TRANSCODER_TONEMAP_CURVE", "transcoder.tonemap_curve"),
    ("TRANSCODER_OPTIMIZE_ENABLED", "transcoder.optimize.enabled"),
    ("TRANSCODER_OPTIMIZE_DIR", "transcoder.optimize.dir"),
    (
        "TRANSCODER_OPTIMIZE_MAX_STORAGE_GB",
        "transcoder.optimize.max_storage_gb",
    ),
    (
        "TRANSCODER_OPTIMIZE_MAX_VERSION_GB",
        "transcoder.optimize.max_version_gb",
    ),
    (
        "TRANSCODER_OPTIMIZE_MAX_CPU_PERCENT",
        "transcoder.optimize.max_cpu_percent",
    ),
    (
        "TRANSCODER_OPTIMIZE_PLAN_INTERVAL_SECS",
        "transcoder.optimize.plan_interval_secs",
    ),
    (
        "TRANSCODER_OPTIMIZE_LOOKAHEAD_EPISODES",
        "transcoder.optimize.lookahead_episodes",
    ),
    (
        "TRANSCODER_OPTIMIZE_RECOMMENDATIONS",
        "transcoder.optimize.recommendations",
    ),
    ("PLUGIN_ASSETS_DIR", "plugin.assets_dir"),
    ("SCANNER_DEBOUNCE_MS", "scanner.debounce_ms"),
    ("SCANNER_CONCURRENCY", "scanner.concurrency"),
    ("SEARCH_DEVICE", "search.device"),
    ("SEARCH_THREADS", "search.threads"),
    ("SEARCH_BATCH_SIZE", "search.batch_size"),
    ("SEARCH_BUSY_PERCENT", "search.busy_percent"),
    ("SEARCH_ONNXRUNTIME_LIBRARY", "search.onnxruntime_library"),
    (
        "LIVE_TV_RECORDING_CONCURRENCY",
        "live_tv.recording_concurrency",
    ),
    ("BACKUP_DIR", "backup.dir"),
    ("BACKUP_INTERVAL_HOURS", "backup.interval_hours"),
    ("BACKUP_KEEP", "backup.keep"),
    ("CACHE_TRANSCODE_DIR", "cache.transcode_dir"),
    ("CACHE_IMAGE_DIR", "cache.image_dir"),
    ("CACHE_EMBED_DIR", "cache.embed_dir"),
    ("CACHE_SUBTITLE_DIR", "cache.subtitle_dir"),
    ("CACHE_SUBTITLE_KEEP_DAYS", "cache.subtitle_keep_days"),
    (
        "EXTERNAL_AUTH_AUTO_PROVISION",
        "external_auth.auto_provision",
    ),
];

fn default_user_config_path() -> PathBuf {
    if let Some(path) = std::env::var_os(CONFIG_FILE_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(path);
    }
    user_config_dir().map_or_else(
        || PathBuf::from("./config.toml"),
        |dir| dir.join("config.toml"),
    )
}

/// `$XDG_CONFIG_HOME/noflex`, or `~/.config/noflex` without it. `None` when
/// neither says where home is.
fn user_config_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::UserDirs::new().map(|u| u.home_dir().join(".config")))?;
    Some(config.join("noflex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let c = Config::default();
        assert!(c.bind.contains("28793"));
        assert_eq!(c.scanner.debounce_ms, 500);
        assert!(!c.plugin.search_path.is_empty());
    }

    // The only test that touches the process environment, so no other
    // test in this binary races its reads.
    #[test]
    fn load_reads_the_named_file_then_env() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("elsewhere.toml");
        std::fs::write(&file, "cross_origin = false\nbind = \"127.0.0.1:1\"\n").expect("write");
        let plugins = std::env::join_paths(["/srv/a", "/srv/b"]).expect("path list");
        let vars = [
            ("NOFLEX_CONFIG_FILE", file.as_os_str()),
            ("NOFLEX_PLUGIN_SEARCH_PATH", plugins.as_os_str()),
            ("NOFLEX_BIND", "0.0.0.0:9999".as_ref()),
            ("NOFLEX_PUBLIC_URL", "https://media.example".as_ref()),
            ("NOFLEX_LIVE_TV_RECORDING_CONCURRENCY", "7".as_ref()),
            ("NOFLEX_SCANNER_DEBOUNCE_MS", "123".as_ref()),
            ("NOFLEX_EXTERNAL_AUTH_AUTO_PROVISION", "true".as_ref()),
            ("NOFLEX_CACHE_IMAGE_DIR", "/srv/images".as_ref()),
            ("NOFLEX_NOT_A_KEY", "ignored".as_ref()),
        ];
        // SAFETY: no other test in this binary reads or writes the environment.
        unsafe {
            for (key, value) in vars {
                std::env::set_var(key, value);
            }
        }
        let loaded = Config::load();
        unsafe {
            for (key, _) in vars {
                std::env::remove_var(key);
            }
        }
        let c = loaded.expect("load");
        assert!(!c.cross_origin, "read from NOFLEX_CONFIG_FILE");
        assert_eq!(c.bind, "0.0.0.0:9999", "env outranks the file");
        assert_eq!(
            c.plugin.search_path,
            [PathBuf::from("/srv/a"), PathBuf::from("/srv/b")]
        );
        assert_eq!(c.public_url.as_deref(), Some("https://media.example"));
        assert_eq!(c.live_tv.recording_concurrency, 7);
        assert_eq!(c.scanner.debounce_ms, 123);
        assert!(c.external_auth.auto_provision);
        assert_eq!(c.cache.image_dir, "/srv/images");
    }

    /// Service units, images and installers configure the server through its
    /// environment, and a misspelled variable is silently ignored.
    #[test]
    fn packaging_sets_only_variables_the_server_reads() {
        fn files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut paths = Vec::new();
        // What installs and runs the server, not the scripts that build it.
        for dir in [
            "nix",
            "packaging/linux",
            "packaging/container",
            "packaging/windows",
        ] {
            files(&root.join(dir), &mut paths);
        }
        let known = |name: &str| {
            name == CONFIG_FILE_ENV
                || name == PLUGIN_SEARCH_PATH_ENV
                || name == "NOFLEX_LOG"
                || ENV_KEYS
                    .iter()
                    .any(|(env, _)| name.strip_prefix("NOFLEX_") == Some(env))
        };
        let mut unknown = Vec::new();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (at, _) in text.match_indices("NOFLEX_") {
                let name: String = text[at..]
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                    .collect();
                if !known(&name) {
                    unknown.push(format!("{} sets {name}", path.display()));
                }
            }
        }
        assert!(unknown.is_empty(), "{}", unknown.join("\n"));
    }

    #[test]
    fn plugin_override_fields_are_optional() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[plugin.overrides.tmdb]\npriorities.metadata.movie = 200\n\n[plugin.overrides.tvdb]\ndisabled = true\n",
        )
        .expect("write config");
        let figment =
            Figment::from(Serialized::defaults(Config::default())).merge(Toml::file(&path));
        let c = figment.extract::<Config>().expect("parse overrides");
        assert_eq!(
            c.plugin.overrides["tmdb"].priorities["metadata"]["movie"],
            200
        );
        assert!(!c.plugin.overrides["tmdb"].disabled);
        assert!(c.plugin.overrides["tvdb"].disabled);
    }

    #[test]
    fn env_keys_cover_every_scalar_setting() {
        fn leaves(prefix: &str, value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, child) in map {
                        let path = if prefix.is_empty() {
                            key.clone()
                        } else {
                            format!("{prefix}.{key}")
                        };
                        leaves(&path, child, out);
                    }
                }
                serde_json::Value::Array(_) => {}
                _ => out.push(prefix.to_owned()),
            }
        }
        let mut paths = Vec::new();
        leaves(
            "",
            &serde_json::to_value(Config::default()).expect("serialize defaults"),
            &mut paths,
        );
        for path in paths {
            let env = path.replace('.', "_").to_ascii_uppercase();
            assert!(
                ENV_KEYS.contains(&(env.as_str(), path.as_str())),
                "no NOFLEX_{env} mapping for `{path}`"
            );
        }
        assert!(
            !ENV_KEYS
                .iter()
                .any(|(_, path)| path.starts_with("plugin.overrides"))
        );
    }
}
