// SPDX-License-Identifier: AGPL-3.0-only
//
// Plugin settings as the host resolves them: an administrator's value
// (storage) wins over the server's config.toml stanza or the secret files,
// which win over the manifest's default.

use std::collections::{BTreeMap, HashMap};

use async_graphql::{Enum, SimpleObject};
use noflex_storage::{domain::PluginSettingWrite, error::StorageError};
use thiserror::Error;

use crate::{
    HostError, PluginId, PluginManifest, SecretStore,
    manifest::{SettingKind, setting_string},
};

/// The server's `[plugin.per_plugin_config.<id>]` stanzas, by plugin id.
pub type ConfigStanzas = HashMap<String, toml::Table>;

/// Where a setting's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
#[graphql(name = "PluginSettingSource")]
pub enum SettingSource {
    /// An administrator set it from the admin UI.
    Admin,
    /// The server's config.toml, or for a secret, a secret file.
    Config,
    /// The manifest's default.
    Default,
    Unset,
}

/// One setting a plugin's manifest declares, with its value now.
#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "PluginSetting")]
pub struct PluginSetting {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: SettingKind,
    pub required: bool,
    /// The values a choice allows; empty for any other kind.
    pub choices: Vec<String>,
    pub default: Option<String>,
    /// The value in effect, as a string (`true`, `42`). Always null for a
    /// secret, whose value never leaves the server.
    pub value: Option<String>,
    pub is_set: bool,
    pub source: SettingSource,
}

/// Why [`PluginRegistry::update_settings`](crate::PluginRegistry::update_settings)
/// refused or failed.
#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("The plugin isn't loaded")]
    NotLoaded,
    #[error("The plugin has no setting “{0}”")]
    UnknownKey(String),
    /// The message leads with the setting's label, for showing as is.
    #[error("{label} {reason}")]
    Invalid {
        key: String,
        label: String,
        reason: String,
    },
    #[error("{label} is required")]
    Required { key: String, label: String },
    #[error("plugin settings need storage")]
    NoStorage,
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Reload(#[from] HostError),
}

impl SettingsError {
    /// Whether the caller sent something the schema refuses, as opposed to
    /// the host failing.
    pub fn is_bad_input(&self) -> bool {
        matches!(
            self,
            Self::NotLoaded | Self::UnknownKey(_) | Self::Invalid { .. } | Self::Required { .. }
        )
    }
}

/// The TOML document `host-config.read` hands `manifest`'s plugin: its
/// config.toml stanza with each non-secret setting replaced by the value in
/// `admin`, or filled from its default when neither sets it.
pub(crate) fn render(
    manifest: &PluginManifest,
    base: Option<&toml::Table>,
    admin: &BTreeMap<String, String>,
) -> String {
    let mut table = base.cloned().unwrap_or_default();
    for spec in manifest
        .settings
        .iter()
        .filter(|s| s.kind != SettingKind::Secret)
    {
        if let Some(raw) = admin.get(&spec.key) {
            match spec.parse_value(raw) {
                Ok(value) => {
                    table.insert(spec.key.clone(), value);
                    continue;
                }
                // A plugin update can change a setting's kind under a value
                // stored for the old one.
                Err(reason) => tracing::warn!(
                    plugin = %manifest.plugin_id,
                    key = %spec.key,
                    %reason,
                    "ignoring a stored setting that no longer fits its kind",
                ),
            }
        }
        if !table.contains_key(&spec.key)
            && let Some(default) = &spec.default
        {
            table.insert(spec.key.clone(), default.clone());
        }
    }
    toml::to_string(&table).unwrap_or_else(|e| {
        tracing::warn!(plugin = %manifest.plugin_id, error = %e, "plugin config doesn't render");
        String::new()
    })
}

