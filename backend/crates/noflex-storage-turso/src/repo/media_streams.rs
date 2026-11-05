// SPDX-License-Identifier: AGPL-3.0-only
//
// `media_streams` and its per-kind tables: a file's probe, in columns.

use noflex_core::text::canonical_language;
use noflex_storage::{domain::SubtitleTrack, error::StorageResult};
use serde::Deserialize;
use turso::{Connection, Value};
use uuid::Uuid;

use crate::{convert::uuid_to_blob, error::map_turso};

/// The parts of a stored probe that become columns. Everything is optional, because
/// an unprobed file stores `{}` and that isn't an error.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Probe {
    video: Option<ProbedVideo>,
    audio: Vec<ProbedAudio>,
    subtitle: Vec<ProbedSubtitle>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProbedVideo {
    width: u32,
    height: u32,
    codec: Option<String>,
    transfer: Option<String>,
    dovi_profile: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProbedAudio {
    codec: Option<String>,
    language: Option<String>,
    channels: u32,
    channel_layout: Option<String>,
    audio_object_type: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProbedSubtitle {
    codec: Option<String>,
    language: Option<String>,
    forced: bool,
}

impl ProbedVideo {
    /// By either dimension, because a scope or 4:3 film is short or narrow for its class.
    fn resolution(&self) -> &'static str {
        let reaches = |width, height| self.width >= width || self.height >= height;
        if reaches(6000, 3000) {
            "8k"
        } else if reaches(3000, 1800) {
            "4k"
        } else if reaches(2300, 1300) {
            "1440p"
        } else if reaches(1700, 900) {
            "1080p"
        } else if reaches(1100, 650) {
            "720p"
        } else {
            "sd"
        }
    }

    /// The same order as `VideoRange::of`: Dolby Vision outranks its base layer.
    /// HDR10+ isn't probed, so it reads as `hdr10`.
    fn dynamic_range(&self) -> &'static str {
        match (&self.dovi_profile, self.transfer.as_deref()) {
            (Some(_), _) => "dolby_vision",
            (None, Some("pq")) => "hdr10",
            (None, Some("hlg")) => "hlg",
            _ => "sdr",
        }
    }
}

impl ProbedAudio {
    fn layout(&self) -> String {
        let object_based = matches!(self.audio_object_type.as_deref(), Some("joc" | "atmos"))
            || self.channel_layout.as_deref() == Some("object_based");
        match self.channels {
            _ if object_based => "atmos".to_string(),
            8.. => "7.1".to_string(),
            6 => "5.1".to_string(),
            2 => "stereo".to_string(),
            1 => "mono".to_string(),
            other => other.to_string(),
        }
    }
}

/// Undetermined isn't a language.
fn stored_language(language: Option<&str>) -> Value {
    match language.map(canonical_language) {
        Some(code) if !code.is_empty() && code != "und" => Value::Text(code),
        _ => Value::Null,
    }
}

fn optional_text(text: Option<&str>) -> Value {
    text.map_or(Value::Null, |text| Value::Text(text.to_string()))
}

