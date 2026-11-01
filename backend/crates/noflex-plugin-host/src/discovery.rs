// SPDX-License-Identifier: AGPL-3.0-only

use std::path::{Path, PathBuf};

use directories::BaseDirs;

/// The tiered plugin directory search path.
///
/// Resolved at construction time. Each tier is either an absolute path
/// (system + state) or an `$XDG_CONFIG_HOME`-derived path (user). When two
/// plugins share a `plugin_id`, the **later** tier wins so a user-installed
/// build shadows the system one (intentional, useful for testing).
#[derive(Debug, Clone)]
pub struct SearchPath {
    /// Tiers in scan order. Earlier tiers are loaded first; later tiers
    /// override earlier on `plugin_id` collision.
    pub tiers: Vec<PluginDir>,
}

#[derive(Debug, Clone)]
pub struct PluginDir {
    pub tier: Tier,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    System,
    State,
    User,
    /// Caller-provided extra directory.
    Custom,
}

impl SearchPath {
    /// Build the standard three-tier search path:
    /// `/etc/noflex/plugins`, `/var/lib/noflex/plugins`, `$XDG_CONFIG_HOME/noflex/plugins`.
    ///
    /// Missing directories are still recorded — the watcher in
    /// [`crate::watcher`] will pick them up if they appear later, and the
    /// initial scan tolerates absent paths.
    pub fn default_tiers() -> Self {
        let mut tiers = vec![
            PluginDir {
                tier: Tier::System,
                path: PathBuf::from("/etc/noflex/plugins"),
            },
            PluginDir {
                tier: Tier::State,
                path: PathBuf::from("/var/lib/noflex/plugins"),
            },
        ];
        if let Some(base) = BaseDirs::new() {
            tiers.push(PluginDir {
                tier: Tier::User,
                path: base.config_dir().join("noflex").join("plugins"),
            });
        }
        Self { tiers }
    }

    /// Build a search path from explicit directories. Useful for tests and
    /// for non-standard deployments (`[plugins] search_paths = [...]` in
    /// the server config).
    pub fn from_explicit<I, P>(paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let tiers = paths
            .into_iter()
            .map(|p| PluginDir {
                tier: Tier::Custom,
                path: p.into(),
            })
            .collect();
        Self { tiers }
    }

    /// Append a custom directory. Lower priority than existing tiers if
    /// added last; useful for `noflex plugin dev --extra-dir`.
    pub fn push(&mut self, path: impl Into<PathBuf>) {
        self.tiers.push(PluginDir {
            tier: Tier::Custom,
            path: path.into(),
        });
    }

    /// Enumerate `.wasm` files across all configured tiers, in scan order.
    /// Missing or non-directory paths are silently skipped (initial-scan
    /// tolerance).
    pub fn discover_wasm_files(&self) -> Vec<(Tier, PathBuf)> {
        let mut out = Vec::new();
        for dir in &self.tiers {
            let Ok(entries) = std::fs::read_dir(&dir.path) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if is_wasm_file(&p) {
                    out.push((dir.tier, p));
                }
            }
        }
        out
    }

    /// Iterator over the directory paths configured for watching by [`crate::watcher`].
    pub fn directories(&self) -> impl Iterator<Item = &Path> {
        self.tiers.iter().map(|d| d.path.as_path())
    }
}

fn is_wasm_file(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wasm"))
        && p.is_file()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn default_tiers_includes_system_and_state() {
        let sp = SearchPath::default_tiers();
        let paths: Vec<_> = sp.tiers.iter().map(|d| d.path.clone()).collect();
        assert!(paths.iter().any(|p| p.ends_with("/etc/noflex/plugins")));
        assert!(paths.iter().any(|p| p.ends_with("/var/lib/noflex/plugins")));
    }

    #[test]
    fn discovers_wasm_files_in_explicit_dir() {
        let tmp = tempdir().unwrap();
        let wasm_path = tmp.path().join("test.wasm");
        let ignored = tmp.path().join("readme.txt");
        fs::write(&wasm_path, b"\0asm\x01\0\0\0").unwrap();
        fs::write(&ignored, b"hello").unwrap();

        let sp = SearchPath::from_explicit([tmp.path()]);
        let found = sp.discover_wasm_files();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, wasm_path);
    }

    #[test]
    fn discovery_tolerates_missing_dirs() {
        let sp = SearchPath::from_explicit(["/nonexistent/path/that/should/not/exist"]);
        assert!(sp.discover_wasm_files().is_empty());
    }
}
