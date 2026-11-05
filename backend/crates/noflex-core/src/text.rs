// SPDX-License-Identifier: AGPL-3.0-only
//
// Small text helpers shared by the scanner, the metadata layer, and the
// GraphQL surface.

/// Slugify into stable key fragments (tag slugs, scan-sort keys).
/// Lower-cased, runs of non-alphanumeric chars collapsed to a single
/// `-`, leading/trailing dashes trimmed.
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = true; // suppresses leading dash
    for ch in input.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Languages as files spell them: ISO 639-2 terminology, then bibliographic where
/// it differs, then ISO 639-1. Muxers use all three for the same language.
const LANGUAGE_SPELLINGS: &[&[&str]] = &[
    &["eng", "en"],
    &["fra", "fre", "fr"],
    &["deu", "ger", "de"],
    &["spa", "es"],
    &["ita", "it"],
    &["por", "pt"],
    &["nld", "dut", "nl"],
    &["swe", "sv"],
    &["nor", "no"],
    &["nob", "nb"],
    &["nno", "nn"],
    &["dan", "da"],
    &["fin", "fi"],
    &["isl", "ice", "is"],
    &["pol", "pl"],
    &["ces", "cze", "cs"],
    &["slk", "slo", "sk"],
    &["hun", "hu"],
    &["ron", "rum", "ro"],
    &["bul", "bg"],
    &["hrv", "hr"],
    &["srp", "sr"],
    &["slv", "sl"],
    &["ell", "gre", "el"],
    &["tur", "tr"],
    &["rus", "ru"],
    &["ukr", "uk"],
    &["ara", "ar"],
    &["heb", "he"],
    &["fas", "per", "fa"],
    &["hin", "hi"],
    &["ben", "bn"],
    &["tam", "ta"],
    &["tel", "te"],
    &["tha", "th"],
    &["vie", "vi"],
    &["ind", "id"],
    &["msa", "may", "ms"],
    &["zho", "chi", "zh"],
    &["jpn", "ja"],
    &["kor", "ko"],
];

/// `code`'s language in the one spelling used to compare them. An unknown code
/// keeps its own, lowercased.
pub fn canonical_language(code: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    LANGUAGE_SPELLINGS
        .iter()
        .find(|spellings| spellings.contains(&code.as_str()))
        .map_or(code, |spellings| spellings[0].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("The Matrix"), "the-matrix");
        assert_eq!(slugify("Director's Cut"), "director-s-cut");
        assert_eq!(slugify("  spaces  "), "spaces");
    }
}
