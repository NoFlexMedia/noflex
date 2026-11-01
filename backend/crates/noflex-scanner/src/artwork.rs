// SPDX-License-Identifier: AGPL-3.0-only
//
// Plex-convention local artwork discovery (`poster.jpg`, `fanart.png`,
// `season01-poster.jpg`, `<episode>-thumb.jpg`, …).
//
// Discovery runs once per scan over every walked media file, changed or
// not: a poster dropped next to an untouched video must still be picked
// up. Each file contributes claims — "this directory may hold art for
// this work, in this role" — which are deduplicated so each directory is
// listed at most once per scan, however many episodes share it.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use noflex_core::ImageKind;
use regex::Regex;

/// Recognised artwork extensions, in tie-break order: when `poster.jpg`
/// and `poster.png` both exist, the earlier extension wins.
pub(crate) const ART_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Ranks below this are name-specific (video stem or season number);
/// generic names (`poster`, `fanart`, …) rank at or above it, so a
/// specific name always beats a generic one for the same work and kind.
const GENERIC: u8 = 10;

/// How a directory relates to the work whose art it may hold. Stems are
/// stored lowercase; matching is case-insensitive throughout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ArtRole {
    /// A movie in its own folder; `stem` is one of its video stems.
    MovieFolder {
        stem: String,
    },
    /// A movie file directly at a library root. Only stem-prefixed names
    /// count: a root-level `poster.jpg` belongs to no single movie.
    BareMovie {
        stem: String,
    },
    ShowFolder,
    /// Season-numbered names (`season01-poster.jpg`) in the show folder.
    ShowFolderSeason {
        number: u32,
    },
    SeasonFolder,
    Episode {
        stem: String,
    },
    AlbumFolder,
    ArtistFolder,
}

impl ArtRole {
    pub(crate) fn movie(video: &Path, in_folder: bool) -> Option<Self> {
        let stem = video.file_stem()?.to_str()?.to_lowercase();
        Some(if in_folder {
            Self::MovieFolder { stem }
        } else {
            Self::BareMovie { stem }
        })
    }

    pub(crate) fn episode(video: &Path) -> Option<Self> {
        let stem = video.file_stem()?.to_str()?.to_lowercase();
        Some(Self::Episode { stem })
    }
}

/// Map an artwork file name to the image kind it supplies in `role`, and
/// its rank (lower wins). `None` for names the role doesn't recognise,
/// including non-image extensions.
pub(crate) fn match_art(role: &ArtRole, file_name: &str) -> Option<(ImageKind, u8)> {
    let lower = file_name.to_lowercase();
    let (stem, ext) = lower.rsplit_once('.')?;
    if !ART_EXTS.contains(&ext) {
        return None;
    }
    match role {
        ArtRole::MovieFolder { stem: video } => stem_movie_art(stem, video).or_else(|| {
            generic(
                stem,
                &[
                    (ImageKind::Primary, &["poster", "folder", "cover"]),
                    (
                        ImageKind::Backdrop,
                        &["fanart", "background", "art", "backdrop"],
                    ),
                    (ImageKind::Banner, &["banner"]),
                    (ImageKind::Logo, &["logo", "clearlogo"]),
                ],
            )
        }),
        ArtRole::BareMovie { stem: video } => stem_movie_art(stem, video),
        ArtRole::ShowFolder => generic(
            stem,
            &[
                (ImageKind::Primary, &["poster", "folder", "show"]),
                (
                    ImageKind::Backdrop,
                    &["fanart", "background", "art", "backdrop"],
                ),
                (ImageKind::Banner, &["banner"]),
                (ImageKind::Logo, &["logo", "clearlogo"]),
            ],
        ),
        ArtRole::ShowFolderSeason { number } => season_numbered_art(stem, *number),
        ArtRole::SeasonFolder => generic(
            stem,
            &[(ImageKind::Primary, &["poster", "folder", "cover"])],
        ),
        ArtRole::Episode { stem: video } => {
            if stem == video {
                Some((ImageKind::Thumb, 0))
            } else if stem.strip_prefix(video.as_str()) == Some("-thumb") {
                Some((ImageKind::Thumb, 1))
            } else {
                None
            }
        }
        ArtRole::AlbumFolder => generic(
            stem,
            &[(
                ImageKind::Primary,
                &["cover", "folder", "front", "album", "poster"],
            )],
        ),
        ArtRole::ArtistFolder => generic(
            stem,
            &[
                (ImageKind::Primary, &["artist", "poster", "folder"]),
                (ImageKind::Backdrop, &["fanart", "backdrop"]),
            ],
        ),
    }
}