/// Every setting `manifest` declares, resolved the way [`render`] and
/// `host-secrets.read` resolve them.
pub(crate) fn view(
    manifest: &PluginManifest,
    base: Option<&toml::Table>,
    admin: &BTreeMap<String, String>,
    admin_secrets: &[String],
    files: &SecretStore,
) -> Vec<PluginSetting> {
    let id = PluginId(manifest.plugin_id.clone());
    manifest
        .settings
        .iter()
        .map(|spec| {
            let (value, source) = if spec.kind == SettingKind::Secret {
                let source = if admin_secrets.contains(&spec.key) {
                    SettingSource::Admin
                } else if files.read(&id, &spec.key, &manifest.secrets_read).is_some() {
                    SettingSource::Config
                } else {
                    SettingSource::Unset
                };
                (None, source)
            } else if let Some(raw) = admin
                .get(&spec.key)
                .filter(|raw| spec.parse_value(raw).is_ok())
            {
                (Some(raw.clone()), SettingSource::Admin)
            } else if let Some(value) = base.and_then(|b| b.get(&spec.key)) {
                (Some(setting_string(value)), SettingSource::Config)
            } else if let Some(default) = spec.default_string() {
                (Some(default), SettingSource::Default)
            } else {
                (None, SettingSource::Unset)
            };
            PluginSetting {
                key: spec.key.clone(),
                label: spec.label.clone(),
                description: spec.description.clone(),
                kind: spec.kind,
                required: spec.required,
                choices: spec.choices.clone().unwrap_or_default(),
                default: spec.default_string(),
                value,
                is_set: source != SettingSource::Unset,
                source,
            }
        })
        .collect()
}

