// SPDX-License-Identifier: AGPL-3.0-only
//
// Plex- and torrent-style filename / folder classifier.
//
// Two responsibilities, deliberately separated:
//
// 1. **Identity** — a movie's identity is its enclosing folder (or its
//    filename, if the file is bare at a library root). An episode's is its
//    show, season and episode numbers, wherever its season folder sits.
//
// 2. **Content** — title / year / edition / part. Tries Plex's canonical
//    forms first, then falls back to a fuzzy parser that strips torrent
//    suffixes (1080p, BluRay, x265, [Group], …).
//
// Merging two movie candidates that resolve to the same canonical work is
// the metadata provider's job — the scanner emits one per location.

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::OnceLock,
};

// Canonical home is noflex-core (shared with the metadata/GraphQL tag
// path); re-exported so existing scanner call sites stay put.
pub use noflex_core::slugify;
use regex::Regex;

// ---- Public types ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMovie {
    pub title: String,
    pub year: Option<i32>,
    pub edition: Option<String>,
    pub part: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MovieIdentity {
    /// File lives in an enclosing per-movie folder. Identity is that
    /// folder's absolute path; cd1/cd2 in the same folder share identity.
    InFolder(PathBuf),
    /// File lives at a library root with no enclosing folder. Identity is
    /// the file path itself (each bare file is its own candidate).
    Bare(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieClassification {
    pub identity: MovieIdentity,
    pub parsed: ParsedMovie,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEpisode {
    pub show: String,
    pub show_year: Option<i32>,
    pub season: u32,
    pub episode: u32,
    pub episode_end: Option<u32>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeClassification {
    pub parsed: ParsedEpisode,
    /// Absolute path of the matched season folder, or of the file's parent
    /// when no ancestor below the root is one. Locates season and show
    /// artwork only: files in "Season 1 (1080p)" and "Season 1 (4K)" are
    /// versions of the same episodes.
    pub season_folder: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonFolderInfo {
    pub number: u32,
    /// Some when the folder name combines show + season + junk, like
    /// "Game of Thrones Season 1 (1080p Tigole)".
    pub show: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicClassification {
    pub artist: String,
    pub album: String,
    pub track: String,
    pub track_number: Option<u32>,
    pub artist_path: PathBuf,
    pub album_path: PathBuf,
    pub track_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoClassification {
    pub title: String,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    pub path: PathBuf,
}

// ---- Junk tag list ---------------------------------------------------------

/// Tokens that, when encountered in a filename, mark the end of the title
/// region. Case-insensitive. Order is by group only — alternation tries them
/// all.
const JUNK_TAGS: &[&str] = &[
    // Resolution
    "1080p",
    "720p",
    "2160p",
    "4k",
    "8k",
    "480p",
    "1440p",
    "360p",
    "576p",
    // Source
    "bluray",
    "brrip",
    "bdrip",
    "webrip",
    "web-dl",
    "webdl",
    "web",
    "hdtv",
    "dvdrip",
    "dvd",
    "hdrip",
    "dsr",
    "pdtv",
    "remux",
    "uhd",
    "ultrahd",
    "blu-ray",
    "stv",
    // Streaming services
    "amzn",
    "dsnp",
    "hmax",
    "atvp",
    // HDR
    "hdr",
    "hdr10",
    "hdr10+",
    "hdr10plus",
    "sdr",
    "dv",
    "dovi",
    "dolby-vision",
    // Codec
    "x264",
    "x265",
    "h264",
    "h265",
    "hevc",
    "avc",
    "av1",
    "xvid",
    "divx",
    "mpeg2",
    // Audio
    "aac",
    "ac3",
    "eac3",
    "ddp",
    "ddp5",
    "dd5",
    "dts",
    "dts-hd",
    "dts-x",
    "truehd",
    "atmos",
    "mp3",
    "flac",
    "ogg",
    // Bit depth
    "10bit",
    "8bit",
    "12bit",
    // Release tags. Cuts like "Extended" are editions (`EDITIONS`); a
    // remaster or another framing is a version of one.
    "repack",
    "proper",
    "remastered",
    "open matte",
    "internal",
    "limited",
];

/// Edition labels a filename may carry besides `{edition-…}`, each with the
/// name it's stored under, so every spelling of a cut lands in one edition.
const EDITIONS: &[(&str, &str)] = &[
    (r"director'?s?[\s-]+cut", "Director's Cut"),
    (
        r"extended(?:[\s-]+(?:edition|cut|version))?",
        "Extended Edition",
    ),
    (
        r"theatrical(?:[\s-]+(?:cut|edition|version|release))?",
        "Theatrical Cut",
    ),
    (r"unrated(?:[\s-]+(?:edition|cut|version))?", "Unrated"),
    (r"uncut", "Uncut"),
    (r"final[\s-]+cut", "Final Cut"),
    (r"special[\s-]+edition", "Special Edition"),
    (r"ultimate[\s-]+(?:edition|cut)", "Ultimate Edition"),
    (r"collector'?s?[\s-]+edition", "Collector's Edition"),
    (r"imax(?:[\s-]+edition)?", "IMAX"),
];

/// Abbreviated labels. "DC" starts titles too ("DC League of Super-Pets"),
/// so these only count between a release year and the quality tags.
const EDITION_ABBREVIATIONS: &[(&str, &str)] = &[("dc", "Director's Cut")];

/// Words for "season" that name a season folder.
const SEASON_WORDS: &str = r"season|staffel|stagione|saison|temporada|seizoen|säsong|sæson|sesong|kausi|sezon|sezona|sezóna|sezonul|series|시즌|シーズン|сезон";

// ---- Regex helpers ---------------------------------------------------------

fn year_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\b(19\d{2}|20\d{2})\b").unwrap())
}

fn junk_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        let pat = format!(r"(?i)\b(?:{})\b", JUNK_TAGS.join("|"));
        Regex::new(&pat).unwrap()
    })
}

fn edition_brace_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\{edition-([^}]+)\}").unwrap())
}

fn edition_label_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| label_re(EDITIONS))
}

fn edition_abbreviation_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| label_re(EDITION_ABBREVIATIONS))
}

/// One capture group per label, in order, so the group says which matched.
fn label_re(labels: &[(&str, &str)]) -> Regex {
    let alternatives: Vec<_> = labels.iter().map(|(p, _)| format!("({p})")).collect();
    Regex::new(&format!(r"(?i)\b(?:{})\b", alternatives.join("|"))).unwrap()
}