fn stem_movie_art(stem: &str, video: &str) -> Option<(ImageKind, u8)> {
    match stem.strip_prefix(video)? {
        "" => Some((ImageKind::Primary, 0)),
        "-poster" => Some((ImageKind::Primary, 1)),
        "-fanart" => Some((ImageKind::Backdrop, 0)),
        _ => None,
    }
}

fn generic(stem: &str, table: &[(ImageKind, &[&str])]) -> Option<(ImageKind, u8)> {
    table.iter().find_map(|(kind, names)| {
        let pos = names.iter().position(|n| *n == stem)?;
        Some((*kind, GENERIC + u8::try_from(pos).ok()?))
    })
}

/// `season01-poster`, `season1-poster`, `season01` (poster), `seasonNN-fanart`,
/// `seasonNN-banner`; `season-specials-*` is season 0.
fn season_numbered_art(stem: &str, number: u32) -> Option<(ImageKind, u8)> {
    static R: OnceLock<Option<Regex>> = OnceLock::new();
    let re = R
        .get_or_init(|| {
            Regex::new(r"^season(?:(\d{1,4})|-specials)(?:-(poster|fanart|banner))?$").ok()
        })
        .as_ref()?;
    let caps = re.captures(stem)?;
    let named = match caps.get(1) {
        Some(digits) => digits.as_str().parse::<u32>().ok()?,
        None => 0,
    };
    if named != number {
        return None;
    }
    match caps.get(2).map(|m| m.as_str()) {
        Some("poster") => Some((ImageKind::Primary, 0)),
        // Bare `Season01.jpg` is a poster, but `-poster` is more explicit.
        None => Some((ImageKind::Primary, 1)),
        Some("fanart") => Some((ImageKind::Backdrop, 0)),
        Some("banner") => Some((ImageKind::Banner, 0)),
        Some(_) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Claim {
    dir: PathBuf,
    role: ArtRole,
    work_key: String,
}

/// Deduplicated artwork claims gathered over one scan.
#[derive(Debug, Default)]
pub(crate) struct ArtClaims {
    claims: HashSet<Claim>,
}

impl ArtClaims {
    pub(crate) fn add(&mut self, dir: &Path, role: ArtRole, work_key: &str) {
        self.claims.insert(Claim {
            dir: dir.to_path_buf(),
            role,
            work_key: work_key.to_string(),
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// Every work key some claim refers to.
    pub(crate) fn work_keys(&self) -> HashSet<&str> {
        self.claims.iter().map(|c| c.work_key.as_str()).collect()
    }

    pub(crate) fn dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self
            .claims
            .iter()
            .map(|c| c.dir.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        dirs.sort();
        dirs
    }

    /// The winning artwork file per `(work key, kind)`, given each
    /// directory's image-file names. Ties on rank fall to [`ART_EXTS`]
    /// order, then to the lexically smallest path, so the choice never
    /// depends on directory iteration order.
    ///
    /// Several season folders can feed one Season (`Season 01 (1080p)`,
    /// `Season 01 (4K)`). Art inside them loses to any other claim, and
    /// among them the first folder by path that has a match supplies it.
    pub(crate) fn resolve(
        &self,
        listings: &HashMap<PathBuf, Vec<String>>,
    ) -> HashMap<(String, ImageKind), PathBuf> {
        type Candidate = (Option<PathBuf>, u8, usize, PathBuf);
        let mut best: HashMap<(String, ImageKind), Candidate> = HashMap::new();
        for claim in &self.claims {
            let Some(names) = listings.get(&claim.dir) else {
                continue;
            };
            for name in names {
                let Some((kind, rank)) = match_art(&claim.role, name) else {
                    continue;
                };
                let season_folder =
                    matches!(claim.role, ArtRole::SeasonFolder).then(|| claim.dir.clone());
                let candidate = (season_folder, rank, ext_rank(name), claim.dir.join(name));
                best.entry((claim.work_key.clone(), kind))
                    .and_modify(|current| {
                        if candidate < *current {
                            *current = candidate.clone();
                        }
                    })
                    .or_insert(candidate);
            }
        }
        best.into_iter()
            .map(|(key, (_, _, _, path))| (key, path))
            .collect()
    }
}

fn ext_rank(name: &str) -> usize {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    ART_EXTS
        .iter()
        .position(|e| *e == ext)
        .unwrap_or(ART_EXTS.len())
}

/// Names of the image files directly inside each of `dirs`, one
/// `read_dir` per directory. Unreadable directories list as empty and
/// non-UTF-8 names are skipped. Blocking.
pub(crate) fn list_art_dirs(dirs: Vec<PathBuf>) -> HashMap<PathBuf, Vec<String>> {
    dirs.into_iter()
        .map(|dir| {
            let names = list_art_dir(&dir);
            (dir, names)
        })
        .collect()
}

fn list_art_dir(dir: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(err) => {
            tracing::debug!(dir = %dir.display(), %err, "artwork: directory unreadable");
            return Vec::new();
        }
    };
    let mut names: Vec<String> = entries
        .flatten()
        // `file_type` comes from the dirent on most filesystems — no stat.
        // Symlinks are kept; the image route resolves them at serve time.
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            name.rsplit_once('.')
                .is_some_and(|(_, ext)| ART_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn folder(stem: &str) -> ArtRole {
        ArtRole::MovieFolder { stem: stem.into() }
    }

    #[test]
    fn movie_folder_names() {
        let r = folder("the matrix (1999)");
        for (name, want) in [
            ("poster.jpg", Some((ImageKind::Primary, GENERIC))),
            ("Folder.JPG", Some((ImageKind::Primary, GENERIC + 1))),
            ("cover.webp", Some((ImageKind::Primary, GENERIC + 2))),
            ("The Matrix (1999).png", Some((ImageKind::Primary, 0))),
            (
                "The Matrix (1999)-poster.jpeg",
                Some((ImageKind::Primary, 1)),
            ),
            ("fanart.jpg", Some((ImageKind::Backdrop, GENERIC))),
            ("background.jpg", Some((ImageKind::Backdrop, GENERIC + 1))),
            ("art.jpg", Some((ImageKind::Backdrop, GENERIC + 2))),
            ("backdrop.jpg", Some((ImageKind::Backdrop, GENERIC + 3))),
            (
                "The Matrix (1999)-fanart.jpg",
                Some((ImageKind::Backdrop, 0)),
            ),
            ("banner.png", Some((ImageKind::Banner, GENERIC))),
            ("logo.png", Some((ImageKind::Logo, GENERIC))),
            ("clearlogo.png", Some((ImageKind::Logo, GENERIC + 1))),
            ("poster.gif", None),
            ("poster", None),
            ("The Matrix (1999)-thumb.jpg", None),
            ("Other Movie.jpg", None),
        ] {
            assert_eq!(match_art(&r, name), want, "{name}");
        }
    }

    #[test]
    fn bare_movie_only_claims_stem_names() {
        let r = ArtRole::BareMovie {
            stem: "heat (1995)".into(),
        };
        assert_eq!(match_art(&r, "poster.jpg"), None);
        assert_eq!(match_art(&r, "fanart.jpg"), None);
        assert_eq!(
            match_art(&r, "Heat (1995).jpg"),
            Some((ImageKind::Primary, 0))
        );
        assert_eq!(
            match_art(&r, "Heat (1995)-fanart.jpg"),
            Some((ImageKind::Backdrop, 0))
        );
    }

    #[test]
    fn show_and_season_names() {
        assert_eq!(
            match_art(&ArtRole::ShowFolder, "show.jpg"),
            Some((ImageKind::Primary, GENERIC + 2))
        );
        assert_eq!(match_art(&ArtRole::ShowFolder, "season01-poster.jpg"), None);
        assert_eq!(match_art(&ArtRole::ShowFolder, "cover.jpg"), None);

        let s1 = ArtRole::ShowFolderSeason { number: 1 };
        for (name, want) in [
            ("season01-poster.jpg", Some((ImageKind::Primary, 0))),
            ("Season1-Poster.png", Some((ImageKind::Primary, 0))),
            ("Season01.jpg", Some((ImageKind::Primary, 1))),
            ("season01-fanart.jpg", Some((ImageKind::Backdrop, 0))),
            ("season01-banner.jpg", Some((ImageKind::Banner, 0))),
            ("season02-poster.jpg", None),
            ("season-specials-poster.jpg", None),
            ("season01-thumb.jpg", None),
            ("poster.jpg", None),
        ] {
            assert_eq!(match_art(&s1, name), want, "{name}");
        }
        let specials = ArtRole::ShowFolderSeason { number: 0 };
        assert_eq!(
            match_art(&specials, "season-specials-poster.jpg"),
            Some((ImageKind::Primary, 0))
        );
        assert_eq!(
            match_art(&specials, "season00-poster.jpg"),
            Some((ImageKind::Primary, 0))
        );

        assert_eq!(
            match_art(&ArtRole::SeasonFolder, "poster.jpg"),
            Some((ImageKind::Primary, GENERIC))
        );
        assert_eq!(match_art(&ArtRole::SeasonFolder, "fanart.jpg"), None);
    }

    #[test]
    fn episode_names() {
        let r = ArtRole::Episode {
            stem: "show - s01e01 - pilot".into(),
        };
        assert_eq!(
            match_art(&r, "Show - s01e01 - Pilot.jpg"),
            Some((ImageKind::Thumb, 0))
        );
        assert_eq!(
            match_art(&r, "Show - s01e01 - Pilot-thumb.jpg"),
            Some((ImageKind::Thumb, 1))
        );
        assert_eq!(match_art(&r, "poster.jpg"), None);
        assert_eq!(match_art(&r, "Show - s01e02.jpg"), None);
    }

    #[test]
    fn music_names() {
        assert_eq!(
            match_art(&ArtRole::AlbumFolder, "front.png"),
            Some((ImageKind::Primary, GENERIC + 2))
        );
        assert_eq!(match_art(&ArtRole::AlbumFolder, "fanart.png"), None);
        assert_eq!(
            match_art(&ArtRole::ArtistFolder, "artist.jpg"),
            Some((ImageKind::Primary, GENERIC))
        );
        assert_eq!(
            match_art(&ArtRole::ArtistFolder, "backdrop.jpg"),
            Some((ImageKind::Backdrop, GENERIC + 1))
        );
        assert_eq!(match_art(&ArtRole::ArtistFolder, "cover.jpg"), None);
    }

    fn listings(entries: &[(&str, &[&str])]) -> HashMap<PathBuf, Vec<String>> {
        entries
            .iter()
            .map(|(dir, names)| {
                (
                    PathBuf::from(dir),
                    names.iter().map(|n| (*n).to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn resolve_prefers_specific_then_listed_order_then_extension() {
        let mut claims = ArtClaims::default();
        claims.add(Path::new("/m/Heat"), folder("heat"), "movie");
        let got = claims.resolve(&listings(&[(
            "/m/Heat",
            &[
                "cover.jpg",
                "folder.jpg",
                "poster.png",
                "poster.jpg",
                "fanart.jpg",
                "Heat-fanart.png",
            ],
        )]));
        assert_eq!(
            got[&("movie".to_string(), ImageKind::Primary)],
            PathBuf::from("/m/Heat/poster.jpg")
        );
        assert_eq!(
            got[&("movie".to_string(), ImageKind::Backdrop)],
            PathBuf::from("/m/Heat/Heat-fanart.png")
        );

        let got = claims.resolve(&listings(&[("/m/Heat", &["poster.jpg", "Heat.jpg"])]));
        assert_eq!(
            got[&("movie".to_string(), ImageKind::Primary)],
            PathBuf::from("/m/Heat/Heat.jpg")
        );
    }

    #[test]
    fn season_numbered_show_art_beats_season_folder_poster() {
        let mut claims = ArtClaims::default();
        claims.add(
            Path::new("/tv/Show"),
            ArtRole::ShowFolderSeason { number: 1 },
            "s1",
        );
        claims.add(Path::new("/tv/Show/Season 01"), ArtRole::SeasonFolder, "s1");
        let got = claims.resolve(&listings(&[
            ("/tv/Show", &["season01-poster.jpg"]),
            ("/tv/Show/Season 01", &["poster.jpg"]),
        ]));
        assert_eq!(
            got[&("s1".to_string(), ImageKind::Primary)],
            PathBuf::from("/tv/Show/season01-poster.jpg")
        );
    }

    #[test]
    fn first_season_folder_by_path_supplies_season_art() {
        let mut claims = ArtClaims::default();
        for dir in ["/tv/Show/Season 01 (4K)", "/tv/Show/Season 01 (1080p)"] {
            claims.add(Path::new(dir), ArtRole::SeasonFolder, "s1");
        }
        claims.add(
            Path::new("/tv/Show"),
            ArtRole::ShowFolderSeason { number: 1 },
            "s1",
        );
        let season_art = [
            ("/tv/Show/Season 01 (1080p)", &["folder.png"][..]),
            ("/tv/Show/Season 01 (4K)", &["poster.jpg"][..]),
        ];
        let got = claims.resolve(&listings(&season_art));
        assert_eq!(
            got[&("s1".to_string(), ImageKind::Primary)],
            PathBuf::from("/tv/Show/Season 01 (1080p)/folder.png"),
            "folder order beats name rank and extension"
        );

        let mut with_show_art = season_art.to_vec();
        with_show_art.push(("/tv/Show", &["Season01.webp", "season01-fanart.jpg"]));
        let got = claims.resolve(&listings(&with_show_art));
        assert_eq!(
            got[&("s1".to_string(), ImageKind::Primary)],
            PathBuf::from("/tv/Show/Season01.webp")
        );
        assert_eq!(
            got[&("s1".to_string(), ImageKind::Backdrop)],
            PathBuf::from("/tv/Show/season01-fanart.jpg")
        );
    }

    #[test]
    fn same_rank_across_dirs_breaks_ties_by_path() {
        let mut claims = ArtClaims::default();
        claims.add(Path::new("/b/Show"), ArtRole::ShowFolder, "series");
        claims.add(Path::new("/a/Show"), ArtRole::ShowFolder, "series");
        let got = claims.resolve(&listings(&[
            ("/a/Show", &["poster.jpg"]),
            ("/b/Show", &["poster.jpg"]),
        ]));
        assert_eq!(
            got[&("series".to_string(), ImageKind::Primary)],
            PathBuf::from("/a/Show/poster.jpg")
        );
    }

    #[test]
    fn listing_keeps_only_image_files() {
        let dir = tempdir().unwrap();
        for name in ["poster.JPG", "movie.mkv", "notes.txt", "fanart.webp"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        std::fs::create_dir(dir.path().join("cover.jpg")).unwrap();
        let missing = dir.path().join("missing");
        let got = list_art_dirs(vec![dir.path().to_path_buf(), missing.clone()]);
        assert_eq!(got[dir.path()], vec!["fanart.webp", "poster.JPG"]);
        assert!(got[&missing].is_empty());
    }
}
