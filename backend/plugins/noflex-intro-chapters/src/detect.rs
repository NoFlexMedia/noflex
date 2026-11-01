// SPDX-License-Identifier: AGPL-3.0-only
//
// Pure detection core of the intro-chapters plugin: chapter titles →
// intro/credits marker guesses. Nothing in here touches the WIT
// bindings (the `From<host_library::...>` conversions live in the
// wasm-only guest module), so `cargo test` exercises this file on the
// host target.

/// Fallback intro length when the intro chapter is the last one and
/// there is no next chapter to bound it.
const FALLBACK_INTRO_LEN_MS: i64 = 90_000;

/// Both intro bounds resolved from real chapters.
const CONFIDENCE_CHAPTER_BOUNDS: f32 = 0.9;
/// Intro end guessed, or credits-only.
const CONFIDENCE_FALLBACK: f32 = 0.7;

/// Substring matches against the lowercased title. "op" is NOT here:
/// as a substring it would hit "stop", "operation", ..., so it is
/// matched as a whole word instead (see [`is_intro_title`]).
const INTRO_KEYWORDS: &[&str] = &["intro", "opening", "prologue", "cold open"];
const CREDITS_KEYWORDS: &[&str] = &["credits", "end credits", "ending"];

/// One chapter marker, mirroring `host-library.chapter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub position_ms: i64,
    pub title: Option<String>,
}

/// Markers the edition already carries — any set marker short-circuits
/// detection entirely.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExistingMarkers {
    pub intro_start_ms: Option<i64>,
    pub intro_end_ms: Option<i64>,
    pub credits_start_ms: Option<i64>,
}

impl ExistingMarkers {
    fn any_set(self) -> bool {
        self.intro_start_ms.is_some()
            || self.intro_end_ms.is_some()
            || self.credits_start_ms.is_some()
    }
}

/// A detection result, mirroring `intro-detector.intro-detection`.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub intro_start_ms: Option<i64>,
    pub intro_end_ms: Option<i64>,
    pub credits_start_ms: Option<i64>,
    pub confidence: f32,
}

fn is_intro_title(lower: &str) -> bool {
    INTRO_KEYWORDS.iter().any(|kw| lower.contains(kw))
        // Whole-word match so the anime convention "OP" / "OP 1" hits
        // without "stop" or "operation" matching.
        || lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|token| token == "op")
}

