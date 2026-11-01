// SPDX-License-Identifier: AGPL-3.0-only
//
// Recursive walker that yields files supported by a library kind. Hidden
// dot-paths and non-whitelisted extensions are skipped.

use std::{
    fs::Metadata,
    path::{Component, Path, PathBuf},
};

use noflex_storage::domain::LibraryKind;
use walkdir::WalkDir;

/// Lower-case extensions, compared case-insensitively.
pub const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "webm", "ts", "wmv", "flv", "ogv", "asf", "mxf", "mpeg",
    "vob", "3gp",
];
pub const AUDIO_EXTS: &[&str] = &["flac", "mp3", "m4a", "ogg", "opus", "wav"];
pub const PHOTO_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "heic", "heif", "tif", "tiff"];

#[derive(Debug)]
pub struct MediaFileCandidate {
    pub path: PathBuf,
    pub meta: Metadata,
}

/// Walk `root` recursively, returning every supported file for `kind`.
///
/// This is intentionally synchronous; callers wrap it in
/// [`tokio::task::spawn_blocking`] when they need an async boundary.
pub fn walk_media(
    root: &Path,
    kind: LibraryKind,
    excluded_folders: &[PathBuf],
) -> Vec<MediaFileCandidate> {
    let mut out = Vec::new();
    let iter = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !is_hidden(entry.file_name().to_str().unwrap_or(""))
                && !entry.path().strip_prefix(root).is_ok_and(|relative| {
                    excluded_folders
                        .iter()
                        .any(|path| relative.starts_with(path))
                })
        });
    for entry in iter.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !has_media_extension(path, kind) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!(?path, %err, "skip: failed to stat");
                continue;
            }
        };
        out.push(MediaFileCandidate {
            path: path.to_path_buf(),
            meta,
        });
    }
    out
}

/// Normalizes a configured exclusion into a relative path. Absolute paths and
/// parent traversal are rejected rather than interpreted against the process
/// working directory.
pub fn normalize_excluded_folder(path: &str) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "excluded folder must stay under the library root: {path}"
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("excluded folder must not be empty".to_string());
    }
    Ok(normalized)
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn has_media_extension(path: &Path, kind: LibraryKind) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    let lower = ext.to_ascii_lowercase();
    let extensions = match kind {
        LibraryKind::Music => AUDIO_EXTS,
        LibraryKind::Photos => PHOTO_EXTS,
        _ => VIDEO_EXTS,
    };
    extensions.iter().any(|e| *e == lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_sets_are_case_insensitive_and_disjoint() {
        for extension in AUDIO_EXTS {
            assert!(has_media_extension(
                Path::new(&format!("track.{extension}")),
                LibraryKind::Music
            ));
            assert!(!has_media_extension(
                Path::new(&format!("track.{extension}")),
                LibraryKind::Photos
            ));
        }
        for extension in PHOTO_EXTS {
            assert!(has_media_extension(
                Path::new(&format!("photo.{extension}")),
                LibraryKind::Photos
            ));
            assert!(!has_media_extension(
                Path::new(&format!("photo.{extension}")),
                LibraryKind::Music
            ));
        }
        assert!(has_media_extension(
            Path::new("track.FLAC"),
            LibraryKind::Music
        ));
        assert!(has_media_extension(
            Path::new("photo.JpEg"),
            LibraryKind::Photos
        ));
        assert!(!has_media_extension(
            Path::new("notes.txt"),
            LibraryKind::Photos
        ));
    }

    #[test]
    fn exclusions_are_normalized_without_escaping_the_root() {
        assert_eq!(
            normalize_excluded_folder("incoming/./failed").unwrap(),
            PathBuf::from("incoming/failed")
        );
        assert!(normalize_excluded_folder("../outside").is_err());
        assert!(normalize_excluded_folder("/outside").is_err());
        assert!(normalize_excluded_folder(".").is_err());
    }

    #[test]
    fn excluded_folder_matches_path_components_not_prefixes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("nested/cache")).unwrap();
        std::fs::create_dir_all(root.path().join("nested/cache-extra")).unwrap();
        std::fs::write(root.path().join("nested/cache/hidden.mkv"), b"x").unwrap();
        std::fs::write(root.path().join("nested/cache-extra/visible.mkv"), b"x").unwrap();

        let files = walk_media(
            root.path(),
            LibraryKind::Movies,
            &[PathBuf::from("nested/cache")],
        );
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("nested/cache-extra/visible.mkv"));
    }
}