/// Every label in `s[range]`, as its span in `s` and the edition it names.
fn labels_in<'a>(
    re: &'a Regex,
    labels: &'static [(&'static str, &'static str)],
    s: &'a str,
    range: Range<usize>,
) -> impl Iterator<Item = (Range<usize>, &'static str)> + 'a {
    let offset = range.start;
    re.captures_iter(&s[range]).filter_map(move |c| {
        let (i, m) = c
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(i, m)| Some((i, m?)))?;
        Some((m.start() + offset..m.end() + offset, labels[i - 1].1))
    })
}

fn part_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\s*-?\s*(?:cd|part|disc)\s*0*(\d+)\b").unwrap())
}

fn balanced_brackets_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\([^)]*\)|\[[^\]]*\]|\{[^}]*\}").unwrap())
}

fn whitespace_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\s+").unwrap())
}

/// An episode marker: `S01E02`, `s1e2`, `S01xE02`, `S01.E02`, `S1 E2`,
/// `S01x02`, `1x02` or `Season 1 Episode 2`, then any further episodes of a
/// multi-episode file (`E01E02`, `E23-E24-E26`, `E01-02`, `02x03x04`). The
/// match starts one separator early, as the regex crate has no lookbehind.
fn episode_marker_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?ix)
            (?:^|[\s._\[(-])
            (?P<marker>
                (?:
                    s(?P<s1>\d{1,4})[\s._-]?x?[\s._-]?e(?P<e1>\d{1,4})
                  | s(?P<s2>\d{1,4})x(?P<e2>\d{1,4})
                  | (?P<s3>\d{1,2})x(?P<e3>\d{2,3})
                  | season[\s._-]*(?P<s4>\d{1,4})[\s._-]*episode[\s._-]*(?P<e4>\d{1,4})
                )
                (?P<more>(?:(?:-?e|x|-)\d{1,4})*)
            )
            \b
            ",
        )
        .unwrap()
    })
}

/// A file in a season folder named by its episode alone: `Episode 16`,
/// `Episode 16 - Title`, `02 - Title`, `2. Title`, `02`. A bare number needs
/// a separator before any title, so `12 Angry Men` isn't episode 12.
fn bare_episode_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^(?:ep(?:isode)?[\s._-]*)?(\d{1,4})(?:\s*[-.]\s*(.*))?$").unwrap()
    })
}

// ---- Fuzzy filename parser -------------------------------------------------

/// Parse a string (a filename stem or a folder name) for movie-style content:
/// title, year, edition, part. Tolerates torrent-style suffixes.
///
/// Always succeeds — returns an empty title for inputs that have no
/// recognisable prefix before the first junk token.
pub fn parse_filename_movie(input: &str) -> ParsedMovie {
    let mut work = input.to_string();

    // 1. Edition tag {edition-X}.
    let edition = edition_brace_re()
        .captures(&work)
        .map(|c| c.get(1).unwrap().as_str().trim().to_string());
    if edition.is_some() {
        work = edition_brace_re().replace(&work, "").to_string();
    }

    // 2. Part marker " - cd1", " - disc1", " - part1".
    let part = part_re()
        .captures(&work)
        .and_then(|c| c.get(1).unwrap().as_str().parse::<u32>().ok());
    if part.is_some() {
        work = part_re().replace(&work, "").to_string();
    }

    // 3. Normalise separators.
    let normalised = normalise_separators(&work);

    // 4. The first junk tag ends the title region. Inside it, the release
    //    year is the last bracketed year, else the last bare one: titles
    //    carry year-like numbers ("Blade Runner 2049 (2017)", "2001 A
    //    Space Odyssey 1968"), release years trail them. A year that
    //    would leave no title is the title ("1917").
    let junk_cut = junk_re()
        .find(&normalised)
        .map_or(normalised.len(), |j| j.start());
    let region = &normalised[..junk_cut];
    let bracketed = |y: &regex::Match<'_>| {
        region[..y.start()].ends_with(['(', '[']) && region[y.end()..].starts_with([')', ']'])
    };
    let candidates: Vec<_> = year_re()
        .find_iter(region)
        .filter(|y| !clean_title(&region[..y.start()]).is_empty())
        .collect();
    let chosen = candidates
        .iter()
        .rev()
        .find(|y| bracketed(y))
        .or_else(|| candidates.last());
    let label = edition_label(&normalised, chosen.map(|y| y.end()), junk_cut);
    let (cut, year) = match chosen {
        Some(y) => (y.start(), y.as_str().parse().ok()),
        None => (
            label.as_ref().map_or(junk_cut, |l| l.0.start.min(junk_cut)),
            None,
        ),
    };
    let edition = edition.or_else(|| label.map(|l| l.1.to_string()));

    let title_prefix = &normalised[..cut];
    let title = clean_title(title_prefix);

    ParsedMovie {
        title,
        year,
        edition,
        part,
    }
}

/// The edition a cut named among the release tags gives: anywhere after the
/// release year, or with no year, only trailing the title ("Léon Extended",
/// not "The Extended Family"). Abbreviations only count after a year.
fn edition_label(
    normalised: &str,
    year_end: Option<usize>,
    junk_cut: usize,
) -> Option<(Range<usize>, &'static str)> {
    let full = |range| labels_in(edition_label_re(), EDITIONS, normalised, range);
    match year_end {
        Some(from) => full(from..normalised.len()).next().or_else(|| {
            labels_in(
                edition_abbreviation_re(),
                EDITION_ABBREVIATIONS,
                normalised,
                from..junk_cut.max(from),
            )
            .next()
        }),
        None => full(0..normalised.len()).find(|(span, _)| {
            let between = &normalised[span.end..junk_cut.max(span.end)];
            !clean_title(&normalised[..span.start]).is_empty()
                && clean_title(&edition_label_re().replace_all(between, "")).is_empty()
        }),
    }
}

/// Dots and underscores become spaces, but only when there are no spaces
/// yet or many dots (a classic dot-separated torrent name), so a
/// perfectly-spaced name that happens to contain a dot ("Mr. Robot")
/// isn't mangled.
fn normalise_separators(s: &str) -> String {
    if s.contains(' ') && s.matches('.').count() < 3 {
        s.to_string()
    } else {
        s.replace(['.', '_'], " ")
    }
}

