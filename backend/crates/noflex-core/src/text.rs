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