/// Check `changes` against `manifest`'s schema and turn them into storage
/// writes. A `None` or empty value clears the key, which a required setting
/// allows only while config.toml, a secret file or its default still
/// supplies one.
pub(crate) fn plan(
    manifest: &PluginManifest,
    base: Option<&toml::Table>,
    files: &SecretStore,
    changes: Vec<(String, Option<String>)>,
) -> Result<Vec<PluginSettingWrite>, SettingsError> {
    let id = PluginId(manifest.plugin_id.clone());
    let mut writes: Vec<PluginSettingWrite> = Vec::with_capacity(changes.len());
    for (key, value) in changes {
        let Some(spec) = manifest.setting(&key) else {
            return Err(SettingsError::UnknownKey(key));
        };
        let invalid = |reason: &str| SettingsError::Invalid {
            key: key.clone(),
            label: spec.label.clone(),
            reason: reason.to_string(),
        };
        if writes.iter().any(|w| w.key == key) {
            return Err(invalid("is listed twice"));
        }
        let secret = spec.kind == SettingKind::Secret;
        let value = match value.filter(|v| !v.is_empty()) {
            Some(raw) if raw.len() > crate::dispatch::MAX_SECRET_VALUE_BYTES => {
                return Err(invalid("is too long"));
            }
            Some(raw) => Some(setting_string(
                &spec.parse_value(&raw).map_err(|reason| invalid(&reason))?,
            )),
            None => {
                let supplied = if secret {
                    files.read(&id, &key, &manifest.secrets_read).is_some()
                } else {
                    base.is_some_and(|b| b.contains_key(&key)) || spec.default.is_some()
                };
                if spec.required && !supplied {
                    return Err(SettingsError::Required {
                        key,
                        label: spec.label.clone(),
                    });
                }
                None
            }
        };
        writes.push(PluginSettingWrite { key, value, secret });
    }
    Ok(writes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> PluginManifest {
        PluginManifest::from_bytes(
            br#"
                manifest_version = 1
                plugin_id = "webhook"
                display_name = "Webhook"
                version = "0.1.0"
                noflex_api_version = "0.1"
                capabilities = ["secrets.read"]
                secrets_read = ["auth_token"]

                [[settings]]
                key = "url"
                label = "URL"
                kind = "url"
                required = true

                [[settings]]
                key = "retries"
                label = "Retries"
                kind = "integer"
                default = 3

                [[settings]]
                key = "auth_token"
                label = "Token"
                kind = "secret"
            "#,
        )
        .unwrap()
    }

    fn table(raw: &str) -> toml::Table {
        toml::from_str(raw).unwrap()
    }

    #[test]
    fn admin_values_win_over_config_and_defaults_fill_the_rest() {
        let base = table("url = \"https://config.example\"\nunrelated = 1\n");
        let admin = BTreeMap::from([("url".to_string(), "https://admin.example".to_string())]);
        let rendered: toml::Table =
            toml::from_str(&render(&manifest(), Some(&base), &admin)).unwrap();
        assert_eq!(rendered["url"].as_str(), Some("https://admin.example"));
        assert_eq!(rendered["retries"].as_integer(), Some(3), "typed default");
        assert_eq!(rendered["unrelated"].as_integer(), Some(1), "stanza kept");
        assert!(!rendered.contains_key("auth_token"), "secrets stay out");

        let rendered: toml::Table =
            toml::from_str(&render(&manifest(), Some(&base), &BTreeMap::new())).unwrap();
        assert_eq!(rendered["url"].as_str(), Some("https://config.example"));
    }

    #[test]
    fn view_says_where_each_value_comes_from() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("webhook.toml"), "auth_token = \"t\"").unwrap();
        let files = SecretStore::with_tiers(vec![dir.path().to_path_buf()]);
        let base = table("url = \"https://config.example\"");
        let view = view(&manifest(), Some(&base), &BTreeMap::new(), &[], &files);
        let source = |key: &str| view.iter().find(|s| s.key == key).unwrap();
        assert_eq!(source("url").source, SettingSource::Config);
        assert_eq!(source("retries").source, SettingSource::Default);
        assert_eq!(source("retries").value.as_deref(), Some("3"));
        let token = source("auth_token");
        assert_eq!(
            (token.source, token.value.as_deref()),
            (SettingSource::Config, None)
        );

        let admin_set = ["auth_token".to_string()];
        let view = super::view(&manifest(), None, &BTreeMap::new(), &admin_set, &files);
        let token = view.iter().find(|s| s.key == "auth_token").unwrap();
        assert_eq!(
            (token.source, token.is_set, token.value.as_deref()),
            (SettingSource::Admin, true, None),
            "an admin secret shadows the file and is never shown"
        );
        let url = view.iter().find(|s| s.key == "url").unwrap();
        assert_eq!((url.source, url.is_set), (SettingSource::Unset, false));
    }

    #[test]
    fn plan_checks_values_and_required_fallbacks() {
        let files = SecretStore::with_tiers(vec![]);
        let plan = |base: Option<&toml::Table>, changes: &[(&str, Option<&str>)]| {
            plan(
                &manifest(),
                base,
                &files,
                changes
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
                    .collect(),
            )
        };
        let writes = plan(
            None,
            &[
                ("url", Some(" https://a.example ")),
                ("auth_token", Some("s")),
            ],
        )
        .expect("valid");
        assert_eq!(writes[0].value.as_deref(), Some("https://a.example"));
        assert!(writes[1].secret);

        assert!(matches!(
            plan(None, &[("nope", Some("x"))]),
            Err(SettingsError::UnknownKey(_))
        ));
        assert!(matches!(
            plan(None, &[("retries", Some("many"))]),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(matches!(
            plan(None, &[("url", None)]),
            Err(SettingsError::Required { .. })
        ));
        assert!(matches!(
            plan(None, &[("url", Some(""))]),
            Err(SettingsError::Required { .. })
        ));
        let base = table("url = \"https://config.example\"");
        assert!(
            plan(Some(&base), &[("url", None)]).is_ok(),
            "config.toml still sets it"
        );
        assert!(matches!(
            plan(None, &[("retries", Some("1")), ("retries", None)]),
            Err(SettingsError::Invalid { .. })
        ));
    }
}