/// Strip balanced brackets, then trim non-alphanumeric boundary characters,
/// then collapse internal whitespace. Preserves apostrophes/internal
/// punctuation (e.g. "Director's Cut").
fn clean_title(s: &str) -> String {
    let de_bracketed = balanced_brackets_re().replace_all(s, "");
    let trimmed = de_bracketed
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_string();
    whitespace_re().replace_all(&trimmed, " ").into_owned()
}

// ---- Season folder parser --------------------------------------------------

/// Recognise a season-bearing directory name. Returns the season number plus
/// an optional show prefix when the folder combines both
/// (e.g. "Game of Thrones Season 1 (1080p ...)").
///
/// Supported forms, with "Season" in any language of `SEASON_WORDS`:
///   - `Season 1`, `Season 01`, `Season1`, `Season 1 (anything)`,
///     `Season 1 1080p BluRay`
///   - `<Show> Season 1`, with the same tails
///   - `3. Staffel`
///   - `Specials` → season 0
///   - `S01`, `S1`, and a release's `<Show>.S01.1080p.WEB-DL-GRP`
pub fn parse_season_folder(name: &str) -> Option<SeasonFolderInfo> {
    let n = name.trim();

    static NAMED: OnceLock<Regex> = OnceLock::new();
    let named = NAMED.get_or_init(|| {
        Regex::new(&format!(
            r"(?i)^(?:(?P<show>.+?)[\s._-]+)?(?:{SEASON_WORDS})[\s._-]*(?P<n>\d{{1,4}})(?P<tail>.*)$"
        ))
        .unwrap()
    });
    if let Some(c) = named.captures(n)
        && is_season_tail(&c["tail"])
    {
        return Some(SeasonFolderInfo {
            number: c["n"].parse().ok()?,
            show: c.name("show").map(|m| show_name(m.as_str())),
        });
    }

    static ORDINAL: OnceLock<Regex> = OnceLock::new();
    let ordinal = ORDINAL.get_or_init(|| Regex::new(r"(?i)^(\d{1,3})\.\s*staffel$").unwrap());
    if let Some(c) = ordinal.captures(n) {
        return Some(SeasonFolderInfo {
            number: c[1].parse().ok()?,
            show: None,
        });
    }

    static SPECIALS: OnceLock<Regex> = OnceLock::new();
    let specials = SPECIALS.get_or_init(|| Regex::new(r"(?i)^specials?\s*$").unwrap());
    if specials.is_match(n) {
        return Some(SeasonFolderInfo {
            number: 0,
            show: None,
        });
    }

    static SHORT: OnceLock<Regex> = OnceLock::new();
    let short = SHORT.get_or_init(|| {
        Regex::new(r"(?i)^\[?(?:(?P<show>.+?)[\s._\[-]+)?s(?P<n>\d{1,4})(?P<tail>[\s._\]-].*)?$")
            .unwrap()
    });
    if let Some(c) = short.captures(n)
        && c.name("tail").is_none_or(|t| is_season_tail(t.as_str()))
    {
        return Some(SeasonFolderInfo {
            number: c["n"].parse().ok()?,
            show: c.name("show").map(|m| show_name(m.as_str())),
        });
    }

    None
}

/// What may follow a season number in a folder name: nothing, something
/// bracketed, or release tags. Anything else ("Season 1 Episode 2") means
/// the folder isn't a season's.
fn is_season_tail(tail: &str) -> bool {
    let t = tail.trim_start_matches([' ', '.', '_', '-', ']']);
    t.is_empty()
        || t.starts_with(['(', '['])
        || junk_re()
            .find(&normalise_separators(t))
            .is_some_and(|m| m.start() == 0)
}

/// A show name from a season folder's prefix, without any release tags
/// (e.g. "Show Name 1080p" → "Show Name").
fn show_name(raw: &str) -> String {
    let raw = raw.trim();
    let cleaned = parse_filename_movie(raw);
    if cleaned.title.is_empty() {
        raw.to_string()
    } else {
        cleaned.title
    }
}

// ---- Episode filename parser -----------------------------------------------

struct FilenameEpisode {
    season: u32,
    episode: u32,
    episode_end: Option<u32>,
    title: Option<String>,
    /// What came before the season/episode marker — the filename's idea of
    /// the show name. Often abbreviated in torrents, so the directory tree
    /// is preferred when available.
    show_prefix: Option<String>,
}

/// The episode title in what follows the `SxxEyy` marker: everything up
/// to the first quality/release token, so the version tags that tell two
/// files of one episode apart (`- 1080p`, `.WEB-DL.x264`) never become
/// its title. Year-like numbers are not tokens (`S01E05 - 1984`). `None`
/// when no title precedes the tags.
fn episode_title(rest: &str) -> Option<String> {
    let normalised = normalise_separators(rest);
    let cut = junk_re()
        .find(&normalised)
        .map_or(normalised.len(), |j| j.start());
    let title = clean_title(&normalised[..cut]);
    (!title.is_empty()).then_some(title)
}

fn parse_episode_filename(name: &str) -> Option<FilenameEpisode> {
    let stem = file_stem(name);
    let caps = episode_marker_re().captures(stem)?;
    let marker = caps.name("marker")?;
    let (season, episode) = [("s1", "e1"), ("s2", "e2"), ("s3", "e3"), ("s4", "e4")]
        .into_iter()
        .find_map(|(s, e)| Some((caps.name(s)?, caps.name(e)?)))?;
    let season = season.as_str().parse().ok()?;
    let episode = episode.as_str().parse().ok()?;
    let episode_end = caps
        .name("more")
        .and_then(|m| m.as_str().rsplit(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|&end| end > episode);
    let title = episode_title(&stem[marker.end()..]);
    let show_prefix =
        Some(clean_title(&normalise_separators(&stem[..marker.start()]))).filter(|p| !p.is_empty());
    Some(FilenameEpisode {
        season,
        episode,
        episode_end,
        title,
        show_prefix,
    })
}

/// An episode named by its number alone, as a file directly in a season
/// folder may be.
fn parse_bare_episode(name: &str, season: u32) -> Option<FilenameEpisode> {
    let caps = bare_episode_re().captures(file_stem(name).trim())?;
    Some(FilenameEpisode {
        season,
        episode: caps[1].parse().ok()?,
        episode_end: None,
        title: caps.get(2).and_then(|m| episode_title(m.as_str())),
        show_prefix: None,
    })
}

/// `name` without its extension.
fn file_stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) => {
            stem
        }
        _ => name,
    }
}

// ---- Movie classifier ------------------------------------------------------

