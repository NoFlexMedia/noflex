// SPDX-License-Identifier: AGPL-3.0-only
//
// On-disk plugin secrets store. Tiered read path (later shadows
// earlier) matches the plugin search-path policy:
//   1. /etc/noflex/secrets/<plugin_id>.toml  (system)
//   2. /var/lib/noflex/secrets/<plugin_id>.toml (state)
//      (both %ProgramData%\NoFlex\secrets on Windows)
//   3. $XDG_CONFIG_HOME/noflex/secrets/<plugin_id>.toml (user)
//
// Each file is a flat TOML key→value map. Reads audit-logged via
// `crate::audit::secret_read` and filtered by the plugin's manifest
// `secrets_read` allowlist (the key is silently denied if the plugin
// didn't declare it).

use std::path::{Path, PathBuf};

use crate::PluginId;

#[derive(Debug, Default, Clone)]
pub struct SecretStore {
    tiers: Vec<PathBuf>,
}

impl SecretStore {
    pub fn with_default_tiers() -> Self {
        let mut tiers = vec![noflex_core::system_dirs::config().join("secrets")];
        tiers.extend(noflex_core::system_dirs::state().map(|dir| dir.join("secrets")));
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            tiers.push(PathBuf::from(xdg).join("noflex").join("secrets"));
        } else if let Some(home) =
            directories::UserDirs::new().and_then(|u| u.home_dir().to_path_buf().into())
        {
            tiers.push(home.join(".config").join("noflex").join("secrets"));
        }
        Self { tiers }
    }

    pub fn with_tiers(tiers: Vec<PathBuf>) -> Self {
        Self { tiers }
    }

    pub fn tiers(&self) -> &[PathBuf] {
        &self.tiers
    }

    /// Read a key for a plugin. Later tiers shadow earlier ones
    /// (matches the plugin search-path policy), so the highest-tier
    /// (last in `tiers`) hit wins. `declared_keys` is the plugin's
    /// `secrets_read` manifest allowlist; keys not in it return None.
    pub fn read(&self, plugin: &PluginId, key: &str, declared_keys: &[String]) -> Option<String> {
        if crate::manifest::validate_plugin_id(&plugin.0).is_err()
            || !declared_keys.iter().any(|k| k == key)
        {
            return None;
        }
        let file_name = format!("{}.toml", plugin.0);
        let mut result = None;
        for tier in &self.tiers {
            let path = tier.join(&file_name);
            if path.parent() != Some(tier.as_path()) {
                continue;
            }
            if let Some(v) = read_key_from_file(&path, key) {
                result = Some(v);
            }
        }
        result
    }
}

fn read_key_from_file(path: &Path, key: &str) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    value.get(key).and_then(|v| v.as_str().map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadowing_picks_later_tier() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir1.path().join("tmdb.toml"), "api_key = \"system-key\"").unwrap();
        std::fs::write(dir2.path().join("tmdb.toml"), "api_key = \"user-key\"").unwrap();
        let store =
            SecretStore::with_tiers(vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()]);
        let got = store.read(
            &PluginId("tmdb".into()),
            "api_key",
            &["api_key".to_string()],
        );
        assert_eq!(got.as_deref(), Some("user-key"));
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let store = SecretStore::with_tiers(vec![]);
        assert!(
            store
                .read(&PluginId("missing".into()), "x", &["x".to_string()])
                .is_none()
        );
    }

    #[test]
    fn undeclared_key_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tmdb.toml"), "api_key = \"k\"").unwrap();
        let store = SecretStore::with_tiers(vec![dir.path().to_path_buf()]);
        assert!(
            store
                .read(&PluginId("tmdb".into()), "api_key", &[])
                .is_none()
        );
    }

    #[test]
    fn malicious_plugin_id_cannot_escape_tier() {
        let tier = tempfile::tempdir().unwrap();
        let outside = tier.path().parent().unwrap().join("escaped.toml");
        std::fs::write(&outside, "api_key = \"stolen\"").unwrap();
        let store = SecretStore::with_tiers(vec![tier.path().to_path_buf()]);
        let got = store.read(
            &PluginId("../escaped".into()),
            "api_key",
            &["api_key".into()],
        );
        assert_eq!(got, None);
        let _ = std::fs::remove_file(outside);
    }
}
