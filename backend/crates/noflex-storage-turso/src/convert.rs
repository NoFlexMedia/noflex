// SPDX-License-Identifier: AGPL-3.0-only
//
// Type ↔ row mapping helpers. Keep these in one place so SQL strings can
// rely on consistent column shapes.

use noflex_core::{ExternalIdKind, MetadataStatus, WorkKind};
use noflex_storage::{
    canonical::canonical_timestamp,
    domain::LibraryKind,
    error::{StorageError, StorageResult},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use turso::Value;
use uuid::Uuid;

use crate::error::{backend, backend_msg};

pub(crate) fn uuid_to_blob(id: Uuid) -> Vec<u8> {
    id.as_bytes().to_vec()
}

pub(crate) fn blob_to_uuid(bytes: &[u8]) -> StorageResult<Uuid> {
    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| backend_msg(format!("expected 16-byte uuid blob, got {}", bytes.len())))?;
    Ok(Uuid::from_bytes(arr))
}

/// Timestamps are stored as [`canonical_timestamp`] text so SQL can
/// compare and sort them lexically.
pub(crate) fn dt_to_text(dt: OffsetDateTime) -> StorageResult<String> {
    Ok(canonical_timestamp(dt))
}

pub(crate) fn text_to_dt(s: &str) -> StorageResult<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).map_err(backend)
}

pub(crate) fn library_kind_to_text(k: LibraryKind) -> &'static str {
    match k {
        LibraryKind::Movies => "movies",
        LibraryKind::Shows => "shows",
        LibraryKind::Music => "music",
        LibraryKind::Audiobooks => "audiobooks",
        LibraryKind::Photos => "photos",
        LibraryKind::Mixed => "mixed",
    }
}

pub(crate) fn text_to_library_kind(s: &str) -> StorageResult<LibraryKind> {
    Ok(match s {
        "movies" => LibraryKind::Movies,
        "shows" => LibraryKind::Shows,
        "music" => LibraryKind::Music,
        "audiobooks" => LibraryKind::Audiobooks,
        "photos" => LibraryKind::Photos,
        "mixed" => LibraryKind::Mixed,
        other => return Err(backend_msg(format!("unknown library kind {other}"))),
    })
}

pub(crate) fn work_kind_to_text(k: WorkKind) -> &'static str {
    match k {
        WorkKind::Movie => "movie",
        WorkKind::Series => "series",
        WorkKind::Season => "season",
        WorkKind::Episode => "episode",
        WorkKind::Album => "album",
        WorkKind::Track => "track",
        WorkKind::Artist => "artist",
        WorkKind::AudioBook => "audio_book",
        WorkKind::Photo => "photo",
    }
}

pub(crate) fn text_to_work_kind(s: &str) -> StorageResult<WorkKind> {
    Ok(match s {
        "movie" => WorkKind::Movie,
        "series" => WorkKind::Series,
        "season" => WorkKind::Season,
        "episode" => WorkKind::Episode,
        "album" => WorkKind::Album,
        "track" => WorkKind::Track,
        "artist" => WorkKind::Artist,
        "audio_book" => WorkKind::AudioBook,
        "photo" => WorkKind::Photo,
        other => return Err(backend_msg(format!("unknown work kind {other}"))),
    })
}

pub(crate) fn metadata_status_to_text(s: MetadataStatus) -> &'static str {
    match s {
        MetadataStatus::Unconfirmed => "unconfirmed",
        MetadataStatus::Confirmed => "confirmed",
        MetadataStatus::UserEdited => "user_edited",
    }
}

pub(crate) fn text_to_metadata_status(s: &str) -> StorageResult<MetadataStatus> {
    Ok(match s {
        "unconfirmed" => MetadataStatus::Unconfirmed,
        "confirmed" => MetadataStatus::Confirmed,
        "user_edited" => MetadataStatus::UserEdited,
        other => return Err(backend_msg(format!("unknown metadata status {other}"))),
    })
}

pub(crate) fn ext_id_kind_to_text(k: ExternalIdKind) -> &'static str {
    match k {
        ExternalIdKind::Tmdb => "tmdb",
        ExternalIdKind::Tvdb => "tvdb",
        ExternalIdKind::Imdb => "imdb",
        ExternalIdKind::MusicBrainzReleaseGroup => "music_brainz_release_group",
        ExternalIdKind::MusicBrainzArtist => "music_brainz_artist",
        ExternalIdKind::MusicBrainzRecording => "music_brainz_recording",
        ExternalIdKind::AniDb => "ani_db",
        ExternalIdKind::Custom => "custom",
    }
}

/// Read a `BLOB`/`TEXT` column where the value is known not to be NULL.
pub(crate) fn col_blob(v: &Value) -> StorageResult<&[u8]> {
    v.as_blob()
        .map(|b| b.as_slice())
        .ok_or_else(|| backend_msg(format!("expected BLOB column, got {v:?}")))
}

pub(crate) fn col_text(v: &Value) -> StorageResult<&str> {
    v.as_text()
        .map(|s| s.as_str())
        .ok_or_else(|| backend_msg(format!("expected TEXT column, got {v:?}")))
}

pub(crate) fn col_text_opt(v: &Value) -> StorageResult<Option<String>> {
    match v {
        Value::Null => Ok(None),
        Value::Text(s) => Ok(Some(s.clone())),
        other => Err(backend_msg(format!(
            "expected TEXT or NULL column, got {other:?}"
        ))),
    }
}

pub(crate) fn col_int(v: &Value) -> StorageResult<i64> {
    v.as_integer()
        .copied()
        .ok_or_else(|| backend_msg(format!("expected INTEGER column, got {v:?}")))
}

pub(crate) fn col_int_opt(v: &Value) -> StorageResult<Option<i64>> {
    match v {
        Value::Null => Ok(None),
        Value::Integer(i) => Ok(Some(*i)),
        other => Err(backend_msg(format!(
            "expected INTEGER or NULL column, got {other:?}"
        ))),
    }
}

pub(crate) fn col_blob_opt(v: &Value) -> StorageResult<Option<Vec<u8>>> {
    match v {
        Value::Null => Ok(None),
        Value::Blob(b) => Ok(Some(b.clone())),
        other => Err(backend_msg(format!(
            "expected BLOB or NULL column, got {other:?}"
        ))),
    }
}

pub(crate) fn col_real(v: &Value) -> StorageResult<f32> {
    match v {
        Value::Real(r) => Ok(*r as f32),
        other => Err(backend_msg(format!("expected REAL column, got {other:?}"))),
    }
}

pub(crate) fn col_real_opt(v: &Value) -> StorageResult<Option<f32>> {
    match v {
        Value::Null => Ok(None),
        Value::Real(r) => Ok(Some(*r as f32)),
        other => Err(backend_msg(format!(
            "expected REAL or NULL column, got {other:?}"
        ))),
    }
}

pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(s: &str) -> StorageResult<T> {
    serde_json::from_str(s).map_err(|e| StorageError::Backend(Box::new(e)))
}

pub(crate) fn to_json<T: serde::Serialize>(v: &T) -> StorageResult<String> {
    serde_json::to_string(v).map_err(|e| StorageError::Backend(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn timestamps_store_as_fixed_width_utc() {
        let at = datetime!(2026-05-01 10:15:30.25 +05:30);
        let text = dt_to_text(at).unwrap();
        assert_eq!(text, "2026-05-01T04:45:30.250000000Z");
        assert_eq!(text_to_dt(&text).unwrap(), at);
        // Rows written before normalization still parse.
        assert_eq!(text_to_dt("2026-05-01T10:15:30.25+05:30").unwrap(), at);
    }
}
