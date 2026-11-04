// SPDX-License-Identifier: AGPL-3.0-only
//
// Post-processing and manifests for optimized versions: one MP4 holding
// video and every audio track, where each track has its own fragments.
//
//     ftyp moov | moof(video) mdat | moof(audio 1) mdat | moof(video) mdat | ...
//
// The file plays standalone, a truncated copy plays up to where its bytes end,
// and HLS can address each track by byte range, so audio stays switchable with
// no second copy. ffmpeg writes the layout (`+separate_moof`); its global sidx
// comes out wrong for it ("Non-consecutive fragments"), so the index here comes
// from walking the fragments instead.

use std::{
    collections::HashMap,
    fmt::Write as _,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hls::{HlsVariant, render_master};

/// ffmpeg `-movflags` producing the layout this module understands.
pub const PACKAGE_MOVFLAGS: &str = "+cmaf+frag_keyframe+separate_moof+delay_moov+default_base_moof";

/// Bounds a metadata box read into memory. Real `moov` and `moof` boxes are
/// far smaller; a larger one means a corrupt size field.
const MAX_METADATA_BOX: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("malformed package: {0}")]
    Malformed(&'static str),
    #[error("no track {0} in this package")]
    UnknownTrack(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Other,
}

/// One track's `moof` + `mdat` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fragment {
    pub offset: u64,
    pub size: u64,
    /// In the track's timescale.
    pub duration: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackIndex {
    pub track_id: u32,
    pub kind: TrackKind,
    pub timescale: u32,
    pub fragments: Vec<Fragment>,
}

impl TrackIndex {
    fn seconds(&self, ticks: u64) -> f64 {
        ticks as f64 / f64::from(self.timescale.max(1))
    }

    /// Total bytes of this track's fragments.
    pub fn size_bytes(&self) -> u64 {
        self.fragments.iter().map(|fragment| fragment.size).sum()
    }
}

/// Where everything is in a finished package; manifests come from this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageIndex {
    /// The file's first `head_size` bytes are its `ftyp` and `moov`.
    pub head_size: u64,
    pub duration_ms: u64,
    pub size_bytes: u64,
    /// In track id order, which is also the order probes list streams.
    pub tracks: Vec<TrackIndex>,
}

/// One audio entry of a package's master playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRendition {
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
    pub uri: String,
}

/// A child box's kind and its bytes, header included.
type ChildBox<'a> = ([u8; 4], &'a [u8]);

struct BoxHeader {
    kind: [u8; 4],
    offset: u64,
    size: u64,
    header: u64,
}

/// Top-level boxes of `file`, read without touching their bodies.
fn top_level_boxes(file: &mut File, length: u64) -> Result<Vec<BoxHeader>, PackageError> {
    let mut boxes = Vec::new();
    let mut offset = 0_u64;
    while offset + 8 <= length {
        file.seek(SeekFrom::Start(offset))?;
        let mut head = [0_u8; 16];
        let wanted = 16.min(length - offset) as usize;
        file.read_exact(&mut head[..wanted])?;
        let parsed = parse_header(&head[..wanted], offset, length - offset)?;
        offset = offset
            .checked_add(parsed.size)
            .ok_or(PackageError::Malformed("box size overflows"))?;
        boxes.push(parsed);
    }
    Ok(boxes)
}

fn parse_header(bytes: &[u8], offset: u64, available: u64) -> Result<BoxHeader, PackageError> {
    let size32 = u32_at(bytes, 0).map_err(|_| PackageError::Malformed("truncated box header"))?;
    let kind: [u8; 4] = bytes
        .get(4..8)
        .and_then(|kind| kind.try_into().ok())
        .ok_or(PackageError::Malformed("truncated box header"))?;
    let (size, header) = match size32 {
        0 => (available, 8),
        1 => {
            let large = bytes
                .get(8..16)
                .and_then(|size| size.try_into().ok())
                .map(u64::from_be_bytes)
                .ok_or(PackageError::Malformed("truncated box header"))?;
            (large, 16)
        }
        size => (u64::from(size), 8),
    };
    if size < header {
        return Err(PackageError::Malformed("box smaller than its header"));
    }
    Ok(BoxHeader {
        kind,
        offset,
        size,
        header,
    })
}

