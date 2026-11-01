// SPDX-License-Identifier: AGPL-3.0-only
//
// Typed runtime configuration. Loaded via figment from
// $XDG_CONFIG_HOME/noflex/config.toml (with /etc/noflex/config.toml as
// a system-tier fallback) and overridden by `NOFLEX_*` env vars (see
// [`ENV_KEYS`]). The
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
    #[allow(dead_code)]
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
    pub log: LogConfig,
    pub transcoder: TranscoderConfig,
    pub plugin: PluginConfig,
    pub scanner: ScannerConfig,
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranscodeFallback {
    #[default]
    Auto,
    Software,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LadderEntry {
    pub max_height: u32,
    pub video_codec: noflex_core::VideoCodec,
    pub audio_codec: noflex_core::AudioCodec,
    pub container: String,
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub search_path: Vec<PathBuf>,
    /// Root directory the `host-assets.read-file` import serves
    /// (embedding model weights, tokenizers, ...). Unset disables the
    /// import: guests get an `unavailable` plugin error.
    #[serde(default)]
    pub assets_dir: Option<PathBuf>,
    pub overrides: std::collections::HashMap<String, PluginOverride>,
    pub per_plugin_config: std::collections::HashMap<String, toml::Value>,
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
            bind: "127.0.0.1:8096".to_string(),
            public_url: None,
            data_dir,
            cache_dir,
            log: LogConfig {
                level: "info".to_string(),
                file: None,
            },
            transcoder: TranscoderConfig {
                hwaccel: HwAccel::Auto,
                fallback: TranscodeFallback::Auto,
                ladder: vec![LadderEntry {
                    max_height: 1080,
                    video_codec: noflex_core::VideoCodec::H264,
                    audio_codec: noflex_core::AudioCodec::Aac,
                    container: "mp4".to_string(),
                    bitrate_kbps: 8000,
                }],
            },
            plugin: PluginConfig {
                search_path: default_search_path(),
                assets_dir: None,
                overrides: std::collections::HashMap::new(),
                per_plugin_config: std::collections::HashMap::new(),
            },
            scanner: ScannerConfig {
                debounce_ms: 500,
                concurrency: 4,
            },
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
    out.push(PathBuf::from("/etc/noflex/plugins"));
    out.push(PathBuf::from("/var/lib/noflex/plugins"));
    if let Some(home) = directories::UserDirs::new().and_then(|u| u.home_dir().to_path_buf().into())
    {
        out.push(home.join(".config").join("noflex").join("plugins"));
    }
    out
}

impl Config {
    /// Load config from the standard search path + env-var overrides.
    /// The system-tier TOML (`/etc/noflex/config.toml`) is consulted
    /// first, then the user-tier
    /// (`$XDG_CONFIG_HOME/noflex/config.toml` or
    /// `~/.config/noflex/config.toml`), then `NOFLEX_*` env vars
    /// override.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&default_user_config_path())
    }

    #[allow(dead_code)]
    pub fn user_config_path() -> PathBuf {
        default_user_config_path()
    }

    pub fn load_from(user_path: &std::path::Path) -> Result<Self, ConfigError> {
        let mut fig: Figment = Figment::from(Serialized::defaults(Config::default()));
        if std::path::Path::new("/etc/noflex/config.toml").exists() {
            fig = fig.merge(Toml::file("/etc/noflex/config.toml"));
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
        fig.extract::<Config>()
            .map_err(|e| ConfigError::Parse(e.to_string()))
    }
}

/// `NOFLEX_<NAME>` → config key. Explicit because keys themselves
/// contain `_`, so splitting the name on `_` can't recover the nesting
/// (`LIVE_TV_RECORDING_CONCURRENCY` would become
/// `live.tv.recording.concurrency`). Arrays and maps are TOML-only;
/// unlisted `NOFLEX_*` variables are ignored.
pub const ENV_KEYS: &[(&str, &str)] = &[
    ("BIND", "bind"),
    ("PUBLIC_URL", "public_url"),
    ("DATA_DIR", "data_dir"),
    ("CACHE_DIR", "cache_dir"),
    ("LOG_LEVEL", "log.level"),
    ("LOG_FILE", "log.file"),
    ("TRANSCODER_HWACCEL", "transcoder.hwaccel"),
    ("TRANSCODER_FALLBACK", "transcoder.fallback"),
    ("PLUGIN_ASSETS_DIR", "plugin.assets_dir"),
    ("SCANNER_DEBOUNCE_MS", "scanner.debounce_ms"),
    ("SCANNER_CONCURRENCY", "scanner.concurrency"),
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
    (
        "EXTERNAL_AUTH_AUTO_PROVISION",
        "external_auth.auto_provision",
    ),
];

fn default_user_config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("noflex").join("config.toml");
    }
    if let Some(home) = directories::UserDirs::new().and_then(|u| u.home_dir().to_path_buf().into())
    {
        return home.join(".config").join("noflex").join("config.toml");
    }
    PathBuf::from("./config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let c = Config::default();
        assert!(c.bind.contains("8096"));
        assert_eq!(c.scanner.debounce_ms, 500);
        assert!(!c.plugin.search_path.is_empty());
    }

    // The only test that touches the process environment, so no other
    // test in this binary races its reads.
    #[test]
    fn load_from_nonexistent_path_uses_defaults_and_env() {
        let vars = [
            ("NOFLEX_BIND", "0.0.0.0:9999"),
            ("NOFLEX_PUBLIC_URL", "https://media.example"),
            ("NOFLEX_LIVE_TV_RECORDING_CONCURRENCY", "7"),
            ("NOFLEX_SCANNER_DEBOUNCE_MS", "123"),
            ("NOFLEX_EXTERNAL_AUTH_AUTO_PROVISION", "true"),
            ("NOFLEX_CACHE_IMAGE_DIR", "/srv/images"),
            ("NOFLEX_NOT_A_KEY", "ignored"),
        ];
        // SAFETY: no other test in this binary reads or writes the environment.
        unsafe {
            for (key, value) in vars {
                std::env::set_var(key, value);
            }
        }
        let loaded = Config::load_from(std::path::Path::new("/nonexistent/config.toml"));
        unsafe {
            for (key, _) in vars {
                std::env::remove_var(key);
            }
        }
        let c = loaded.expect("load");
        assert_eq!(c.bind, "0.0.0.0:9999");
        assert_eq!(c.public_url.as_deref(), Some("https://media.example"));
        assert_eq!(c.live_tv.recording_concurrency, 7);
        assert_eq!(c.scanner.debounce_ms, 123);
        assert!(c.external_auth.auto_provision);
        assert_eq!(c.cache.image_dir, "/srv/images");
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