/// Insert the base row of one stream and return its id for the kind's own row.
async fn insert_stream(
    conn: &Connection,
    file: &[u8],
    kind: &str,
    position: usize,
    codec: Option<&str>,
    language: Value,
) -> StorageResult<Value> {
    let id = Value::Blob(uuid_to_blob(Uuid::now_v7()));
    conn.execute(
        "INSERT INTO media_streams (id, media_file_id, kind, position, codec, language) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        vec![
            id.clone(),
            Value::Blob(file.to_vec()),
            Value::Text(kind.to_string()),
            Value::Integer(position as i64),
            optional_text(codec),
            language,
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(id)
}

async fn insert_subtitle(
    conn: &Connection,
    file: &[u8],
    position: usize,
    (codec, language, forced, external): (Option<&str>, Option<&str>, bool, bool),
) -> StorageResult<()> {
    let id = insert_stream(
        conn,
        file,
        "subtitle",
        position,
        codec,
        stored_language(language),
    )
    .await?;
    conn.execute(
        "INSERT INTO subtitle_streams (stream_id, external, forced) VALUES (?1, ?2, ?3)",
        vec![
            id,
            Value::Integer(external.into()),
            Value::Integer(forced.into()),
        ],
    )
    .await
    .map_err(map_turso)?;
    Ok(())
}

/// Replace `file`'s streams with what `probe` and `sidecars` say. Runs inside the
/// write of the file itself, so the columns can't drift from the probe. A probe we
/// can't read has no streams, rather than failing the scan.
pub(crate) async fn replace(
    conn: &Connection,
    file: &[u8],
    probe: &serde_json::Value,
    sidecars: &[SubtitleTrack],
) -> StorageResult<()> {
    conn.execute(
        "DELETE FROM media_streams WHERE media_file_id = ?1",
        vec![Value::Blob(file.to_vec())],
    )
    .await
    .map_err(map_turso)?;
    let probe = Probe::deserialize(probe).unwrap_or_default();
    let mut position = 0;
    if let Some(video) = probe.video.filter(|video| video.height > 0) {
        let id = insert_stream(
            conn,
            file,
            "video",
            position,
            video.codec.as_deref(),
            Value::Null,
        )
        .await?;
        conn.execute(
            "INSERT INTO video_streams (stream_id, width, height, resolution, dynamic_range) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            vec![
                id,
                Value::Integer(video.width.into()),
                Value::Integer(video.height.into()),
                Value::Text(video.resolution().to_string()),
                Value::Text(video.dynamic_range().to_string()),
            ],
        )
        .await
        .map_err(map_turso)?;
        position += 1;
    }
    for audio in &probe.audio {
        let language = stored_language(audio.language.as_deref());
        let id = insert_stream(
            conn,
            file,
            "audio",
            position,
            audio.codec.as_deref(),
            language,
        )
        .await?;
        conn.execute(
            "INSERT INTO audio_streams (stream_id, channels, layout) VALUES (?1, ?2, ?3)",
            vec![
                id,
                Value::Integer(audio.channels.into()),
                Value::Text(audio.layout()),
            ],
        )
        .await
        .map_err(map_turso)?;
        position += 1;
    }
    for subtitle in &probe.subtitle {
        let facts = (
            subtitle.codec.as_deref(),
            subtitle.language.as_deref(),
            subtitle.forced,
            false,
        );
        insert_subtitle(conn, file, position, facts).await?;
        position += 1;
    }
    for sidecar in sidecars {
        let facts = (
            Some(sidecar.codec.as_str()),
            sidecar.language.as_deref(),
            sidecar.forced,
            true,
        );
        insert_subtitle(conn, file, position, facts).await?;
        position += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(width: u32, height: u32) -> ProbedVideo {
        ProbedVideo {
            width,
            height,
            ..Default::default()
        }
    }

    #[test]
    fn resolution_goes_by_whichever_dimension_reaches_the_class() {
        assert_eq!(video(3840, 1600).resolution(), "4k", "scope");
        assert_eq!(video(1440, 1080).resolution(), "1080p", "4:3");
        assert_eq!(video(1920, 800).resolution(), "1080p", "scope");
        assert_eq!(video(2560, 1440).resolution(), "1440p");
        assert_eq!(video(720, 480).resolution(), "sd");
    }

    #[test]
    fn dolby_vision_outranks_its_base_layer() {
        let probe = ProbedVideo {
            transfer: Some("pq".to_string()),
            dovi_profile: Some("profile8".to_string()),
            ..Default::default()
        };
        assert_eq!(probe.dynamic_range(), "dolby_vision");
    }

    #[test]
    fn object_audio_reads_as_atmos_whatever_its_channels() {
        let track = ProbedAudio {
            channels: 8,
            audio_object_type: Some("joc".to_string()),
            ..Default::default()
        };
        assert_eq!(track.layout(), "atmos");
    }
}