/// The boxes directly inside `bytes`, as `(kind, whole box)`.
fn children(bytes: &[u8]) -> Result<Vec<ChildBox<'_>>, PackageError> {
    let mut out = Vec::new();
    let mut offset = 0_usize;
    while offset + 8 <= bytes.len() {
        let rest = &bytes[offset..];
        let header = parse_header(rest, 0, rest.len() as u64)?;
        let size = usize::try_from(header.size)
            .ok()
            .filter(|size| *size <= rest.len())
            .ok_or(PackageError::Malformed("box overruns its parent"))?;
        out.push((header.kind, &rest[..size]));
        offset += size;
    }
    Ok(out)
}

/// The body of the first child of `whole_box` named `kind`.
fn child<'a>(whole_box: &'a [u8], kind: &[u8; 4]) -> Result<Option<&'a [u8]>, PackageError> {
    Ok(children(body(whole_box)?)?
        .into_iter()
        .find(|(found, _)| found == kind)
        .map(|(_, found)| found))
}

fn body(whole_box: &[u8]) -> Result<&[u8], PackageError> {
    let header = parse_header(whole_box, 0, whole_box.len() as u64)?;
    Ok(&whole_box[header.header as usize..])
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, PackageError> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or(PackageError::Malformed("box too short"))
}

fn make_box(kind: &[u8; 4], payload: &[u8]) -> Result<Vec<u8>, PackageError> {
    let size = u32::try_from(payload.len() + 8)
        .map_err(|_| PackageError::Malformed("metadata box too large"))?;
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    Ok(out)
}

/// What `moov` says about one track.
struct TrackInfo {
    kind: TrackKind,
    timescale: u32,
    default_sample_duration: u32,
}

fn track_id_of(trak: &[u8]) -> Result<u32, PackageError> {
    let tkhd = child(trak, b"tkhd")?.ok_or(PackageError::Malformed("trak without tkhd"))?;
    let tkhd = body(tkhd)?;
    u32_at(tkhd, if tkhd.first() == Some(&1) { 20 } else { 12 })
}

fn parse_tracks(moov: &[u8]) -> Result<HashMap<u32, TrackInfo>, PackageError> {
    let mut defaults = HashMap::new();
    if let Some(mvex) = child(moov, b"mvex")? {
        for (kind, trex) in children(body(mvex)?)? {
            if &kind == b"trex" {
                let trex = body(trex)?;
                defaults.insert(u32_at(trex, 4)?, u32_at(trex, 12)?);
            }
        }
    }
    let mut tracks = HashMap::new();
    for (kind, trak) in children(body(moov)?)? {
        if &kind != b"trak" {
            continue;
        }
        let id = track_id_of(trak)?;
        let mdia = child(trak, b"mdia")?.ok_or(PackageError::Malformed("trak without mdia"))?;
        let mdhd = body(child(mdia, b"mdhd")?.ok_or(PackageError::Malformed("no mdhd"))?)?;
        let timescale = u32_at(mdhd, if mdhd.first() == Some(&1) { 20 } else { 12 })?;
        let hdlr = body(child(mdia, b"hdlr")?.ok_or(PackageError::Malformed("no hdlr"))?)?;
        let kind = match hdlr.get(8..12) {
            Some(b"vide") => TrackKind::Video,
            Some(b"soun") => TrackKind::Audio,
            _ => TrackKind::Other,
        };
        tracks.insert(
            id,
            TrackInfo {
                kind,
                timescale,
                default_sample_duration: defaults.get(&id).copied().unwrap_or(0),
            },
        );
    }
    Ok(tracks)
}

