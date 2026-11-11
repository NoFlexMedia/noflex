// SPDX-License-Identifier: AGPL-3.0-only

//! Where a server keeps what belongs to the machine rather than to one account:
//! the system tiers of configuration, plugins and secrets. Unix splits them into
//! `/etc/noflex` and `/var/lib/noflex`; Windows has the one `%ProgramData%\NoFlex`.

use std::path::PathBuf;

/// The system tier an administrator configures: `config.toml`, `plugins/` and
/// `secrets/`. `/etc/noflex`, or `%ProgramData%\NoFlex` on Windows.
pub fn config() -> PathBuf {
    #[cfg(windows)]
    {
        program_data()
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/noflex")
    }
}

/// The tier the server's own installs write, for `plugins/` and `secrets/`:
/// `/var/lib/noflex`. `None` on Windows, where it's [`config`].
pub fn state() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        None
    }
    #[cfg(not(windows))]
    {
        Some(PathBuf::from("/var/lib/noflex"))
    }
}

/// `%ProgramData%\NoFlex`, where the installer puts the service's config and data.
#[cfg(windows)]
fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
        .join("NoFlex")
}
