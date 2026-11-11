// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

use async_graphql::Enum;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::HostError;

const MANIFEST_VERSION: u32 = 1;
const CAPABILITIES: &[&str] = &[
    "assets.read",
    "inference.run",
    "background",
    "http.egress",
    "http.private-network",
    "secrets.read",
    "user-secrets.read",
    "user-secrets.write",
];

/// The `noflex:manifest` custom section payload, expected as UTF-8 TOML.
///
/// Embedded into the `.wasm` component by build tooling (`wasm-tools
/// custom-section add`). Reading it does **not** instantiate the component;
/// the host can reject incompatible plugins before paying instantiation cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub plugin_id: String,
    pub display_name: String,
    pub version: String,
    /// `MAJOR.MINOR`, e.g. `"0.2"`. Pre-1.0 the host requires an exact
    /// match with its own API version; see [`Self::check_api_compat`].
    pub noflex_api_version: String,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Event subscriptions for the notifier fan-out, named by
    /// `DomainEvent::kind_str` (work-added, scan-completed, ...). Empty
    /// means every event — the historical behavior. A non-empty list is
    /// only meaningful with the `notifier` hook; [`Self::check_events`]
    /// rejects anything else at load time.
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub http_egress_hosts: Vec<String>,
    #[serde(default)]
    pub secrets_read: Vec<String>,
    /// Per-user secret keys the plugin may read via
    /// `host-secrets.read-for-user` (tokens persisted at `user-link`
    /// time). Same allowlist semantics as `secrets_read`.
    #[serde(default)]
    pub user_secrets_read: Vec<String>,
    /// Per-user secret keys the plugin may write via
    /// `host-secrets.write-for-user` (e.g. refreshed OAuth tokens).
    /// Distinct from the read allowlist: reading a token back does not
    /// imply license to overwrite it.
    #[serde(default)]
    pub user_secrets_write: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Per-hook priority defaults. Server config can override per-WorkKind.
    #[serde(default)]
    pub priority: BTreeMap<String, i32>,
    /// Per-hook call deadline overrides in seconds (wall clock, measured
    /// from when the instance starts the call). Each entry applies to its
    /// own hook only; unlisted hooks keep the engine default.
    #[serde(default)]
    pub deadlines_secs: BTreeMap<String, u32>,
    /// Per-plugin memory cap override (MB). Default 64.
    #[serde(default)]
    pub memory_limit_mb: Option<u32>,
    /// Declarative background-task registrations. The schedule is part of
    /// the manifest so it is auditable before the plugin ever runs; the
    /// host invokes `background.run-task` on the plugin's single instance.
    /// Requires the `background` capability.
    #[serde(default)]
    pub tasks: Vec<TaskRegistration>,
    /// Embedding identity claim. Required when `hooks` includes
    /// `embedder`: the host namespaces stored vectors by `namespace`,
    /// records `model_id` alongside them, and rejects `embed` results
    /// whose vector length differs from `dimensions`.
    #[serde(default)]
    pub embedding: Option<EmbeddingClaim>,
    /// Settings an administrator can edit from the admin UI, each a
    /// top-level key of the plugin's config stanza (or, for a
    /// [`SettingKind::Secret`], a key of `secrets_read`). Validated by
    /// [`Self::check_contract`].
    #[serde(default)]
    pub settings: Vec<SettingSpec>,
}

/// One `[[settings]]` entry of a manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingSpec {
    /// A top-level key of the config stanza `host-config.read` returns,
    /// or for a secret, the key `host-secrets.read` takes.
    pub key: String,
    /// Shown beside the field. Sentence case.
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    pub kind: SettingKind,
    /// Whether the plugin can't work without a value. The host still loads
    /// it without one; an administrator just can't clear the last source.
    #[serde(default)]
    pub required: bool,
    /// The values a [`SettingKind::Choice`] allows, and only for that kind.
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    /// Handed to the guest when neither an administrator nor config.toml
    /// sets the key. Must match `kind`; never allowed for a secret.
    #[serde(default)]
    pub default: Option<toml::Value>,
}

/// What a plugin setting holds, which picks its form field and how a value
/// is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Enum)]
#[serde(rename_all = "lowercase")]
#[graphql(name = "PluginSettingKind")]
pub enum SettingKind {
    Text,
    /// An absolute `http` or `https` URL.
    Url,
    /// Read through `host-secrets.read`, never `host-config.read`, and
    /// never readable back.
    Secret,
    Boolean,
    /// A signed 64-bit integer.
    Integer,
    /// One of the setting's `choices`.
    Choice,
}