/// The track a `moof` belongs to and how long it plays, in that track's
/// timescale. A `moof` carrying several tracks isn't this layout.
fn parse_moof(moof: &[u8], tracks: &HashMap<u32, TrackInfo>) -> Result<(u32, u64), PackageError> {
    let trafs = children(body(moof)?)?
        .into_iter()
        .filter(|(kind, _)| kind == b"traf")
        .map(|(_, traf)| traf)
        .collect::<Vec<_>>();
    let [traf] = trafs[..] else {
        return Err(PackageError::Malformed(
            "fragment doesn't hold exactly one track",
        ));
    };
    let tfhd = body(child(traf, b"tfhd")?.ok_or(PackageError::Malformed("traf without tfhd"))?)?;
    let flags = u32_at(tfhd, 0)? & 0x00ff_ffff;
    let track = u32_at(tfhd, 4)?;
    let mut position = 8;
    if flags & 0x01 != 0 {
        position += 8; // base-data-offset
    }
    if flags & 0x02 != 0 {
        position += 4; // sample-description-index
    }
    let default_duration = if flags & 0x08 != 0 {
        u32_at(tfhd, position)?
    } else {
        tracks
            .get(&track)
            .ok_or(PackageError::UnknownTrack(track))?
            .default_sample_duration
    };

    let mut duration = 0_u64;
    for (kind, trun) in children(body(traf)?)? {
        if &kind != b"trun" {
            continue;
        }
        let trun = body(trun)?;
        let flags = u32_at(trun, 0)? & 0x00ff_ffff;
        let samples = u32_at(trun, 4)?;
        if flags & 0x100 == 0 {
            duration += u64::from(samples) * u64::from(default_duration);
            continue;
        }
        let mut position = 8;
        if flags & 0x01 != 0 {
            position += 4; // data-offset
        }
        if flags & 0x04 != 0 {
            position += 4; // first-sample-flags
        }
        let per_sample = 4
            + usize::from(flags & 0x200 != 0) * 4
            + usize::from(flags & 0x400 != 0) * 4
            + usize::from(flags & 0x800 != 0) * 4;
        for sample in 0..samples as usize {
            duration += u64::from(u32_at(trun, position + sample * per_sample)?);
        }
    }
    Ok((track, duration))
}

