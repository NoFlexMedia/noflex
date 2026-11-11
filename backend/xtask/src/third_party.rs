// SPDX-License-Identifier: AGPL-3.0-only
//
// The third-party code the apps ship that their package managers don't track, as
// `packaging/third-party/apps.toml` records it. `check` holds that record to the
// app, `sync` turns it into the license screen's assets, and `source` gathers the
// pinned sources a release publishes beside its builds, as the LGPL asks.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::download::{fetch, read_toml, sha256};

const RECORD: &str = "packaging/third-party/apps.toml";
const NOTICES: &str = "packaging/third-party/notices";
const ASSETS: &str = "flutter/assets/licenses";
const MANIFEST: &str = "manifest.json";
const PLATFORMS: &[&str] = &["android", "ios", "macos", "windows", "linux"];
/// Where App Review decides, and a GPL library would keep the app out.
const APP_STORE: &[&str] = &["ios", "macos"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    media_kit: String,
    text: BTreeMap<String, Pinned>,
    library: BTreeMap<String, Library>,
    build: BTreeMap<String, Build>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pinned {
    url: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    name: String,
    license: String,
    #[serde(default)]
    files: Vec<String>,
    notice: Option<String>,
    #[serde(default)]
    texts: Vec<String>,
    #[serde(default)]
    license_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Build {
    platforms: Vec<String>,
    source: Option<Pinned>,
    #[serde(default)]
    library: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    version: Option<String>,
    url: Option<String>,
    sha256: Option<String>,
}

impl Entry {
    fn pinned(&self) -> Option<(&str, &str)> {
        Some((self.url.as_deref()?, self.sha256.as_deref()?))
    }
}

/// What the app reads: each platform's libraries, and the files of their texts.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Manifest {
    generated: String,
    platforms: BTreeMap<String, Vec<Shown>>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Shown {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    files: Vec<String>,
}

impl Record {
    fn load(root: &Path) -> Result<Self> {
        read_toml(&root.join(RECORD))
    }

    /// Every mistake at once, so fixing the record isn't one run per line.
    fn validate(&self, root: &Path) -> Result<()> {
        let mut problems = Vec::new();
        let mut used = BTreeSet::new();
        for (build_id, build) in &self.build {
            for platform in &build.platforms {
                if !PLATFORMS.contains(&platform.as_str()) {
                    problems.push(format!("build.{build_id}: unknown platform `{platform}`"));
                }
            }
            let reviewed = build
                .platforms
                .iter()
                .any(|p| APP_STORE.contains(&p.as_str()));
            for entry in &build.library {
                let at = format!("build.{build_id}, {}", entry.id);
                used.insert(entry.id.as_str());
                let Some(library) = self.library.get(&entry.id) else {
                    problems.push(format!("{at}: no [library.{}]", entry.id));
                    continue;
                };
                match (&entry.url, &entry.sha256) {
                    (Some(_), None) => problems.push(format!(
                        "{at}: a url with no sha256; `cargo xtask third-party source` prints it"
                    )),
                    (None, Some(_)) => problems.push(format!("{at}: a sha256 with no url")),
                    _ => {}
                }
                if reviewed && is_gpl(&library.license) {
                    problems.push(format!(
                        "{at}: {} can't ship on the App Store",
                        library.license
                    ));
                }
            }
        }
        for (id, library) in &self.library {
            if !used.contains(id.as_str()) {
                problems.push(format!("library.{id}: no build ships it"));
            }
            let sources = [
                library.notice.is_some(),
                !library.files.is_empty(),
                !library.license_urls.is_empty(),
            ];
            if sources.iter().filter(|&&s| s).count() != 1 {
                problems.push(format!(
                    "library.{id}: needs exactly one of notice, files and license_urls"
                ));
            }
            if let Some(notice) = &library.notice
                && !root.join(NOTICES).join(notice).is_file()
            {
                problems.push(format!("library.{id}: no {NOTICES}/{notice}"));
            }
            if !library.files.is_empty() && self.first_pinned(id).is_none() {
                problems.push(format!(
                    "library.{id}: `files` needs a build that pins its source"
                ));
            }
            for text in &library.texts {
                if !self.text.contains_key(text) {
                    problems.push(format!("library.{id}: no [text.\"{text}\"]"));
                }
            }
        }
        match media_kit_pin(root) {
            Ok(pin) if pin == self.media_kit => {}
            Ok(pin) => problems.push(format!(
                "flutter/pubspec.lock pins media_kit {pin}, but {RECORD} describes {}; \
                 read its builds again (packaging/AGENTS.md)",
                self.media_kit
            )),
            Err(error) => problems.push(format!("{error:#}")),
        }
        if !problems.is_empty() {
            bail!("{RECORD}:\n  {}", problems.join("\n  "));
        }
        Ok(())
    }

    /// The source a library's `files` come from: the first build that pins one.
    fn first_pinned(&self, id: &str) -> Option<(&str, &str)> {
        self.build
            .values()
            .flat_map(|build| &build.library)
            .filter(|entry| entry.id == id)
            .find_map(Entry::pinned)
    }

    fn manifest(&self) -> Manifest {
        let platforms = PLATFORMS
            .iter()
            .map(|&platform| {
                let shown = self
                    .build
                    .values()
                    .filter(|build| build.platforms.iter().any(|p| p == platform))
                    .flat_map(|build| &build.library)
                    .map(|entry| {
                        let library = &self.library[&entry.id];
                        Shown {
                            name: library.name.clone(),
                            version: entry.version.clone(),
                            files: std::iter::once(&entry.id)
                                .chain(&library.texts)
                                .map(|name| format!("{name}.txt"))
                                .collect(),
                        }
                    })
                    .collect();
                (platform.to_string(), shown)
            })
            .collect();
        Manifest {
            generated: format!("by `cargo xtask third-party sync` from {RECORD}"),
            platforms,
        }
    }
}

/// Whether an SPDX expression names a GPL or AGPL license, rather than the LGPL.
fn is_gpl(expression: &str) -> bool {
    expression
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .any(|token| token.starts_with("GPL-") || token.starts_with("AGPL-"))
}

/// The media_kit commit `flutter/pubspec.lock` resolves.
fn media_kit_pin(root: &Path) -> Result<String> {
    let path = root.join("flutter/pubspec.lock");
    let lock =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    lock.lines()
        .skip_while(|line| *line != "  media_kit:")
        .skip(1)
        .take_while(|line| line.starts_with("    "))
        .find_map(|line| line.trim().strip_prefix("resolved-ref: "))
        .map(|pin| pin.trim_matches('"').to_string())
        .context("flutter/pubspec.lock resolves no media_kit commit")
}

/// The record matches the app, and the assets match the record. Needs no network,
/// so CI runs it, as a test.
pub fn check(root: &Path) -> Result<()> {
    let record = Record::load(root)?;
    record.validate(root)?;
    let assets = root.join(ASSETS);
    let path = assets.join(MANIFEST);
    let committed: Manifest = serde_json::from_str(
        &std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))?;
    if committed != record.manifest() {
        bail!(
            "{ASSETS}/{MANIFEST} is out of date with {RECORD}; run `cargo xtask third-party sync`"
        );
    }
    let named: BTreeSet<String> = committed
        .platforms
        .values()
        .flatten()
        .flat_map(|shown| shown.files.iter().cloned())
        .collect();
    let present = texts_in(&assets)?;
    let missing: Vec<_> = named.difference(&present).cloned().collect();
    let stale: Vec<_> = present.difference(&named).cloned().collect();
    if !missing.is_empty() || !stale.is_empty() {
        bail!(
            "{ASSETS} differs from its manifest (missing: {missing:?}, unused: {stale:?}); \
             run `cargo xtask third-party sync`"
        );
    }
    Ok(())
}

/// Writes every text the manifest names into the app's assets, and the manifest.
/// Pinned files are checked against their hashes; `license_urls` can't be.
pub fn sync(root: &Path) -> Result<()> {
    let record = Record::load(root)?;
    record.validate(root)?;
    let cache = root.join(".cache/third-party");
    let assets = root.join(ASSETS);
    std::fs::create_dir_all(&assets)?;
    let mut written = BTreeSet::new();
    for (id, pinned) in &record.text {
        let file = cache.join("texts").join(id);
        fetch(&pinned.url, &pinned.sha256, &file)?;
        let name = format!("{id}.txt");
        std::fs::copy(&file, assets.join(&name))?;
        written.insert(name);
    }
    for (id, library) in &record.library {
        let text = if let Some(notice) = &library.notice {
            let path = root.join(NOTICES).join(notice);
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?
        } else if !library.files.is_empty() {
            let (url, sha) = record.first_pinned(id).expect("validated");
            let archive = fetch_source(&cache, url, sha)?;
            library
                .files
                .iter()
                .map(|file| extract(&archive, file))
                .collect::<Result<Vec<_>>>()?
                .join("\n")
        } else {
            library
                .license_urls
                .iter()
                .map(|url| download_text(url))
                .collect::<Result<Vec<_>>>()?
                .join("\n")
        };
        let name = format!("{id}.txt");
        std::fs::write(assets.join(&name), normalize(&text))?;
        written.insert(name);
    }
    for stale in texts_in(&assets)?.difference(&written) {
        std::fs::remove_file(assets.join(stale))?;
    }
    let manifest = serde_json::to_string_pretty(&record.manifest())? + "\n";
    std::fs::write(assets.join(MANIFEST), manifest)?;
    println!("==> wrote {ASSETS}");
    check(root)
}

/// Gathers each build's scripts and every pinned library source into `out`, with
/// a README naming what each file is and what couldn't be pinned.
pub fn source(root: &Path, out: &Path) -> Result<()> {
    let record = Record::load(root)?;
    let cache = root.join(".cache/third-party");
    // A missing hash is what this is for when the record is being written, so
    // it's reported here rather than refused by `validate` first.
    let mut unhashed = Vec::new();
    for (build_id, build) in &record.build {
        for entry in &build.library {
            if let (Some(url), None) = (&entry.url, &entry.sha256) {
                let file = cache.join("unhashed").join(file_name(url));
                fetch_unverified(url, &file)?;
                unhashed.push(format!(
                    "build.{build_id}, {}: sha256 = \"{}\"",
                    entry.id,
                    sha256(&file)?
                ));
            }
        }
    }
    if !unhashed.is_empty() {
        bail!(
            "add these to {RECORD}, once you've checked the files are what the build used:\n  {}",
            unhashed.join("\n  ")
        );
    }
    record.validate(root)?;
    std::fs::create_dir_all(out)?;
    let mut readme = format!(
        "The source of the third-party libraries NoFlex's apps ship, as {RECORD} records\n\
         them for this release. Each build's own scripts and patches are build-<name>.*.\n"
    );
    for (build_id, build) in &record.build {
        readme += &format!("\n{build_id} ({}):\n", build.platforms.join(", "));
        if let Some(pinned) = &build.source {
            let name = format!("build-{build_id}.{}", extension(&pinned.url));
            copy_pinned(&cache, &pinned.url, &pinned.sha256, &out.join(&name))?;
            readme += &format!("  {name}  its build scripts, from {}\n", pinned.url);
        }
        for entry in &build.library {
            let library = &record.library[&entry.id];
            match (entry.pinned(), &entry.version) {
                (Some((url, sha)), Some(version)) => {
                    let name = format!("{}-{version}.{}", entry.id, extension(url));
                    copy_pinned(&cache, url, sha, &out.join(&name))?;
                    readme += &format!("  {name}  {}, from {url}\n", library.name);
                }
                _ => {
                    readme += &format!(
                        "  {} has no pinned source; {RECORD} says why\n",
                        library.name
                    );
                }
            }
        }
    }
    std::fs::write(out.join("README.txt"), readme)?;
    println!("==> gathered sources into {}", out.display());
    Ok(())
}

fn copy_pinned(cache: &Path, url: &str, sha: &str, dest: &Path) -> Result<()> {
    let archive = fetch_source(cache, url, sha)?;
    if !dest.is_file() {
        std::fs::copy(&archive, dest).with_context(|| format!("copy to {}", dest.display()))?;
    }
    Ok(())
}

fn fetch_source(cache: &Path, url: &str, sha: &str) -> Result<PathBuf> {
    let file = cache
        .join("sources")
        .join(format!("{}-{}", &sha[..12], file_name(url)));
    fetch(url, sha, &file)?;
    Ok(file)
}

fn fetch_unverified(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(dest)
        .arg(url)
        .status()
        .context("run curl")?;
    if !status.success() {
        bail!("download failed: {url}");
    }
    Ok(())
}

fn download_text(url: &str) -> Result<String> {
    println!("==> fetching {url}");
    let output = Command::new("curl")
        .args(["--fail", "--location", "--silent", "--show-error", url])
        .output()
        .context("run curl")?;
    if !output.status.success() {
        bail!(
            "download failed: {url}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).with_context(|| format!("{url} isn't UTF-8"))
}

/// One file from a source archive, wherever its top directory is named.
fn extract(archive: &Path, path: &str) -> Result<String> {
    let listing = Command::new("tar")
        .arg("-tf")
        .arg(archive)
        .output()
        .context("run tar")?;
    if !listing.status.success() {
        bail!("could not list {}", archive.display());
    }
    let listing = String::from_utf8_lossy(&listing.stdout);
    let member = listing
        .lines()
        .find(|member| member.split_once('/').is_some_and(|(_, rest)| rest == path))
        .with_context(|| format!("{} has no {path}", archive.display()))?;
    let output = Command::new("tar")
        .arg("-xOf")
        .arg(archive)
        .arg(member)
        .output()
        .context("run tar")?;
    if !output.status.success() {
        bail!("could not extract {member} from {}", archive.display());
    }
    String::from_utf8(output.stdout).with_context(|| format!("{member} isn't UTF-8"))
}

/// Unix line endings and one trailing newline, whatever upstream used.
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end().to_string() + "\n"
}

fn texts_in(dir: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.ends_with(".txt") {
            names.insert(name);
        }
    }
    Ok(names)
}

fn file_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

fn extension(url: &str) -> &str {
    let name = file_name(url);
    [".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".zip"]
        .iter()
        .find(|ext| name.ends_with(*ext))
        .map_or("tar.gz", |ext| &ext[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_record_matches_the_app() {
        check(&root()).unwrap();
    }

    #[test]
    fn the_lgpl_isnt_the_gpl() {
        assert!(!is_gpl("LGPL-2.1-or-later"));
        assert!(!is_gpl("MIT OR Apache-2.0"));
        assert!(is_gpl("GPL-2.0-or-later"));
        assert!(is_gpl("(MIT AND GPL-3.0-only)"));
    }
}
