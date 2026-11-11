// SPDX-License-Identifier: AGPL-3.0-only
//
// Intro and credits markers read from an edition's chapter titles
// ("Opening", "OP", "End Credits", ...), set while a scan writes the
// edition so they need no pass of their own.

use noflex_storage::domain::{Chapter, Edition};

/// How long an intro is taken to run when its chapter is the last one,
/// so no next chapter bounds it.
const FALLBACK_INTRO_LEN_MS: i64 = 90_000;

/// Matched anywhere in the lowercased title. "op" is matched as a whole
/// word instead (see [`ChapterMarkers::is_intro`]), as a substring hits
/// "stop" and "operation".
const INTRO_KEYWORDS: &[&str] = &["intro", "opening", "prologue", "cold open"];
const CREDITS_KEYWORDS: &[&str] = &["credits", "end credits", "ending"];

/// The intro and credits markers an edition's chapter titles name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChapterMarkers {
    pub intro_start_ms: Option<i64>,
    pub intro_end_ms: Option<i64>,
    pub credits_start_ms: Option<i64>,
}

impl ChapterMarkers {
    /// Reads markers from `chapters`, or `None` when no title names an
    /// intro or credits.
    ///
    /// The first intro chapter runs to the next chapter, or for 90 s when
    /// it's the last, clamped to `runtime_ms` when that's known. An intro
    /// that comes out empty or past the runtime is dropped, and so is a
    /// credits marker past the runtime; either can stand without the other.
    pub(crate) fn detect(chapters: &[Chapter], runtime_ms: Option<i64>) -> Option<Self> {
        let titled = |matches: fn(&str) -> bool| {
            chapters.iter().position(|c| {
                c.title
                    .as_deref()
                    .is_some_and(|t| matches(&t.to_lowercase()))
            })
        };
        let within_runtime = |ms: i64| runtime_ms.is_none_or(|runtime| ms <= runtime);

        let intro = titled(Self::is_intro).and_then(|i| {
            let start = chapters[i].position_ms;
            let end = match chapters.get(i + 1) {
                Some(next) => next.position_ms,
                None => {
                    let guess = start.saturating_add(FALLBACK_INTRO_LEN_MS);
                    runtime_ms.map_or(guess, |runtime| guess.min(runtime))
                }
            };
            (end > start && within_runtime(end)).then_some((start, end))
        });
        let credits_start_ms = titled(Self::is_credits)
            .map(|i| chapters[i].position_ms)
            .filter(|start| within_runtime(*start));

        if intro.is_none() && credits_start_ms.is_none() {
            return None;
        }
        Some(Self {
            intro_start_ms: intro.map(|(start, _)| start),
            intro_end_ms: intro.map(|(_, end)| end),
            credits_start_ms,
        })
    }

    /// Sets `edition`'s markers from its chapters and runtime. An edition
    /// that already has any marker keeps what it has, as one set
    /// elsewhere knows better than a chapter title.
    pub(crate) fn mark(edition: &mut Edition) {
        let marked = edition.intro_start_ms.is_some()
            || edition.intro_end_ms.is_some()
            || edition.credits_start_ms.is_some();
        if marked {
            return;
        }
        if let Some(markers) = Self::detect(&edition.chapters, edition.runtime_ms) {
            edition.intro_start_ms = markers.intro_start_ms;
            edition.intro_end_ms = markers.intro_end_ms;
            edition.credits_start_ms = markers.credits_start_ms;
        }
    }

    fn is_intro(lower: &str) -> bool {
        INTRO_KEYWORDS.iter().any(|kw| lower.contains(kw))
            // The anime convention "OP" / "OP 1".
            || lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|token| token == "op")
    }

    fn is_credits(lower: &str) -> bool {
        CREDITS_KEYWORDS.iter().any(|kw| lower.contains(kw))
    }
}

#[cfg(test)]
mod tests {
    use noflex_core::{EditionId, WorkId};

    use super::*;

    fn chapter(position_ms: i64, title: &str) -> Chapter {
        Chapter {
            position_ms,
            title: Some(title.into()),
        }
    }