impl SettingSpec {
    /// Check `raw` against this setting's kind and return the value the
    /// guest gets: a TOML boolean or integer for those kinds, else a string.
    ///
    /// # Errors
    ///
    /// A client-safe reason when `raw` isn't a value of the kind.
    pub fn parse_value(&self, raw: &str) -> Result<toml::Value, String> {
        match self.kind {
            SettingKind::Text | SettingKind::Secret => Ok(toml::Value::String(raw.to_string())),
            SettingKind::Url => {
                let is_web = url::Url::parse(raw.trim()).is_ok_and(|url| {
                    matches!(url.scheme(), "http" | "https") && url.host().is_some()
                });
                if !is_web {
                    return Err("must be an http or https URL".into());
                }
                Ok(toml::Value::String(raw.trim().to_string()))
            }
            SettingKind::Boolean => match raw {
                "true" => Ok(toml::Value::Boolean(true)),
                "false" => Ok(toml::Value::Boolean(false)),
                _ => Err("must be true or false".into()),
            },
            SettingKind::Integer => raw
                .trim()
                .parse::<i64>()
                .map(toml::Value::Integer)
                .map_err(|_| "must be a whole number".into()),
            SettingKind::Choice => {
                let choices = self.choices.as_deref().unwrap_or_default();
                if choices.iter().any(|c| c == raw) {
                    Ok(toml::Value::String(raw.to_string()))
                } else {
                    Err(format!("must be one of {}", choices.join(", ")))
                }
            }
        }
    }

    /// The manifest `default` in the string form clients and storage use.
    pub fn default_string(&self) -> Option<String> {
        self.default.as_ref().map(setting_string)
    }

    fn check(&self, manifest: &PluginManifest) -> Result<(), String> {
        let key = &self.key;
        if key.is_empty() || key.len() > crate::dispatch::MAX_SECRET_KEY_BYTES {
            return Err("a setting's key is empty or too long".into());
        }
        if self.label.trim().is_empty() {
            return Err(format!("setting `{key}` has no label"));
        }
        match (self.kind, &self.choices) {
            (SettingKind::Choice, Some(choices)) if !choices.is_empty() => {}
            (SettingKind::Choice, _) => {
                return Err(format!("choice setting `{key}` lists no choices"));
            }
            (_, Some(_)) => return Err(format!("setting `{key}` has choices but isn't a choice")),
            (_, None) => {}
        }
        if self.kind == SettingKind::Secret {
            if self.default.is_some() {
                return Err(format!("secret setting `{key}` has a default"));
            }
            if !manifest.secrets_read.iter().any(|k| k == key) {
                return Err(format!(
                    "secret setting `{key}` isn't declared in secrets_read"
                ));
            }
        }
        if let Some(default) = &self.default {
            let matches = match (self.kind, default) {
                (SettingKind::Boolean, toml::Value::Boolean(_))
                | (SettingKind::Integer, toml::Value::Integer(_)) => true,
                (
                    SettingKind::Text | SettingKind::Url | SettingKind::Choice,
                    toml::Value::String(s),
                ) => self.parse_value(s).is_ok(),
                _ => false,
            };
            if !matches {
                return Err(format!(
                    "setting `{key}` has a default that doesn't fit its kind"
                ));
            }
        }
        Ok(())
    }
}

/// A setting's value as clients see it and storage keeps it: a string as
/// itself, anything else in TOML syntax (`true`, `42`).
pub(crate) fn setting_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The `[embedding]` table of an embedder plugin's manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingClaim {
    /// Vector namespace stored alongside every embedding; queries embed
    /// into the same namespace. kebab-case by convention
    /// (e.g. `"potion-base-8m"`).
    pub namespace: String,
    /// Free-form model identifier persisted with each vector
    /// (e.g. `"minishlab/potion-base-8M"`).
    pub model_id: String,
    /// Dimensionality every returned vector must have.
    pub dimensions: u32,
}

/// One `[[tasks]]` entry: a named piece of periodic background work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRegistration {
    /// Guest-defined task name, handed verbatim to `run-task`.
    pub name: String,
    /// How often the host invokes the task. Sub-second intervals are
    /// allowed; the floor is the runtime's scheduling granularity.
    pub interval_ms: u64,
    /// Optional static payload passed to every scheduled invocation
    /// (JSON by convention). Dynamic payloads come from
    /// `host-tasks.enqueue`.
    #[serde(default)]
    pub payload: Option<String>,
}