fn is_credits_title(lower: &str) -> bool {
    CREDITS_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Guess intro/credits markers from chapter titles.
///
/// Returns `None` when the edition already has any marker (the
/// detector never clobbers or refines), when there are no chapters, or
/// when no title matches. The intro end is the next chapter's position;
/// for a trailing intro chapter it falls back to start + 90s, clamped
/// to the runtime when known. Bounds that come out nonsensical
/// (end ≤ start, or past the runtime) are dropped — an intro pair can
/// be discarded while a credits marker still stands.
pub fn detect(
    chapters: &[Chapter],
    runtime_ms: Option<i64>,
    existing: ExistingMarkers,
) -> Option<Detection> {
    if existing.any_set() || chapters.is_empty() {
        return None;
    }

    let intro = chapters.iter().position(|c| {
        c.title
            .as_deref()
            .is_some_and(|t| is_intro_title(&t.to_lowercase()))
    });
    let credits = chapters.iter().position(|c| {
        c.title
            .as_deref()
            .is_some_and(|t| is_credits_title(&t.to_lowercase()))
    });

    let mut chapter_bounded = false;
    let intro_bounds = intro.and_then(|i| {
        let start = chapters[i].position_ms;
        let (end, bounded) = match chapters.get(i + 1) {
            Some(next) => (next.position_ms, true),
            None => {
                let guess = start.saturating_add(FALLBACK_INTRO_LEN_MS);
                (
                    runtime_ms.map_or(guess, |runtime| guess.min(runtime)),
                    false,
                )
            }
        };
        let sane = end > start && runtime_ms.is_none_or(|runtime| end <= runtime);
        if sane {
            chapter_bounded = bounded;
            Some((start, end))
        } else {
            None
        }
    });

    let credits_start = credits
        .map(|i| chapters[i].position_ms)
        .filter(|start| runtime_ms.is_none_or(|runtime| *start <= runtime));

    if intro_bounds.is_none() && credits_start.is_none() {
        return None;
    }
    Some(Detection {
        intro_start_ms: intro_bounds.map(|(start, _)| start),
        intro_end_ms: intro_bounds.map(|(_, end)| end),
        credits_start_ms: credits_start,
        confidence: if chapter_bounded {
            CONFIDENCE_CHAPTER_BOUNDS
        } else {
            CONFIDENCE_FALLBACK
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(position_ms: i64, title: &str) -> Chapter {
        Chapter {
            position_ms,
            title: Some(title.into()),
        }
    }

    fn no_markers() -> ExistingMarkers {
        ExistingMarkers::default()
    }

    #[test]
    fn existing_markers_short_circuit() {
        let chapters = [chapter(0, "Intro"), chapter(90_000, "Episode")];
        for existing in [
            ExistingMarkers {
                intro_start_ms: Some(0),
                ..Default::default()
            },
            ExistingMarkers {
                intro_end_ms: Some(1),
                ..Default::default()
            },
            ExistingMarkers {
                credits_start_ms: Some(2),
                ..Default::default()
            },
        ] {
            assert_eq!(detect(&chapters, Some(600_000), existing), None);
        }
    }

    #[test]
    fn no_chapters_or_no_match_yields_none() {
        assert_eq!(detect(&[], Some(600_000), no_markers()), None);
        let chapters = [chapter(0, "Part 1"), chapter(300_000, "Part 2")];
        assert_eq!(detect(&chapters, Some(600_000), no_markers()), None);
        let untitled = [Chapter {
            position_ms: 0,
            title: None,
        }];
        assert_eq!(detect(&untitled, Some(600_000), no_markers()), None);
    }

    #[test]
    fn intro_bounded_by_next_chapter() {
        let chapters = [
            chapter(0, "Opening"),
            chapter(85_000, "Episode Start"),
            chapter(1_300_000, "End Credits"),
        ];
        let d = detect(&chapters, Some(1_400_000), no_markers()).expect("detection");
        assert_eq!(d.intro_start_ms, Some(0));
        assert_eq!(d.intro_end_ms, Some(85_000));
        assert_eq!(d.credits_start_ms, Some(1_300_000));
        assert_eq!(d.confidence, 0.9);
    }

    #[test]
    fn keyword_matching_is_case_insensitive() {
        for title in [
            "INTRO",
            "intro",
            "InTrO",
            "prologue",
            "COLD OPEN",
            "The Opening",
        ] {
            let chapters = [chapter(5_000, title), chapter(95_000, "Rest")];
            let d = detect(&chapters, None, no_markers()).unwrap_or_else(|| {
                panic!("`{title}` should mark an intro");
            });
            assert_eq!(d.intro_start_ms, Some(5_000));
        }
    }

    #[test]
    fn op_matches_as_a_word_only() {
        let chapters = [chapter(0, "OP"), chapter(90_000, "Episode")];
        let d = detect(&chapters, None, no_markers()).expect("OP is an intro");
        assert_eq!(d.intro_start_ms, Some(0));

        let chapters = [chapter(0, "OP 2"), chapter(90_000, "Episode")];
        assert!(detect(&chapters, None, no_markers()).is_some());

        // Substring "op" inside other words must not match.
        let chapters = [chapter(0, "Tropical Operation"), chapter(90_000, "Stop")];
        assert_eq!(detect(&chapters, None, no_markers()), None);
    }

    #[test]
    fn trailing_intro_falls_back_to_ninety_seconds() {
        let chapters = [chapter(10_000, "Previously"), chapter(100_000, "Intro")];
        let d = detect(&chapters, Some(600_000), no_markers()).expect("detection");
        assert_eq!(d.intro_start_ms, Some(100_000));
        assert_eq!(d.intro_end_ms, Some(190_000));
        assert_eq!(d.confidence, 0.7);
    }

    #[test]
    fn fallback_end_clamps_to_runtime() {
        let chapters = [chapter(540_000, "Intro")];
        let d = detect(&chapters, Some(600_000), no_markers()).expect("detection");
        assert_eq!(d.intro_end_ms, Some(600_000));
        assert_eq!(d.confidence, 0.7);
    }

    #[test]
    fn nonsensical_intro_bounds_are_dropped() {
        // Intro chapter starts at the runtime: even the clamped end is
        // not past the start, so the pair is dropped — but the credits
        // marker still stands on its own.
        let chapters = [chapter(500_000, "Credits"), chapter(600_000, "Intro")];
        let d = detect(&chapters, Some(600_000), no_markers()).expect("credits survive");
        assert_eq!(d.intro_start_ms, None);
        assert_eq!(d.intro_end_ms, None);
        assert_eq!(d.credits_start_ms, Some(500_000));
        assert_eq!(d.confidence, 0.7);
    }

    #[test]
    fn credits_only_detection() {
        let chapters = [chapter(0, "Part 1"), chapter(1_200_000, "Ending")];
        let d = detect(&chapters, Some(1_300_000), no_markers()).expect("detection");
        assert_eq!(d.intro_start_ms, None);
        assert_eq!(d.credits_start_ms, Some(1_200_000));
        assert_eq!(d.confidence, 0.7);
    }

    #[test]
    fn credits_past_runtime_is_dropped() {
        let chapters = [chapter(0, "Part 1"), chapter(2_000_000, "Credits")];
        assert_eq!(detect(&chapters, Some(1_300_000), no_markers()), None);
    }
}