    fn markers(intro: Option<(i64, i64)>, credits: Option<i64>) -> Option<ChapterMarkers> {
        Some(ChapterMarkers {
            intro_start_ms: intro.map(|(start, _)| start),
            intro_end_ms: intro.map(|(_, end)| end),
            credits_start_ms: credits,
        })
    }

    #[test]
    fn no_chapters_or_no_match_yields_none() {
        assert_eq!(ChapterMarkers::detect(&[], Some(600_000)), None);
        let chapters = [chapter(0, "Part 1"), chapter(300_000, "Part 2")];
        assert_eq!(ChapterMarkers::detect(&chapters, Some(600_000)), None);
        let untitled = [Chapter {
            position_ms: 0,
            title: None,
        }];
        assert_eq!(ChapterMarkers::detect(&untitled, Some(600_000)), None);
    }

    #[test]
    fn intro_bounded_by_next_chapter() {
        let chapters = [
            chapter(0, "Opening"),
            chapter(85_000, "Episode Start"),
            chapter(1_300_000, "End Credits"),
        ];
        assert_eq!(
            ChapterMarkers::detect(&chapters, Some(1_400_000)),
            markers(Some((0, 85_000)), Some(1_300_000))
        );
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
            assert_eq!(
                ChapterMarkers::detect(&chapters, None),
                markers(Some((5_000, 95_000)), None),
                "`{title}` should mark an intro"
            );
        }
    }

    #[test]
    fn op_matches_as_a_word_only() {
        let chapters = [chapter(0, "OP"), chapter(90_000, "Episode")];
        assert!(ChapterMarkers::detect(&chapters, None).is_some());
        let chapters = [chapter(0, "OP 2"), chapter(90_000, "Episode")];
        assert!(ChapterMarkers::detect(&chapters, None).is_some());
        let chapters = [chapter(0, "Tropical Operation"), chapter(90_000, "Stop")];
        assert_eq!(ChapterMarkers::detect(&chapters, None), None);
    }

    #[test]
    fn trailing_intro_runs_ninety_seconds_within_the_runtime() {
        let chapters = [chapter(10_000, "Previously"), chapter(100_000, "Intro")];
        assert_eq!(
            ChapterMarkers::detect(&chapters, Some(600_000)),
            markers(Some((100_000, 190_000)), None)
        );
        let chapters = [chapter(540_000, "Intro")];
        assert_eq!(
            ChapterMarkers::detect(&chapters, Some(600_000)),
            markers(Some((540_000, 600_000)), None)
        );
    }

    #[test]
    fn markers_past_the_runtime_are_dropped_alone() {
        // The intro starts at the runtime, so even its clamped end isn't
        // past its start; the credits still stand.
        let chapters = [chapter(500_000, "Credits"), chapter(600_000, "Intro")];
        assert_eq!(
            ChapterMarkers::detect(&chapters, Some(600_000)),
            markers(None, Some(500_000))
        );
        let chapters = [chapter(0, "Part 1"), chapter(2_000_000, "Credits")];
        assert_eq!(ChapterMarkers::detect(&chapters, Some(1_300_000)), None);
    }

    #[test]
    fn mark_keeps_markers_set_elsewhere() {
        let mut edition = Edition {
            id: EditionId::new(),
            work_id: WorkId::new(),
            name: "Default".into(),
            runtime_ms: Some(1_400_000),
            intro_start_ms: None,
            intro_end_ms: None,
            credits_start_ms: Some(1_350_000),
            chapters: vec![
                chapter(0, "Intro"),
                chapter(90_000, "Episode"),
                chapter(1_300_000, "Credits"),
            ],
        };
        ChapterMarkers::mark(&mut edition);
        assert_eq!(
            (
                edition.intro_start_ms,
                edition.intro_end_ms,
                edition.credits_start_ms
            ),
            (None, None, Some(1_350_000))
        );

        edition.credits_start_ms = None;
        ChapterMarkers::mark(&mut edition);
        assert_eq!(
            (
                edition.intro_start_ms,
                edition.intro_end_ms,
                edition.credits_start_ms
            ),
            (Some(0), Some(90_000), Some(1_300_000))
        );
    }
}