/// True when `capabilities` declares `background` — the shared gate for
/// manifest `[[tasks]]` registration, the guest's `host-tasks.enqueue`
/// import, and host-side task enqueue.
pub(crate) fn declares_background(capabilities: &[String]) -> bool {
    capabilities.iter().any(|c| c == "background")
}
/// The event names a manifest's `events` list may subscribe to. Mirrors
/// `DomainEvent::kind_str` one-for-one; kept as a literal table so a bad
/// entry is a load-time rejection instead of a silent never-delivered
/// subscription.
const NOTIFY_EVENT_KINDS: &[&str] = &[
    "work-added",
    "work-played",
    "scan-started",
    "scan-completed",
    "favorite",
    "progress-at-stop",
    "playback-started",
    "playback-stopped",
    "custom",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PriorityKey {
    pub hook: HookKind,
    pub work_kind: Option<WorkKindFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Enum)]
pub enum HookKind {
    Metadata,
    Image,
    Subtitle,
    PostScan,
    Notifier,
    UserLink,
    Auth,
    Scrobbler,
    IntroDetector,
    Embedder,
    Background,
}

impl HookKind {
    pub const ALL: &'static [HookKind] = &[
        HookKind::Metadata,
        HookKind::Image,
        HookKind::Subtitle,
        HookKind::PostScan,
        HookKind::Notifier,
        HookKind::UserLink,
        HookKind::Auth,
        HookKind::Scrobbler,
        HookKind::IntroDetector,
        HookKind::Embedder,
        HookKind::Background,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            HookKind::Metadata => "metadata",
            HookKind::Image => "image",
            HookKind::Subtitle => "subtitle",
            HookKind::PostScan => "post-scan",
            HookKind::Notifier => "notifier",
            HookKind::UserLink => "user-link",
            HookKind::Auth => "external-auth",
            HookKind::Scrobbler => "scrobbler",
            HookKind::IntroDetector => "intro-detector",
            HookKind::Embedder => "embedder",
            HookKind::Background => "background",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "metadata" => Some(HookKind::Metadata),
            "image" => Some(HookKind::Image),
            "subtitle" => Some(HookKind::Subtitle),
            "post-scan" | "post_scan" => Some(HookKind::PostScan),
            "notifier" => Some(HookKind::Notifier),
            "user-link" | "user_link" => Some(HookKind::UserLink),
            "external-auth" | "external_auth" => Some(HookKind::Auth),
            "scrobbler" => Some(HookKind::Scrobbler),
            "intro-detector" | "intro_detector" => Some(HookKind::IntroDetector),
            "embedder" => Some(HookKind::Embedder),
            "background" => Some(HookKind::Background),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Enum)]
pub enum WorkKindFilter {
    Movie,
    Series,
    Season,
    Episode,
    Album,
    Track,
    Artist,
    AudioBook,
    Photo,
}

impl WorkKindFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkKindFilter::Movie => "movie",
            WorkKindFilter::Series => "series",
            WorkKindFilter::Season => "season",
            WorkKindFilter::Episode => "episode",
            WorkKindFilter::Album => "album",
            WorkKindFilter::Track => "track",
            WorkKindFilter::Artist => "artist",
            WorkKindFilter::AudioBook => "audio-book",
            WorkKindFilter::Photo => "photo",
        }
    }
}

impl std::fmt::Display for WorkKindFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum ManifestParseError {
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("utf-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("unknown api version `{0}` (expected MAJOR.MINOR like \"0.1\")")]
    BadApiVersion(String),
}