fn read_box(file: &mut File, header: &BoxHeader) -> Result<Vec<u8>, PackageError> {
    if header.size > MAX_METADATA_BOX {
        return Err(PackageError::Malformed("metadata box too large"));
    }
    file.seek(SeekFrom::Start(header.offset))?;
    let mut bytes = vec![0_u8; header.size as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// `moov` with a `mehd` holding the total duration added to its `mvex`.
/// Firefox reports only the first fragment's length without it.
fn with_total_duration(moov: &[u8], duration_ms: u64) -> Result<Vec<u8>, PackageError> {
    let mvhd = body(child(moov, b"mvhd")?.ok_or(PackageError::Malformed("moov without mvhd"))?)?;
    let timescale = u32_at(mvhd, if mvhd.first() == Some(&1) { 20 } else { 12 })?;
    let ticks = duration_ms.saturating_mul(u64::from(timescale)) / 1000;
    let mut mehd = Vec::with_capacity(12);
    match u32::try_from(ticks) {
        Ok(ticks) => {
            mehd.extend_from_slice(&0_u32.to_be_bytes());
            mehd.extend_from_slice(&ticks.to_be_bytes());
        }
        Err(_) => {
            mehd.extend_from_slice(&0x0100_0000_u32.to_be_bytes());
            mehd.extend_from_slice(&ticks.to_be_bytes());
        }
    }
    let mehd = make_box(b"mehd", &mehd)?;

    let mut payload = Vec::with_capacity(moov.len() + mehd.len());
    let mut replaced = false;
    for (kind, found) in children(body(moov)?)? {
        if &kind == b"mvex" {
            let mut mvex = mehd.clone();
            for (inner, inner_box) in children(body(found)?)? {
                if &inner != b"mehd" {
                    mvex.extend_from_slice(inner_box);
                }
            }
            payload.extend_from_slice(&make_box(b"mvex", &mvex)?);
            replaced = true;
        } else {
            payload.extend_from_slice(found);
        }
    }
    if !replaced {
        return Err(PackageError::Malformed("moov without mvex"));
    }
    make_box(b"moov", &payload)
}

/// Finish ffmpeg's output at `input` into a package at `output`: index every
/// fragment, add the total duration, and drop the trailing `mfra`, whose
/// absolute offsets the longer `moov` would invalidate. Fragments use
/// default-base-is-moof, so their own offsets survive the move.
///
/// Blocking; run it off the async executor. `output` is overwritten. Input that
/// isn't this layout is [`PackageError::Malformed`].
pub fn finalize(input: &Path, output: &Path) -> Result<PackageIndex, PackageError> {
    let mut source = File::open(input)?;
    let length = source.metadata()?.len();
    let boxes = top_level_boxes(&mut source, length)?;
    let find = |kind: &[u8; 4]| boxes.iter().find(|found| &found.kind == kind);
    let ftyp = find(b"ftyp").ok_or(PackageError::Malformed("no ftyp"))?;
    let moov = find(b"moov").ok_or(PackageError::Malformed("no moov"))?;
    let ftyp = read_box(&mut source, ftyp)?;
    let moov = read_box(&mut source, moov)?;
    let infos = parse_tracks(&moov)?;

    let mut fragments: HashMap<u32, Vec<Fragment>> = HashMap::new();
    let mut first_moof = None;
    let mut end_of_media = 0;
    for (index, found) in boxes.iter().enumerate() {
        if &found.kind != b"moof" {
            continue;
        }
        let mdat = boxes
            .get(index + 1)
            .filter(|next| &next.kind == b"mdat")
            .ok_or(PackageError::Malformed("moof without mdat"))?;
        let (track, duration) = parse_moof(&read_box(&mut source, found)?, &infos)?;
        fragments.entry(track).or_default().push(Fragment {
            offset: found.offset,
            size: found.size + mdat.size,
            duration,
        });
        first_moof.get_or_insert(found.offset);
        end_of_media = mdat.offset + mdat.size;
    }
    let first_moof = first_moof.ok_or(PackageError::Malformed("no fragments"))?;

    let duration_ms = fragments
        .iter()
        .map(|(track, fragments)| {
            let timescale = infos.get(track).map_or(1, |info| info.timescale.max(1));
            let ticks = fragments.iter().map(|f| f.duration).sum::<u64>();
            ticks.saturating_mul(1000) / u64::from(timescale)
        })
        .max()
        .unwrap_or(0);

    let moov = with_total_duration(&moov, duration_ms)?;
    let head_size = (ftyp.len() + moov.len()) as u64;
    let media_size = end_of_media - first_moof;

    let mut sink = File::create(output)?;
    sink.write_all(&ftyp)?;
    sink.write_all(&moov)?;
    source.seek(SeekFrom::Start(first_moof))?;
    let copied = io::copy(&mut (&mut source).take(media_size), &mut sink)?;
    if copied != media_size {
        return Err(PackageError::Malformed("input shrank while packaging"));
    }
    sink.sync_all()?;

    let mut tracks = infos
        .into_iter()
        .map(|(track_id, info)| {
            let mut fragments = fragments.remove(&track_id).unwrap_or_default();
            for fragment in &mut fragments {
                fragment.offset = fragment.offset - first_moof + head_size;
            }
            TrackIndex {
                track_id,
                kind: info.kind,
                timescale: info.timescale,
                fragments,
            }
        })
        .collect::<Vec<_>>();
    tracks.sort_by_key(|track| track.track_id);
    Ok(PackageIndex {
        head_size,
        duration_ms,
        size_bytes: head_size + media_size,
        tracks,
    })
}

/// An init segment announcing only `track_id`, from a package's first
/// [`PackageIndex::head_size`] bytes. An MSE SourceBuffer rejects an init
/// segment announcing tracks it wasn't created for.
pub fn single_track_init(head: &[u8], track_id: u32) -> Result<Vec<u8>, PackageError> {
    let mut out = Vec::with_capacity(head.len());
    let mut moov_payload = None;
    for (kind, found) in children(head)? {
        match &kind {
            b"ftyp" => out.extend_from_slice(found),
            b"moov" => moov_payload = Some(found),
            _ => {}
        }
    }
    let moov = moov_payload.ok_or(PackageError::Malformed("no moov"))?;

    let mut payload = Vec::with_capacity(moov.len());
    let mut found_track = false;
    for (kind, found) in children(body(moov)?)? {
        match &kind {
            b"trak" if track_id_of(found)? == track_id => {
                payload.extend_from_slice(found);
                found_track = true;
            }
            b"mvex" => {
                let mut mvex = Vec::new();
                for (inner, inner_box) in children(body(found)?)? {
                    let keep = &inner != b"trex" || u32_at(body(inner_box)?, 4)? == track_id;
                    if keep {
                        mvex.extend_from_slice(inner_box);
                    }
                }
                payload.extend_from_slice(&make_box(b"mvex", &mvex)?);
            }
            b"mvhd" => payload.extend_from_slice(found),
            _ => {}
        }
    }
    if !found_track {
        return Err(PackageError::UnknownTrack(track_id));
    }
    out.extend_from_slice(&make_box(b"moov", &payload)?);
    Ok(out)
}

/// A media playlist of `track`'s byte ranges inside `file_uri`.
pub fn media_playlist(track: &TrackIndex, init_uri: &str, file_uri: &str) -> String {
    let longest = track
        .fragments
        .iter()
        .map(|fragment| track.seconds(fragment.duration))
        .fold(0.0, f64::max);
    let mut out = String::with_capacity(128 + track.fragments.len() * 64);
    let _ = write!(
        out,
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MAP:URI=\"{init_uri}\"\n",
        longest.ceil() as u64
    );
    for fragment in &track.fragments {
        let _ = write!(
            out,
            "#EXTINF:{:.6},\n#EXT-X-BYTERANGE:{}@{}\n{file_uri}\n",
            track.seconds(fragment.duration),
            fragment.size,
            fragment.offset
        );
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out
}

/// A master playlist offering `video` with `audio` as switchable renditions.
/// `video.codecs` must list the audio codecs too. Without audio this is
/// `render_master` of `video` alone.
pub fn master_playlist(video: &HlsVariant, audio: &[AudioRendition]) -> String {
    let rendered = render_master(std::slice::from_ref(video));
    if audio.is_empty() {
        return rendered;
    }
    let mut media = String::new();
    for rendition in audio {
        let flag = if rendition.default { "YES" } else { "NO" };
        let _ = write!(
            media,
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"{}\",",
            attribute_text(&rendition.name)
        );
        if let Some(language) = &rendition.language {
            let _ = write!(media, "LANGUAGE=\"{}\",", attribute_text(language));
        }
        let _ = writeln!(
            media,
            "DEFAULT={flag},AUTOSELECT={flag},URI=\"{}\"",
            rendition.uri
        );
    }
    rendered.replacen(
        "#EXT-X-STREAM-INF:",
        &format!("{media}#EXT-X-STREAM-INF:AUDIO=\"audio\","),
        1,
    )
}

/// HLS quoted strings can't escape; track titles come from files.
fn attribute_text(text: &str) -> String {
    text.chars()
        .filter(|c| *c != '"' && !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use super::*;

    fn work_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("noflex-package-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A 12-second clip with a 440 Hz and an 880 Hz audio track, written with
    /// `movflags`. `false` when ffmpeg isn't available.
    fn clip(path: &Path, movflags: &str) -> bool {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "testsrc2=s=160x90:r=24:d=12"])
            .args(["-f", "lavfi", "-i", "sine=f=440:d=12"])
            .args(["-f", "lavfi", "-i", "sine=f=880:d=12"])
            .args(["-map", "0", "-map", "1", "-map", "2"])
            .args(["-c:v", "libx264", "-preset", "ultrafast"])
            .args(["-pix_fmt", "yuv420p"])
            .args(["-force_key_frames", "expr:gte(t,n_forced*4)"])
            .args(["-c:a", "aac", "-ac", "2"])
            .args(["-movflags", movflags, "-f", "mp4"])
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    /// The frequency of mono PCM decoded from `playlist`, and its length.
    fn decoded_tone(dir: &Path, playlist: &str) -> (f64, f64) {
        let pcm = dir.join(format!("{playlist}.pcm"));
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-protocol_whitelist", "file,crypto,data"])
            .args(["-allowed_extensions", "ALL", "-i"])
            .arg(dir.join(playlist))
            .args(["-ac", "1", "-ar", "8000", "-f", "s16le"])
            .arg(&pcm)
            .status()
            .unwrap();
        assert!(status.success(), "decoding {playlist}");
        let samples = std::fs::read(&pcm)
            .unwrap()
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        let crossings = samples
            .windows(2)
            .filter(|pair| (pair[0] < 0) != (pair[1] < 0))
            .count();
        let seconds = samples.len() as f64 / 8000.0;
        (crossings as f64 / 2.0 / seconds, seconds)
    }

    #[test]
    fn finalized_package_serves_each_track_on_its_own() {
        let dir = work_dir();
        let raw = dir.join("raw.mp4");
        if !clip(&raw, PACKAGE_MOVFLAGS) {
            eprintln!("ffmpeg unavailable; skipping");
            return;
        }
        let packaged = dir.join("package.mp4");
        let index = finalize(&raw, &packaged).unwrap();
        let bytes = std::fs::read(&packaged).unwrap();

        assert_eq!(index.size_bytes, bytes.len() as u64);
        assert!((11_900..=12_200).contains(&index.duration_ms), "{index:?}");
        let kinds = index.tracks.iter().map(|t| t.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [TrackKind::Video, TrackKind::Audio, TrackKind::Audio]
        );
        for track in &index.tracks {
            assert_eq!(track.fragments.len(), 3, "{track:?}");
            for fragment in &track.fragments {
                let at = fragment.offset as usize;
                assert_eq!(
                    &bytes[at + 4..at + 8],
                    b"moof",
                    "offsets point at fragments"
                );
            }
        }
        let head = &bytes[..index.head_size as usize];
        assert!(head.windows(4).any(|window| window == b"mehd"));
        assert!(
            !bytes[bytes.len().saturating_sub(4096)..]
                .windows(4)
                .any(|window| window == b"mfra"),
            "mfra's absolute offsets would be wrong"
        );

        // Standalone, the file still presents all of its streams.
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_type"])
            .args(["-of", "csv=p=0"])
            .arg(&packaged)
            .output()
            .unwrap();
        let streams = String::from_utf8_lossy(&probe.stdout);
        assert_eq!(streams.split_whitespace().count(), 3, "{streams}");

        // Over HLS, each audio playlist reaches its own track's bytes.
        for track in &index.tracks {
            let init = single_track_init(head, track.track_id).unwrap();
            assert_eq!(
                init.windows(4).filter(|window| *window == b"trak").count(),
                1
            );
            let init_name = format!("init_{}.mp4", track.track_id);
            std::fs::write(dir.join(&init_name), init).unwrap();
            std::fs::write(
                dir.join(format!("track_{}.m3u8", track.track_id)),
                media_playlist(track, &init_name, "package.mp4"),
            )
            .unwrap();
        }
        let (first_hz, first_seconds) = decoded_tone(&dir, "track_2.m3u8");
        let (second_hz, _) = decoded_tone(&dir, "track_3.m3u8");
        assert!((first_hz - 440.0).abs() < 5.0, "{first_hz}");
        assert!((second_hz - 880.0).abs() < 5.0, "{second_hz}");
        assert!((first_seconds - 12.0).abs() < 0.5, "{first_seconds}");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_truncated_package_still_decodes_what_it_holds() {
        let dir = work_dir();
        let raw = dir.join("raw.mp4");
        if !clip(&raw, PACKAGE_MOVFLAGS) {
            eprintln!("ffmpeg unavailable; skipping");
            return;
        }
        let packaged = dir.join("package.mp4");
        let index = finalize(&raw, &packaged).unwrap();
        let bytes = std::fs::read(&packaged).unwrap();
        let cut = dir.join("cut.mp4");
        // Past the first of three video fragments.
        let keep = index.tracks[0].fragments[1].offset as usize;
        std::fs::write(&cut, &bytes[..keep]).unwrap();

        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&cut)
            .args(["-map", "0:v", "-f", "null", "-"])
            .status()
            .unwrap();
        assert!(status.success());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn muxed_fragments_are_refused() {
        let dir = work_dir();
        let raw = dir.join("raw.mp4");
        if !clip(&raw, "+frag_keyframe+delay_moov+default_base_moof") {
            eprintln!("ffmpeg unavailable; skipping");
            return;
        }
        let error = finalize(&raw, &dir.join("out.mp4")).unwrap_err();
        assert!(matches!(error, PackageError::Malformed(_)), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn master_playlist_groups_audio_under_the_video_variant() {
        let video = HlsVariant {
            bandwidth_bps: 4_000_000,
            resolution: Some((1280, 720)),
            codecs: "avc1.64001f,mp4a.40.2".to_string(),
            supplemental_codecs: None,
            video_range: None,
            path: "video.m3u8".to_string(),
        };
        let master = master_playlist(
            &video,
            &[
                AudioRendition {
                    name: "Eng\"lish".to_string(),
                    language: Some("eng".to_string()),
                    default: true,
                    uri: "audio_2.m3u8".to_string(),
                },
                AudioRendition {
                    name: "Japanese".to_string(),
                    language: None,
                    default: false,
                    uri: "audio_3.m3u8".to_string(),
                },
            ],
        );
        assert_eq!(
            master,
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"English\",LANGUAGE=\"eng\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio_2.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Japanese\",DEFAULT=NO,AUTOSELECT=NO,URI=\"audio_3.m3u8\"\n\
             #EXT-X-STREAM-INF:AUDIO=\"audio\",BANDWIDTH=4000000,RESOLUTION=1280x720,CODECS=\"avc1.64001f,mp4a.40.2\"\nvideo.m3u8\n"
        );
    }

    #[test]
    fn media_playlist_rounds_the_target_duration_up() {
        let track = TrackIndex {
            track_id: 2,
            kind: TrackKind::Audio,
            timescale: 1000,
            fragments: vec![
                Fragment {
                    offset: 100,
                    size: 50,
                    duration: 4086,
                },
                Fragment {
                    offset: 300,
                    size: 60,
                    duration: 3914,
                },
            ],
        };
        assert_eq!(
            media_playlist(&track, "init.mp4", "file.mp4"),
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:5\n#EXT-X-MAP:URI=\"init.mp4\"\n\
             #EXTINF:4.086000,\n#EXT-X-BYTERANGE:50@100\nfile.mp4\n\
             #EXTINF:3.914000,\n#EXT-X-BYTERANGE:60@300\nfile.mp4\n\
             #EXT-X-ENDLIST\n"
        );
    }
}