fn find_root<'a>(file: &Path, roots: &'a [PathBuf]) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|r| file.starts_with(r))
        // Prefer the deepest matching root (in case roots are nested).
        .max_by_key(|r| r.as_os_str().len())
}

/// Classify a file as a movie candidate. Returns `None` only if the file
/// isn't under any of the supplied library roots.
pub fn classify_movie(file: &Path, roots: &[PathBuf]) -> Option<MovieClassification> {
    let root = find_root(file, roots)?;
    let parent = file.parent()?;

    let identity = if parent == root {
        MovieIdentity::Bare(file.to_path_buf())
    } else {
        MovieIdentity::InFolder(parent.to_path_buf())
    };

    // Parse the filename stem for edition/part (only the file itself can
    // carry those) and as a fallback for title/year.
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let file_parsed = parse_filename_movie(stem);

    // If we're in a folder, prefer the folder's parse for title/year.
    let parsed = match &identity {
        MovieIdentity::InFolder(folder) => {
            let folder_name = folder.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let folder_parsed = parse_filename_movie(folder_name);
            ParsedMovie {
                title: if folder_parsed.title.is_empty() {
                    file_parsed.title.clone()
                } else {
                    folder_parsed.title
                },
                year: folder_parsed.year.or(file_parsed.year),
                edition: file_parsed.edition.clone(),
                part: file_parsed.part,
            }
        }
        MovieIdentity::Bare(_) => file_parsed,
    };

    Some(MovieClassification { identity, parsed })
}

// ---- Episode classifier ----------------------------------------------------

/// Classify a file as a TV episode candidate. Returns `None` if the filename
/// has no SxxExx marker, or if no library root is an ancestor.
pub fn classify_episode(file: &Path, roots: &[PathBuf]) -> Option<EpisodeClassification> {
    let root = find_root(file, roots)?;
    let filename = file.file_name()?.to_str()?;
    let from_file = match parse_episode_filename(filename) {
        Some(parsed) => parsed,
        None => {
            let parent = file.parent().filter(|p| p != &root.as_path())?;
            let folder = parse_season_folder(parent.file_name()?.to_str()?)?;
            parse_bare_episode(filename, folder.number)?
        }
    };

    // Walk parent dirs from the file's parent up to (but not including) the
    // root, looking for a season-bearing directory.
    let mut season_folder: Option<(PathBuf, SeasonFolderInfo)> = None;
    let mut cursor = file.parent();
    while let Some(dir) = cursor {
        if dir == root {
            break;
        }
        let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(info) = parse_season_folder(name) {
            season_folder = Some((dir.to_path_buf(), info));
            break;
        }
        cursor = dir.parent();
    }

    let (season_folder_path, season_info, show_dir) = match season_folder {
        Some(sf) => {
            // If a season folder was found it MUST agree with the filename —
            // guards against misclassifying loose files.
            if sf.1.number != from_file.season {
                return None;
            }
            let show_dir = sf.0.parent().map(Path::to_path_buf);
            (sf.0, sf.1, show_dir)
        }
        None => {
            // No season folder anywhere up the tree: the file's parent
            // stands in (it is then the show folder, for artwork), and names
            // the show unless it's one episode's release folder.
            let parent = file.parent()?.to_path_buf();
            let show_dir = parent
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| !episode_marker_re().is_match(n))
                .map(|_| parent.clone());
            (
                parent,
                SeasonFolderInfo {
                    number: from_file.season,
                    show: None,
                },
                show_dir,
            )
        }
    };

    // Show name resolution priority:
    //   1. Combined form (`<Show> Season N` folder).
    //   2. The show folder: above the season folder, or holding the file.
    //   3. Filename's show prefix.
    let (show, show_year) = if let Some(combined_show) = season_info.show.clone() {
        // Combined folder may carry "Show (Year)" or "Show Name 1080p"; run it
        // through the fuzzy parser so we strip any junk and capture year.
        let p = parse_filename_movie(&combined_show);
        let title = if p.title.is_empty() {
            combined_show
        } else {
            p.title
        };
        (title, p.year)
    } else if let Some(show_dir) = show_dir.filter(|p| p != root) {
        let name = show_dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let p = parse_filename_movie(name);
        if p.title.is_empty() {
            (
                from_file
                    .show_prefix
                    .clone()
                    .unwrap_or_else(|| name.to_string()),
                p.year,
            )
        } else {
            (p.title, p.year)
        }
    } else {
        let title = from_file.show_prefix.clone().unwrap_or_default();
        if title.is_empty() {
            return None;
        }
        (title, None)
    };

    Some(EpisodeClassification {
        parsed: ParsedEpisode {
            show,
            show_year,
            season: from_file.season,
            episode: from_file.episode,
            episode_end: from_file.episode_end,
            title: from_file.title.clone(),
        },
        season_folder: season_folder_path,
    })
}

// ---- Music / photo classifiers ---------------------------------------------

/// `NN - Title`, `NN. Title`, `D-NN_Title`: an explicit separator.
fn track_number_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^(?:disc\s*)?(?:\d{1,2}[-.])?(\d{1,3})\s*[-._]\s*(.+)$").unwrap()
    })
}

/// `01 Title`, `2-03 Title`: whitespace alone only separates a
/// zero-padded number, so `99 Luftballons` stays a title.
fn padded_track_number_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^(?:disc\s*)?(?:\d{1,2}[-.])?(0\d{1,2})\s+([^-._\s].*)$").unwrap()
    })
}

fn split_track_number(stem: &str) -> (Option<u32>, &str) {
    padded_track_number_re()
        .captures(stem)
        .or_else(|| track_number_re().captures(stem))
        .and_then(|captures| {
            let number = captures.get(1)?.as_str().parse().ok()?;
            Some((Some(number), captures.get(2)?.as_str()))
        })
        .unwrap_or((None, stem))
}

const UNKNOWN_ALBUM: &str = "Unknown Album";

fn clean_media_name(name: &str) -> String {
    let name = name.replace(['_', '.'], " ");
    clean_title(&name)
}