impl PluginManifest {
    /// Parse manifest TOML from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ManifestParseError> {
        let s = std::str::from_utf8(bytes)?;
        let m: PluginManifest = toml::from_str(s)?;
        Ok(m)
    }

    /// Returns (major, minor) for the declared `noflex_api_version`.
    pub fn api_major_minor(&self) -> Result<(u32, u32), ManifestParseError> {
        let mut it = self.noflex_api_version.split('.');
        let major = it
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| ManifestParseError::BadApiVersion(self.noflex_api_version.clone()))?;
        let minor = it
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| ManifestParseError::BadApiVersion(self.noflex_api_version.clone()))?;
        if it.next().is_some() {
            return Err(ManifestParseError::BadApiVersion(
                self.noflex_api_version.clone(),
            ));
        }
        Ok((major, minor))
    }

    /// Validate fields that define the host/guest boundary before compilation.
    pub fn check_contract(&self, path: &Path) -> Result<(), HostError> {
        let reject = |reason: String| HostError::Manifest {
            path: path.to_path_buf(),
            reason,
        };
        if self.manifest_version != MANIFEST_VERSION {
            return Err(reject(format!(
                "unsupported manifest_version {} (expected {MANIFEST_VERSION})",
                self.manifest_version
            )));
        }
        validate_plugin_id(&self.plugin_id).map_err(|reason| reject(reason.to_string()))?;
        for hook in &self.hooks {
            let Some(kind) = HookKind::parse(hook) else {
                return Err(reject(format!("unsupported hook `{hook}`")));
            };
            if kind == HookKind::Subtitle {
                return Err(reject(format!("unsupported hook `{hook}`")));
            }
        }
        for (hook, seconds) in &self.deadlines_secs {
            let Some(kind) = HookKind::parse(hook) else {
                return Err(reject(format!(
                    "deadline configured for unsupported hook `{hook}`"
                )));
            };
            if kind == HookKind::Subtitle || *seconds == 0 {
                return Err(reject(format!("invalid deadline for hook `{hook}`")));
            }
        }
        if self.memory_limit_mb == Some(0) {
            return Err(reject("memory_limit_mb must be greater than zero".into()));
        }
        self.check_capabilities(path)?;
        for (index, setting) in self.settings.iter().enumerate() {
            if self.settings[..index].iter().any(|s| s.key == setting.key) {
                return Err(reject(format!("duplicate setting `{}`", setting.key)));
            }
            setting.check(self).map_err(reject)?;
        }
        Ok(())
    }

    /// The `[[settings]]` entry for `key`, if the manifest declares one.
    pub fn setting(&self, key: &str) -> Option<&SettingSpec> {
        self.settings.iter().find(|s| s.key == key)
    }

    fn check_capabilities(&self, path: &Path) -> Result<(), HostError> {
        let reject = |reason: String| HostError::Manifest {
            path: path.to_path_buf(),
            reason,
        };
        for (index, capability) in self.capabilities.iter().enumerate() {
            if !CAPABILITIES.contains(&capability.as_str()) {
                return Err(reject(format!("unsupported capability `{capability}`")));
            }
            if self.capabilities[..index].contains(capability) {
                return Err(reject(format!("duplicate capability `{capability}`")));
            }
        }
        let has = |capability: &str| self.capabilities.iter().any(|item| item == capability);
        if !self.http_egress_hosts.is_empty() && !has("http.egress") {
            return Err(reject(
                "http_egress_hosts declared without the `http.egress` capability".into(),
            ));
        }
        if has("http.private-network") && !has("http.egress") {
            return Err(reject(
                "`http.private-network` requires the `http.egress` capability".into(),
            ));
        }
        for (keys, capability, field) in [
            (&self.secrets_read, "secrets.read", "secrets_read"),
            (
                &self.user_secrets_read,
                "user-secrets.read",
                "user_secrets_read",
            ),
            (
                &self.user_secrets_write,
                "user-secrets.write",
                "user_secrets_write",
            ),
        ] {
            if !keys.is_empty() && !has(capability) {
                return Err(reject(format!(
                    "{field} declared without the `{capability}` capability"
                )));
            }
            for key in keys {
                if key.is_empty() || key.len() > crate::dispatch::MAX_SECRET_KEY_BYTES {
                    return Err(reject(format!("invalid secret key in {field}")));
                }
            }
        }
        Ok(())
    }

    /// Validate the `[[tasks]]` registrations: non-empty unique names,
    /// non-zero intervals, and the `background` capability declared. Task
    /// registration is declarative precisely so capability review stays
    /// static, so an undeclared task schedule is a load-time rejection.
    pub fn check_tasks(&self, path: &Path) -> Result<(), HostError> {
        let reject = |reason: String| HostError::Manifest {
            path: path.to_path_buf(),
            reason,
        };
        if self.tasks.is_empty() {
            return Ok(());
        }
        if !declares_background(&self.capabilities) {
            return Err(reject(
                "[[tasks]] registered without the `background` capability".into(),
            ));
        }
        for (i, task) in self.tasks.iter().enumerate() {
            if task.name.is_empty() {
                return Err(reject(format!("tasks[{i}] has an empty name")));
            }
            if task.interval_ms == 0 {
                return Err(reject(format!("task `{}` has interval_ms = 0", task.name)));
            }
            if task
                .payload
                .as_ref()
                .is_some_and(|payload| payload.len() > crate::dispatch::MAX_TASK_PAYLOAD_BYTES)
            {
                return Err(reject(format!("task `{}` payload is too large", task.name)));
            }
            if self.tasks[..i].iter().any(|t| t.name == task.name) {
                return Err(reject(format!("duplicate task name `{}`", task.name)));
            }
        }
        Ok(())
    }

    /// Validate the `[embedding]` claim against the declared hooks: the
    /// `embedder` hook requires the table (the host cannot namespace or
    /// dimension-check vectors without it), and a table without the hook
    /// is a manifest bug worth rejecting at load.
    pub fn check_embedding(&self, path: &Path) -> Result<(), HostError> {
        let reject = |reason: String| HostError::Manifest {
            path: path.to_path_buf(),
            reason,
        };
        let declares_embedder = self
            .hooks
            .iter()
            .any(|h| HookKind::parse(h) == Some(HookKind::Embedder));
        match (&self.embedding, declares_embedder) {
            (Some(claim), true) => {
                if claim.namespace.is_empty() {
                    return Err(reject("[embedding] namespace is empty".into()));
                }
                if claim.model_id.is_empty() {
                    return Err(reject("[embedding] model_id is empty".into()));
                }
                if claim.dimensions == 0 {
                    return Err(reject("[embedding] dimensions = 0".into()));
                }
                Ok(())
            }
            (None, true) => Err(reject(
                "embedder hook declared without an [embedding] table".into(),
            )),
            (Some(_), false) => Err(reject(
                "[embedding] table declared without the embedder hook".into(),
            )),
            (None, false) => Ok(()),
        }
    }

    /// Validate the `events` subscription list: every entry must be a
    /// known notify-event name, and a non-empty list only makes sense on
    /// a plugin that exports `notifier` — otherwise the plugin asked to
    /// filter events it can never receive, which is a manifest bug worth
    /// rejecting at load.
    pub fn check_events(&self, path: &Path) -> Result<(), HostError> {
        let reject = |reason: String| HostError::Manifest {
            path: path.to_path_buf(),
            reason,
        };
        if self.events.is_empty() {
            return Ok(());
        }
        let declares_notifier = self
            .hooks
            .iter()
            .any(|h| HookKind::parse(h) == Some(HookKind::Notifier));
        if !declares_notifier {
            return Err(reject("events declared without the notifier hook".into()));
        }
        for event in &self.events {
            if !NOTIFY_EVENT_KINDS.contains(&event.as_str()) {
                return Err(reject(format!(
                    "unknown event `{event}` (expected one of: {})",
                    NOTIFY_EVENT_KINDS.join(", ")
                )));
            }
        }
        Ok(())
    }

    /// True when this plugin subscribes to the event named by
    /// `DomainEvent::kind_str`. An empty `events` list subscribes to
    /// everything.
    pub fn subscribed_to(&self, kind: &str) -> bool {
        self.events.is_empty() || self.events.iter().any(|e| e == kind)
    }

    /// Validate that this manifest names the host's exact plugin API version.
    /// Mixed host/guest versions are unsupported because generated component
    /// bindings move together even when a change looks additive.
    pub fn check_api_compat(&self, path: &Path) -> Result<(), HostError> {
        self.api_major_minor().map_err(|e| HostError::Manifest {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let expected = format!(
            "{}.{}",
            noflex_plugin_wit::API_MAJOR,
            noflex_plugin_wit::API_MINOR
        );
        if self.noflex_api_version != expected {
            return Err(HostError::ApiVersionMismatch {
                expected,
                plugin_api: self.noflex_api_version.clone(),
            });
        }
        Ok(())
    }
}

pub(crate) fn validate_plugin_id(id: &str) -> Result<(), &'static str> {
    let mut components = Path::new(id).components();
    if id.is_empty()
        || components.next() != Some(Component::Normal(std::ffi::OsStr::new(id)))
        || components.next().is_some()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("plugin_id must be a non-empty safe ASCII path component");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_manifest() {
        let toml = r#"
            manifest_version = 1
            plugin_id = "tmdb"
            display_name = "TMDb"
            version = "0.1.0"
            noflex_api_version = "0.2"
            hooks = ["metadata", "image"]
        "#;
        let m = PluginManifest::from_bytes(toml.as_bytes()).expect("parses");
        assert_eq!(m.plugin_id, "tmdb");
        assert_eq!(m.hooks, vec!["metadata".to_string(), "image".to_string()]);
        assert_eq!(m.api_major_minor().unwrap(), (0, 2));
    }

    #[test]
    fn rejects_bad_api_version() {
        let toml = r#"
            manifest_version = 1
            plugin_id = "tmdb"
            display_name = "TMDb"
            version = "0.1.0"
            noflex_api_version = "not-a-version"
            hooks = []
        "#;
        let m = PluginManifest::from_bytes(toml.as_bytes()).expect("toml parses");
        assert!(matches!(
            m.api_major_minor(),
            Err(ManifestParseError::BadApiVersion(_))
        ));
    }

    #[test]
    fn hook_round_trip() {
        for h in HookKind::ALL {
            assert_eq!(HookKind::parse(h.as_str()), Some(*h));
        }
    }

    #[test]
    fn plugin_ids_are_safe_single_components() {
        for valid in ["tmdb", "media-db.v2", "provider_1"] {
            assert_eq!(validate_plugin_id(valid), Ok(()));
        }
        for invalid in ["", ".", "..", "../escape", "a/b", "a\\b", "💥"] {
            assert!(validate_plugin_id(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn contract_rejects_schema_and_unknown_hooks() {
        let parse = |version, hooks| {
            PluginManifest::from_bytes(
                format!(
                    "manifest_version = {version}\nplugin_id = \"p\"\ndisplay_name = \"P\"\n\
                     version = \"1\"\nnoflex_api_version = \"{}.{}\"\nhooks = [{hooks}]",
                    noflex_plugin_wit::API_MAJOR,
                    noflex_plugin_wit::API_MINOR,
                )
                .as_bytes(),
            )
            .unwrap()
        };
        assert!(
            parse(2, "\"metadata\"")
                .check_contract(Path::new("p.wasm"))
                .is_err()
        );
        assert!(
            parse(1, "\"future-hook\"")
                .check_contract(Path::new("p.wasm"))
                .is_err()
        );
        assert!(
            parse(1, "\"subtitle\"")
                .check_contract(Path::new("p.wasm"))
                .is_err()
        );
        // The pre-0.8 `auth` spelling is gone; only `external-auth` names
        // the hook.
        assert!(
            parse(1, "\"auth\"")
                .check_contract(Path::new("p.wasm"))
                .is_err()
        );
        parse(1, "\"external-auth\"")
            .check_contract(Path::new("p.wasm"))
            .expect("external-auth is a supported hook");
    }

    #[test]
    fn api_version_is_exact() {
        let expected = format!(
            "{}.{}",
            noflex_plugin_wit::API_MAJOR,
            noflex_plugin_wit::API_MINOR
        );
        let base = |api: &str| {
            PluginManifest::from_bytes(
                format!(
                    "manifest_version = 1\nplugin_id = \"p\"\ndisplay_name = \"P\"\n\
                     version = \"1\"\nnoflex_api_version = \"{api}\"\nhooks = [\"metadata\"]"
                )
                .as_bytes(),
            )
            .unwrap()
        };
        base(&expected)
            .check_api_compat(Path::new("p.wasm"))
            .unwrap();
        assert!(
            base(&format!("{expected}.0"))
                .check_api_compat(Path::new("p.wasm"))
                .is_err()
        );
    }

    #[test]
    fn parses_tasks() {
        let toml = r#"
            manifest_version = 1
            plugin_id = "intro"
            display_name = "Intro Detector"
            version = "0.1.0"
            noflex_api_version = "0.2"
            hooks = ["background"]
            capabilities = ["background"]

            [[tasks]]
            name = "sweep"
            interval_ms = 50

            [[tasks]]
            name = "nightly"
            interval_ms = 86_400_000
            payload = "{\"mode\":\"full\"}"
        "#;
        let m = PluginManifest::from_bytes(toml.as_bytes()).expect("parses");
        assert_eq!(m.tasks.len(), 2);
        assert_eq!(m.tasks[0].name, "sweep");
        assert_eq!(m.tasks[0].interval_ms, 50);
        assert_eq!(m.tasks[0].payload, None);
        assert_eq!(m.tasks[1].payload.as_deref(), Some("{\"mode\":\"full\"}"));
        m.check_tasks(Path::new("intro.wasm")).expect("valid");
    }

    #[test]
    fn secret_allowlists_require_matching_capabilities() {
        let manifest = PluginManifest::from_bytes(
            br#"
                manifest_version = 1
                plugin_id = "p"
                display_name = "P"
                version = "0.1.0"
                noflex_api_version = "0.2"
                secrets_read = ["token"]
            "#,
        )
        .unwrap();
        assert!(manifest.check_contract(Path::new("p.wasm")).is_err());

        let manifest = PluginManifest::from_bytes(
            br#"
                manifest_version = 1
                plugin_id = "p"
                display_name = "P"
                version = "0.1.0"
                noflex_api_version = "0.2"
                capabilities = ["user-secrets.write"]
                user_secrets_write = ["token"]
            "#,
        )
        .unwrap();
        manifest.check_contract(Path::new("p.wasm")).unwrap();
    }

    #[test]
    fn tasks_require_background_capability() {
        let toml = r#"
            manifest_version = 1
            plugin_id = "intro"
            display_name = "Intro Detector"
            version = "0.1.0"
            noflex_api_version = "0.2"

            [[tasks]]
            name = "sweep"
            interval_ms = 1000
        "#;
        let m = PluginManifest::from_bytes(toml.as_bytes()).expect("parses");
        assert!(m.check_tasks(Path::new("intro.wasm")).is_err());
    }

    #[test]
    fn embedding_claim_pairs_with_embedder_hook() {
        let toml = r#"
            manifest_version = 1
            plugin_id = "potion"
            display_name = "Potion"
            version = "0.1.0"
            noflex_api_version = "0.2"
            hooks = ["embedder"]
            capabilities = ["assets.read"]

            [embedding]
            namespace = "potion-base-8m"
            model_id = "minishlab/potion-base-8M"
            dimensions = 256
        "#;
        let m = PluginManifest::from_bytes(toml.as_bytes()).expect("parses");
        m.check_embedding(Path::new("potion.wasm")).expect("valid");
    }

    #[test]
    fn embedding_requires_hook_and_table() {
        let base = |extra: &str| {
            format!(
                "manifest_version = 1\nplugin_id = \"p\"\ndisplay_name = \"P\"\n\
                 version = \"0.1.0\"\nnoflex_api_version = \"0.2\"\n{extra}"
            )
        };
        let no_table =
            PluginManifest::from_bytes(base("hooks = [\"embedder\"]").as_bytes()).expect("parses");
        assert!(no_table.check_embedding(Path::new("p.wasm")).is_err());

        let no_hook = PluginManifest::from_bytes(
            base("hooks = []\n[embedding]\nnamespace = \"n\"\nmodel_id = \"m\"\ndimensions = 8")
                .as_bytes(),
        )
        .expect("parses");
        assert!(no_hook.check_embedding(Path::new("p.wasm")).is_err());

        let zero_dims = PluginManifest::from_bytes(
            base(
                "hooks = [\"embedder\"]\n[embedding]\nnamespace = \"n\"\nmodel_id = \"m\"\ndimensions = 0",
            )
            .as_bytes(),
        )
        .expect("parses");
        assert!(zero_dims.check_embedding(Path::new("p.wasm")).is_err());
    }

    #[test]
    fn events_default_to_everything() {
        let toml = r#"
            manifest_version = 1
            plugin_id = "gotify"
            display_name = "Gotify"
            version = "0.1.0"
            noflex_api_version = "0.2"
            hooks = ["notifier"]
        "#;
        let m = PluginManifest::from_bytes(toml.as_bytes()).expect("parses");
        assert!(m.events.is_empty());
        m.check_events(Path::new("gotify.wasm")).expect("valid");
        assert!(m.subscribed_to("work-played"));
        assert!(m.subscribed_to("scan-completed"));
    }

    #[test]
    fn events_filter_subscriptions() {
        let toml = r#"
            manifest_version = 1
            plugin_id = "gotify"
            display_name = "Gotify"
            version = "0.1.0"
            noflex_api_version = "0.2"
            hooks = ["notifier"]
            events = ["scan-started", "scan-completed"]
        "#;
        let m = PluginManifest::from_bytes(toml.as_bytes()).expect("parses");
        m.check_events(Path::new("gotify.wasm")).expect("valid");
        assert!(m.subscribed_to("scan-started"));
        assert!(!m.subscribed_to("work-played"));
    }

    #[test]
    fn events_require_known_names_and_notifier_hook() {
        let base = |extra: &str| {
            format!(
                "manifest_version = 1\nplugin_id = \"p\"\ndisplay_name = \"P\"\n\
                 version = \"0.1.0\"\nnoflex_api_version = \"0.2\"\n{extra}"
            )
        };
        let unknown = PluginManifest::from_bytes(
            base("hooks = [\"notifier\"]\nevents = [\"work-deleted\"]").as_bytes(),
        )
        .expect("parses");
        let err = unknown
            .check_events(Path::new("p.wasm"))
            .expect_err("unknown event name");
        assert!(err.to_string().contains("work-deleted"));

        let no_hook = PluginManifest::from_bytes(base("events = [\"scan-started\"]").as_bytes())
            .expect("parses");
        assert!(no_hook.check_events(Path::new("p.wasm")).is_err());
    }

    #[test]
    fn settings_are_checked_against_their_kind() {
        let check = |settings: &str| {
            PluginManifest::from_bytes(
                format!(
                    "manifest_version = 1\nplugin_id = \"p\"\ndisplay_name = \"P\"\n\
                     version = \"0.1.0\"\nnoflex_api_version = \"0.1\"\n\
                     capabilities = [\"secrets.read\"]\nsecrets_read = [\"token\"]\n{settings}"
                )
                .as_bytes(),
            )
            .unwrap()
            .check_contract(Path::new("p.wasm"))
        };
        check(
            r#"
            [[settings]]
            key = "url"
            label = "Endpoint"
            kind = "url"
            required = true
            default = "https://example.com/hook"
            [[settings]]
            key = "mode"
            label = "Mode"
            kind = "choice"
            choices = ["fast", "slow"]
            default = "fast"
            [[settings]]
            key = "retries"
            label = "Retries"
            kind = "integer"
            default = 3
            [[settings]]
            key = "token"
            label = "Token"
            kind = "secret"
            "#,
        )
        .expect("a valid schema");

        for (why, settings) in [
            (
                "duplicate key",
                "[[settings]]\nkey = \"a\"\nlabel = \"A\"\nkind = \"text\"\n\
                 [[settings]]\nkey = \"a\"\nlabel = \"A\"\nkind = \"boolean\"",
            ),
            (
                "choice without choices",
                "[[settings]]\nkey = \"a\"\nlabel = \"A\"\nkind = \"choice\"",
            ),
            (
                "choices on text",
                "[[settings]]\nkey = \"a\"\nlabel = \"A\"\nkind = \"text\"\nchoices = [\"x\"]",
            ),
            (
                "default outside the choices",
                "[[settings]]\nkey = \"a\"\nlabel = \"A\"\nkind = \"choice\"\n\
                 choices = [\"x\"]\ndefault = \"y\"",
            ),
            (
                "string default for an integer",
                "[[settings]]\nkey = \"a\"\nlabel = \"A\"\nkind = \"integer\"\ndefault = \"3\"",
            ),
            (
                "secret with a default",
                "[[settings]]\nkey = \"token\"\nlabel = \"T\"\nkind = \"secret\"\ndefault = \"x\"",
            ),
            (
                "secret outside secrets_read",
                "[[settings]]\nkey = \"other\"\nlabel = \"T\"\nkind = \"secret\"",
            ),
        ] {
            assert!(check(settings).is_err(), "accepted {why}");
        }
    }

    #[test]
    fn setting_values_parse_by_kind() {
        let spec = |kind, choices: Option<Vec<String>>| SettingSpec {
            key: "k".into(),
            label: "K".into(),
            description: None,
            kind,
            required: false,
            choices,
            default: None,
        };
        let url = spec(SettingKind::Url, None);
        assert!(url.parse_value("https://ntfy.example.com/noflex").is_ok());
        for bad in ["ftp://example.com", "example.com", "file:///etc/passwd"] {
            assert!(url.parse_value(bad).is_err(), "accepted {bad}");
        }
        let integer = spec(SettingKind::Integer, None);
        assert_eq!(integer.parse_value("42"), Ok(toml::Value::Integer(42)));
        assert!(integer.parse_value("4.2").is_err());
        let boolean = spec(SettingKind::Boolean, None);
        assert_eq!(boolean.parse_value("true"), Ok(toml::Value::Boolean(true)));
        assert!(boolean.parse_value("yes").is_err());
        let choice = spec(SettingKind::Choice, Some(vec!["a".into(), "b".into()]));
        assert!(choice.parse_value("b").is_ok());
        assert!(choice.parse_value("c").is_err());
    }

    #[test]
    fn tasks_reject_duplicates_and_zero_interval() {
        let base = |body: &str| {
            format!(
                "manifest_version = 1\nplugin_id = \"p\"\ndisplay_name = \"P\"\n\
                 version = \"0.1.0\"\nnoflex_api_version = \"0.2\"\n\
                 capabilities = [\"background\"]\n{body}"
            )
        };
        let dup = PluginManifest::from_bytes(
            base(
                "[[tasks]]\nname = \"a\"\ninterval_ms = 1\n\
                  [[tasks]]\nname = \"a\"\ninterval_ms = 2",
            )
            .as_bytes(),
        )
        .unwrap();
        assert!(dup.check_tasks(Path::new("p.wasm")).is_err());
        let zero =
            PluginManifest::from_bytes(base("[[tasks]]\nname = \"a\"\ninterval_ms = 0").as_bytes())
                .unwrap();
        assert!(zero.check_tasks(Path::new("p.wasm")).is_err());
    }
}
