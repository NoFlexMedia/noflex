// SPDX-License-Identifier: AGPL-3.0-only

use time::{OffsetDateTime, UtcOffset};

/// Fixed-width UTC rendering of `at`: `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ`,
/// always nine fractional digits. For years 0–9999 byte-wise order equals
/// chronological order, so backends may store and compare it as text. The
/// result is valid RFC 3339 and parses back to the same instant.
pub fn canonical_timestamp(at: OffsetDateTime) -> String {
    let at = at.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.nanosecond()
    )
}

/// The [`TrickplayAsset::source_fingerprint`][crate::TrickplayAsset::source_fingerprint]
/// for a media file: `"{path}\n{size_bytes}\n{canonical_timestamp(mtime)}"`.
/// Backends may recompute this from stored media columns, so producers
/// must use this function rather than formatting their own.
pub fn trickplay_source_fingerprint(path: &str, size_bytes: u64, mtime: OffsetDateTime) -> String {
    format!("{path}\n{size_bytes}\n{}", canonical_timestamp(mtime))
}

#[cfg(test)]
mod tests {
    use time::{Duration, format_description::well_known::Rfc3339, macros::datetime};

    use super::*;

    #[test]
    fn normalizes_offset_and_width() {
        let at = datetime!(2026-03-01 05:30:00.5 +05:30);
        assert_eq!(canonical_timestamp(at), "2026-03-01T00:00:00.500000000Z");
        let parsed = OffsetDateTime::parse(&canonical_timestamp(at), &Rfc3339).unwrap();
        assert_eq!(parsed, at);
    }

    #[test]
    fn text_order_is_chronological() {
        let early = datetime!(2026-03-01 05:29:59.9 +05:30);
        let late = datetime!(2026-03-01 00:00:00 UTC);
        assert!(early < late);
        assert!(canonical_timestamp(early) < canonical_timestamp(late));
        let fine = late + Duration::nanoseconds(1);
        assert!(canonical_timestamp(late) < canonical_timestamp(fine));
    }
}