/// Classify the common `Artist/Album[/Disc N]/NN - Track.ext` layout.
/// `Artist/NN - Track.ext` lands in an "Unknown Album"; a flat
/// `Artist - Album - [NN - ]Track.ext` filename is also accepted.
pub fn classify_music(file: &Path, roots: &[PathBuf]) -> Option<MusicClassification> {
    let root = find_root(file, roots)?;
    let relative = file.strip_prefix(root).ok()?;
    let dirs: Vec<_> = relative.parent()?.components().collect();
    let stem = file.file_stem()?.to_str()?;

    let (artist, album, artist_path, album_path, track_stem) = if dirs.len() >= 2 {
        let artist = dirs.first()?.as_os_str().to_str()?;
        let album = dirs.get(1)?.as_os_str().to_str()?;
        (
            clean_media_name(artist),
            clean_media_name(album),
            root.join(artist),
            root.join(artist).join(album),
            stem.to_string(),
        )
    } else if let [artist] = dirs.as_slice() {
        let artist = artist.as_os_str().to_str()?;
        (
            clean_media_name(artist),
            UNKNOWN_ALBUM.to_owned(),
            root.join(artist),
            root.join(artist),
            stem.to_string(),
        )
    } else {
        let parts: Vec<_> = stem.split(" - ").map(str::trim).collect();
        if parts.len() < 3 {
            return None;
        }
        let artist = clean_media_name(parts[0]);
        let album = clean_media_name(parts[1]);
        let track_stem = parts[2..].join(" - ");
        (
            artist,
            album,
            file.with_file_name(parts[0]),
            file.with_file_name(format!("{} - {}", parts[0], parts[1])),
            track_stem,
        )
    };
    if artist.is_empty() || album.is_empty() {
        return None;
    }

    let (track_number, track) = split_track_number(&track_stem);
    let track = clean_media_name(track);
    if track.is_empty() {
        return None;
    }

    Some(MusicClassification {
        artist,
        album,
        track,
        track_number,
        artist_path,
        album_path,
        track_path: file.with_extension(""),
    })
}

/// Derive stable display metadata from a photo's filename. Leading ISO dates
/// are retained in the title and supply the timeline year.
pub fn classify_photo(file: &Path, roots: &[PathBuf]) -> Option<PhotoClassification> {
    find_root(file, roots)?;
    let stem = file.file_stem()?.to_str()?;
    let title = clean_media_name(stem);
    if title.is_empty() {
        return None;
    }
    let year = year_re()
        .find(stem)
        .filter(|m| m.start() == 0)
        .and_then(|m| m.as_str().parse().ok());
    let release_date = stem
        .get(..10)
        .filter(|date| {
            time::Date::parse(
                date,
                &time::macros::format_description!("[year]-[month]-[day]"),
            )
            .is_ok()
        })
        .map(str::to_string);
    Some(PhotoClassification {
        title,
        year,
        release_date,
        path: file.with_extension(""),
    })
}

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/lib")
    }
    fn roots() -> Vec<PathBuf> {
        vec![root()]
    }

    #[test]
    fn music_artist_album_track_layout() {
        let p = PathBuf::from("/lib/Miles Davis/Kind of Blue/02 - Freddie Freeloader.flac");
        let c = classify_music(&p, &roots()).expect("classifies");
        assert_eq!(c.artist, "Miles Davis");
        assert_eq!(c.album, "Kind of Blue");
        assert_eq!(c.track, "Freddie Freeloader");
        assert_eq!(c.track_number, Some(2));
        assert_eq!(c.artist_path, PathBuf::from("/lib/Miles Davis"));
        assert_eq!(c.album_path, PathBuf::from("/lib/Miles Davis/Kind of Blue"));
    }

    #[test]
    fn music_disc_subfolder_and_disc_track_number() {
        let p = PathBuf::from("/lib/Artist/Album/Disc 2/2-03 Song.opus");
        let c = classify_music(&p, &roots()).expect("classifies");
        assert_eq!(c.album, "Album");
        assert_eq!(c.track, "Song");
        assert_eq!(c.track_number, Some(3));
    }

    #[test]
    fn music_flat_filename_layout() {
        let p = PathBuf::from("/lib/Bjork - Debut - 01 - Human Behaviour.mp3");
        let c = classify_music(&p, &roots()).expect("classifies");
        assert_eq!((c.artist.as_str(), c.album.as_str()), ("Bjork", "Debut"));
        assert_eq!(c.track, "Human Behaviour");
        assert_eq!(c.track_number, Some(1));
    }

    #[test]
    fn music_one_folder_layout_falls_back_to_unknown_album() {
        let p = PathBuf::from("/lib/Nena/01 - 99 Luftballons.mp3");
        let c = classify_music(&p, &roots()).expect("classifies");
        assert_eq!(c.artist, "Nena");
        assert_eq!(c.album, UNKNOWN_ALBUM);
        assert_eq!(c.track, "99 Luftballons");
        assert_eq!(c.track_number, Some(1));
        assert_eq!(c.artist_path, PathBuf::from("/lib/Nena"));
    }

    #[test]
    fn music_flat_filename_without_track_number() {
        let p = PathBuf::from("/lib/Nena - 99 Luftballons - 99 Luftballons.mp3");
        let c = classify_music(&p, &roots()).expect("classifies");
        assert_eq!(
            (c.artist.as_str(), c.album.as_str()),
            ("Nena", "99 Luftballons")
        );
        assert_eq!(c.track, "99 Luftballons");
        assert_eq!(c.track_number, None);
    }

    #[test]
    fn leading_digits_need_a_separator_to_be_a_track_number() {
        for (stem, number, title) in [
            ("99 Luftballons", None, "99 Luftballons"),
            ("1984", None, "1984"),
            ("2 Become 1", None, "2 Become 1"),
            ("01 Song", Some(1), "Song"),
            ("07. Song", Some(7), "Song"),
            ("12 - Song", Some(12), "Song"),
            ("2-03 Song", Some(3), "Song"),
            ("2-03 - Song", Some(3), "Song"),
        ] {
            assert_eq!(split_track_number(stem), (number, title), "{stem}");
        }
        let p = PathBuf::from("/lib/Nena/Album/99 Luftballons.mp3");
        let c = classify_music(&p, &roots()).expect("classifies");
        assert_eq!((c.track.as_str(), c.track_number), ("99 Luftballons", None));
    }

    #[test]
    fn photo_filename_supplies_title_and_year() {
        let p = PathBuf::from("/lib/Trips/2024-06-12_Beach_Sunset.HEIC");
        let c = classify_photo(&p, &roots()).expect("classifies");
        assert_eq!(c.title, "2024-06-12 Beach Sunset");
        assert_eq!(c.year, Some(2024));
        assert_eq!(c.release_date.as_deref(), Some("2024-06-12"));
        assert_eq!(c.path, PathBuf::from("/lib/Trips/2024-06-12_Beach_Sunset"));
    }

    #[test]
    fn photo_invalid_iso_prefix_has_no_release_date() {
        let p = PathBuf::from("/lib/Trips/2024-99-99_Beach_Sunset.HEIC");
        let c = classify_photo(&p, &roots()).expect("classifies");
        assert_eq!(c.year, Some(2024));
        assert_eq!(c.release_date, None);
    }

    // ---- parse_filename_movie ----------------------------------------------

    #[test]
    fn fuzzy_strict_plex() {
        let p = parse_filename_movie("The Matrix (1999)");
        assert_eq!(p.title, "The Matrix");
        assert_eq!(p.year, Some(1999));
    }

    #[test]
    fn fuzzy_strict_plex_with_edition() {
        let p = parse_filename_movie("Inception (2010) {edition-Director's Cut}");
        assert_eq!(p.title, "Inception");
        assert_eq!(p.year, Some(2010));
        assert_eq!(p.edition.as_deref(), Some("Director's Cut"));
    }

    #[test]
    fn fuzzy_strict_plex_with_part() {
        let p = parse_filename_movie("Blade Runner (1982) - cd2");
        assert_eq!(p.title, "Blade Runner");
        assert_eq!(p.year, Some(1982));
        assert_eq!(p.part, Some(2));
    }

    #[test]
    fn fuzzy_torrent_dot_separated() {
        let p = parse_filename_movie("Inception.2010.1080p.BluRay.x264");
        assert_eq!(p.title, "Inception");
        assert_eq!(p.year, Some(2010));
    }

    #[test]
    fn fuzzy_torrent_with_group_brackets() {
        let p = parse_filename_movie("[Tigole] Inception (2010) 4K HDR");
        assert_eq!(p.title, "Inception");
        assert_eq!(p.year, Some(2010));
    }

    #[test]
    fn fuzzy_torrent_multi_dash_no_year() {
        let p = parse_filename_movie("Mission - Impossible - Ghost Protocol - 1080p AV1 BluRay");
        assert_eq!(p.title, "Mission - Impossible - Ghost Protocol");
        assert_eq!(p.year, None);
    }

    #[test]
    fn fuzzy_bare_no_year() {
        let p = parse_filename_movie("Inception");
        assert_eq!(p.title, "Inception");
        assert_eq!(p.year, None);
    }

    #[test]
    fn fuzzy_year_like_number_in_title() {
        for (input, title, year) in [
            ("Blade Runner 2049 (2017)", "Blade Runner 2049", Some(2017)),
            (
                "Blade.Runner.2049.2017.1080p.BluRay",
                "Blade Runner 2049",
                Some(2017),
            ),
            (
                "2001 A Space Odyssey (1968)",
                "2001 A Space Odyssey",
                Some(1968),
            ),
            ("1917 (2019)", "1917", Some(2019)),
            ("1917.2019.2160p.UHD", "1917", Some(2019)),
            ("1917", "1917", None),
            ("Zodiac (2007) - 1080p AV1 BluRay", "Zodiac", Some(2007)),
        ] {
            let p = parse_filename_movie(input);
            assert_eq!((p.title.as_str(), p.year), (title, year), "{input}");
        }
    }

    #[test]
    fn fuzzy_edition_labels() {
        for (input, title, year, edition) in [
            (
                "Sintel (2021) - DC - 1080p AV1 BluRay HDR10",
                "Sintel",
                Some(2021),
                Some("Director's Cut"),
            ),
            (
                "Sintel.2021.Directors.Cut.1080p.BluRay.x264-GRP",
                "Sintel",
                Some(2021),
                Some("Director's Cut"),
            ),
            (
                "Sintel 2021 Extended 2160p UHD BluRay",
                "Sintel",
                Some(2021),
                Some("Extended Edition"),
            ),
            (
                "Sintel (2021) [Unrated] [1080p]",
                "Sintel",
                Some(2021),
                Some("Unrated"),
            ),
            (
                "Léon The Professional Extended",
                "Léon The Professional",
                None,
                Some("Extended Edition"),
            ),
            (
                "Sintel (2021) {edition-Final Cut} Extended",
                "Sintel",
                Some(2021),
                Some("Final Cut"),
            ),
            // Labels that are part of a title, and a remaster, aren't editions.
            ("The Extended Family", "The Extended Family", None, None),
            ("Uncut Gems (2019)", "Uncut Gems", Some(2019), None),
            (
                "DC League of Super-Pets (2022) 1080p",
                "DC League of Super-Pets",
                Some(2022),
                None,
            ),
            (
                "Rain Man 1988 REMASTERED 1080p BluRay",
                "Rain Man",
                Some(1988),
                None,
            ),
        ] {
            let p = parse_filename_movie(input);
            assert_eq!(
                (p.title.as_str(), p.year, p.edition.as_deref()),
                (title, year, edition),
                "{input}"
            );
        }
    }

    #[test]
    fn fuzzy_year_only_in_parens() {
        let p = parse_filename_movie("Movie Name (1999)");
        assert_eq!(p.title, "Movie Name");
        assert_eq!(p.year, Some(1999));
    }

    // ---- parse_season_folder ------------------------------------------------

    #[test]
    fn season_folder_strict() {
        assert_eq!(
            parse_season_folder("Season 01"),
            Some(SeasonFolderInfo {
                number: 1,
                show: None
            })
        );
        assert_eq!(
            parse_season_folder("Season 1"),
            Some(SeasonFolderInfo {
                number: 1,
                show: None
            })
        );
        assert_eq!(
            parse_season_folder("season 12"),
            Some(SeasonFolderInfo {
                number: 12,
                show: None
            })
        );
    }

    #[test]
    fn season_folder_with_parens() {
        assert_eq!(
            parse_season_folder("Season 01 (1080p)"),
            Some(SeasonFolderInfo {
                number: 1,
                show: None
            })
        );
    }

    #[test]
    fn season_folder_specials() {
        assert_eq!(
            parse_season_folder("Specials"),
            Some(SeasonFolderInfo {
                number: 0,
                show: None
            })
        );
    }

    #[test]
    fn season_folder_combined_torrent() {
        let info = parse_season_folder(
            "Game of Thrones Season 1 (1080p BluRay x265 HEVC 10bit AAC 5 1 Tigole)",
        )
        .expect("parses");
        assert_eq!(info.number, 1);
        assert_eq!(info.show.as_deref(), Some("Game of Thrones"));
    }

    #[test]
    fn season_folder_combined_simple() {
        let info = parse_season_folder("Breaking Bad Season 02").expect("parses");
        assert_eq!(info.number, 2);
        assert_eq!(info.show.as_deref(), Some("Breaking Bad"));
    }

    #[test]
    fn season_folder_short_form() {
        assert_eq!(
            parse_season_folder("S01"),
            Some(SeasonFolderInfo {
                number: 1,
                show: None
            })
        );
    }

    #[test]
    fn season_folder_rejects_unknown() {
        assert_eq!(parse_season_folder("Random Folder"), None);
        assert_eq!(parse_season_folder("Movies"), None);
        assert_eq!(parse_season_folder("Season (8)"), None);
        assert_eq!(parse_season_folder("Season 1 Episode 2"), None);
        assert_eq!(parse_season_folder("S01E02"), None);
    }

    #[test]
    fn season_folder_other_forms() {
        for (name, number, show) in [
            ("Season1", 1, None),
            ("Staffel 2", 2, None),
            ("Temporada 03", 3, None),
            ("Saison 4 (2016)", 4, None),
            ("3. Staffel", 3, None),
            ("Season 2009", 2009, None),
            (
                "The Wonder Years Season 01 1080p",
                1,
                Some("The Wonder Years"),
            ),
            (
                "The.Wonder.Years.S04.1080p.PDTV.x264-JCH",
                4,
                Some("The Wonder Years"),
            ),
            (
                "The Wonder Years [S04][1080p PDTV x264-JCH]",
                4,
                Some("The Wonder Years"),
            ),
        ] {
            let info = parse_season_folder(name).expect(name);
            assert_eq!(
                (info.number, info.show.as_deref()),
                (number, show),
                "{name}"
            );
        }
    }

    // ---- classify_movie -----------------------------------------------------

    #[test]
    fn classify_in_folder() {
        let p = PathBuf::from("/lib/The Matrix (1999)/The Matrix (1999).mkv");
        let c = classify_movie(&p, &roots()).expect("classifies");
        assert!(matches!(c.identity, MovieIdentity::InFolder(_)));
        assert_eq!(c.parsed.title, "The Matrix");
        assert_eq!(c.parsed.year, Some(1999));
    }

    #[test]
    fn classify_bare_at_root() {
        let p = PathBuf::from("/lib/Inception (2010).mkv");
        let c = classify_movie(&p, &roots()).expect("classifies");
        assert!(matches!(c.identity, MovieIdentity::Bare(_)));
        assert_eq!(c.parsed.title, "Inception");
        assert_eq!(c.parsed.year, Some(2010));
    }

    #[test]
    fn classify_torrent_multidash_at_root() {
        let p = PathBuf::from("/lib/Mission - Impossible - Ghost Protocol - 1080p AV1 BluRay.mp4");
        let c = classify_movie(&p, &roots()).expect("classifies");
        assert!(matches!(c.identity, MovieIdentity::Bare(_)));
        assert_eq!(c.parsed.title, "Mission - Impossible - Ghost Protocol");
        assert_eq!(c.parsed.year, None);
    }

    #[test]
    fn classify_multipart_shares_folder_identity() {
        let cd1 = PathBuf::from("/lib/Blade Runner (1982)/Blade Runner (1982) - cd1.mkv");
        let cd2 = PathBuf::from("/lib/Blade Runner (1982)/Blade Runner (1982) - cd2.mkv");
        let c1 = classify_movie(&cd1, &roots()).expect("cd1");
        let c2 = classify_movie(&cd2, &roots()).expect("cd2");
        assert_eq!(c1.identity, c2.identity, "same folder = same identity");
        assert_eq!(c1.parsed.part, Some(1));
        assert_eq!(c2.parsed.part, Some(2));
    }

    #[test]
    fn classify_folder_vs_bare_different_identity() {
        let in_folder = PathBuf::from("/lib/Inception (2010)/Inception.mkv");
        let bare = PathBuf::from("/lib/Inception.mkv");
        let c1 = classify_movie(&in_folder, &roots()).expect("folder");
        let c2 = classify_movie(&bare, &roots()).expect("bare");
        assert_ne!(
            c1.identity, c2.identity,
            "folder candidate and bare candidate must NOT merge at scan time"
        );
    }

    // ---- classify_episode ---------------------------------------------------

    #[test]
    fn classify_plex_strict() {
        let p = PathBuf::from(
            "/lib/Breaking Bad (2008)/Season 01/Breaking Bad - s01e02 - Cat in the Bag.mkv",
        );
        let c = classify_episode(&p, &roots()).expect("classifies");
        assert_eq!(c.parsed.show, "Breaking Bad");
        assert_eq!(c.parsed.show_year, Some(2008));
        assert_eq!(c.parsed.season, 1);
        assert_eq!(c.parsed.episode, 2);
        assert_eq!(c.parsed.title.as_deref(), Some("Cat in the Bag"));
        assert_eq!(
            c.season_folder,
            PathBuf::from("/lib/Breaking Bad (2008)/Season 01")
        );
    }

    #[test]
    fn episode_title_stops_at_quality_tags() {
        for (name, title) in [
            ("Show - S01E01 - Pilot.mkv", Some("Pilot")),
            ("Show - S01E01 - 1080p.mkv", None),
            ("Show - S01E01 - Pilot - 1080p AV1.mkv", Some("Pilot")),
            ("Show - S01E05 - 1984 - 720p.mkv", Some("1984")),
            (
                "Show - S01E02 - Mission - Impossible - 2160p HEVC HDR.mkv",
                Some("Mission - Impossible"),
            ),
            ("Show - S01E03 - Mr. Robot.mkv", Some("Mr. Robot")),
            ("Show - S01E04 - Pilot [1080p].mkv", Some("Pilot")),
            ("Show.S01E01.1080p.WEB-DL.x264.mkv", None),
            ("Show.S01E01.Pilot.1080p.WEB-DL.x264-GRP.mkv", Some("Pilot")),
            ("Show S01E01.mkv", None),
            ("Show - S01E01-E02 - 1080p.mkv", None),
        ] {
            let parsed = parse_episode_filename(name).expect(name);
            assert_eq!(parsed.title.as_deref(), title, "{name}");
            assert_eq!(parsed.season, 1, "{name}");
        }
        let range = parse_episode_filename("Show - S01E01-E02 - 1080p.mkv").expect("range");
        assert_eq!((range.episode, range.episode_end), (1, Some(2)));
    }

    #[test]
    fn episode_marker_forms() {
        for (name, season, episode, end) in [
            ("Show s01x01.mkv", 1, 1, None),
            ("Show 1x02 - Title.mkv", 1, 2, None),
            ("Show S01xE02 Title.mkv", 1, 2, None),
            ("Show.S01.E02.mkv", 1, 2, None),
            ("Show S1 E9 - Title.mkv", 1, 9, None),
            ("Show Season 3 Episode 9 - Title.mkv", 3, 9, None),
            ("Show - S04E011.mkv", 4, 11, None),
            ("Show S2017E368.mkv", 2017, 368, None),
            ("Show S21E1001.mkv", 21, 1001, None),
            ("Show - S01E23-E24-E26 - Title.mkv", 1, 23, Some(26)),
            ("Show - S01E01E02.mkv", 1, 1, Some(2)),
            ("Show - S01E01-02.mkv", 1, 1, Some(2)),
            ("Show - 02x03x04 - Title.mkv", 2, 3, Some(4)),
            ("Show.S01E01-1080p.mkv", 1, 1, None),
        ] {
            let parsed = parse_episode_filename(name).expect(name);
            assert_eq!(
                (parsed.season, parsed.episode, parsed.episode_end),
                (season, episode, end),
                "{name}"
            );
            assert_eq!(parsed.show_prefix.as_deref(), Some("Show"), "{name}");
        }
        for name in ["Movie 1920x1080.mkv", "Movie.x264.mkv", "Seasons.mkv"] {
            assert!(parse_episode_filename(name).is_none(), "{name}");
        }
    }

    #[test]
    fn bare_episode_in_season_folder() {
        for (name, episode, title) in [
            ("Episode 16.avi", 16, None),
            ("Episode 16 - Some Title.avi", 16, Some("Some Title")),
            ("02 - Title.avi", 2, Some("Title")),
            ("2. Infestation.avi", 2, Some("Infestation")),
            ("7 - 12 Angry Men.avi", 7, Some("12 Angry Men")),
            ("02.avi", 2, None),
        ] {
            let p = PathBuf::from(format!("/lib/Show/Season 2/{name}"));
            let c = classify_episode(&p, &roots()).expect(name);
            assert_eq!(c.parsed.show, "Show", "{name}");
            assert_eq!((c.parsed.season, c.parsed.episode), (2, episode), "{name}");
            assert_eq!(c.parsed.title.as_deref(), title, "{name}");
        }
        // Outside a season folder, and without a separator, a number is a title.
        assert!(classify_episode(&PathBuf::from("/lib/Show/02 - Title.avi"), &roots()).is_none());
        assert!(
            classify_episode(
                &PathBuf::from("/lib/Show/Season 2/12 Angry Men.avi"),
                &roots()
            )
            .is_none()
        );
    }

    #[test]
    fn show_folder_without_season_folder() {
        let p = PathBuf::from("/lib/Watchmen (2019)/Watchmen 1x03 [WEBDL-720p] - Title.mkv");
        let c = classify_episode(&p, &roots()).expect("classifies");
        assert_eq!(
            (c.parsed.show.as_str(), c.parsed.show_year),
            ("Watchmen", Some(2019))
        );
        // One episode's release folder doesn't name the show.
        let p = PathBuf::from(
            "/lib/The.Series.Name.S01E04.WEBRip.x264-GRP/the.series.name.s01e04.webrip.x264-GRP.mkv",
        );
        let c = classify_episode(&p, &roots()).expect("classifies");
        assert_eq!(c.parsed.show, "the series name");
    }

    #[test]
    fn scene_episode_name_classifies() {
        let p = PathBuf::from("/lib/The.Office.S02E03.720p.HDTV.x264.mkv");
        let c = classify_episode(&p, &roots()).expect("classifies");
        assert_eq!(c.parsed.show, "The Office");
        assert_eq!((c.parsed.season, c.parsed.episode), (2, 3));
        assert_eq!(c.parsed.title, None);
    }

    #[test]
    fn classify_multi_episode() {
        let p = PathBuf::from("/lib/Severance/Season 01/Severance - s01e01-e02 - Good News.mkv");
        let c = classify_episode(&p, &roots()).expect("classifies");
        assert_eq!(c.parsed.episode, 1);
        assert_eq!(c.parsed.episode_end, Some(2));
    }

    #[test]
    fn classify_torrent_flat() {
        let p = PathBuf::from(
            "/lib/Game of Thrones Season 1 (1080p BluRay x265 HEVC 10bit AAC 5 1 Tigole)/Game of Thrones - S01E01 - Winter Is Coming.mkv",
        );
        let c = classify_episode(&p, &roots()).expect("classifies");
        assert_eq!(c.parsed.show, "Game of Thrones");
        assert_eq!(c.parsed.season, 1);
        assert_eq!(c.parsed.episode, 1);
    }

    #[test]
    fn classify_multi_versions_same_season_distinct_folders() {
        let p1080 = PathBuf::from(
            "/lib/Game of Thrones (2011)/Season 01 (1080p)/Game of Thrones - s01e01 - X.mkv",
        );
        let p4k = PathBuf::from(
            "/lib/Game of Thrones (2011)/Season 01 (4K)/Game of Thrones - s01e01 - X.mkv",
        );
        let c1 = classify_episode(&p1080, &roots()).expect("1080");
        let c2 = classify_episode(&p4k, &roots()).expect("4k");
        // Same canonical episode by metadata...
        assert_eq!(c1.parsed.show, c2.parsed.show);
        assert_eq!(c1.parsed.season, c2.parsed.season);
        assert_eq!(c1.parsed.episode, c2.parsed.episode);
        // ...from distinct season folders, each with its own artwork.
        assert_ne!(c1.season_folder, c2.season_folder);
    }

    #[test]
    fn classify_specials() {
        let p = PathBuf::from("/lib/Show (2020)/Specials/Show - s00e01 - Behind the Scenes.mkv");
        let c = classify_episode(&p, &roots()).expect("classifies");
        assert_eq!(c.parsed.season, 0);
        assert_eq!(c.parsed.episode, 1);
    }

    #[test]
    fn classify_rejects_season_mismatch() {
        let p = PathBuf::from("/lib/Show/Season 02/Show - s01e01 - X.mkv");
        assert!(classify_episode(&p, &roots()).is_none());
    }
}
