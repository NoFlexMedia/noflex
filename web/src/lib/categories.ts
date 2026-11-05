// SPDX-License-Identifier: AGPL-3.0-only
//
// The server's `ContentCategory` values, as the app names and draws them.

import BookOpen from "phosphor-svelte/lib/BookOpen";
import FilmStrip from "phosphor-svelte/lib/FilmStrip";
import ImageIcon from "phosphor-svelte/lib/ImageIcon";
import MusicNotes from "phosphor-svelte/lib/MusicNotes";
import Sparkle from "phosphor-svelte/lib/Sparkle";
import Television from "phosphor-svelte/lib/Television";

// `shelves`: opens on recommendations and new arrivals, with the full grid a tab away.
export const CATEGORIES: readonly { value: string; slug: string; label: string; icon: any; shelves?: boolean }[] = [
  { value: "MOVIES", slug: "movies", label: "Movies", icon: FilmStrip, shelves: true },
  { value: "SHOWS", slug: "shows", label: "TV Shows", icon: Television },
  { value: "ANIME", slug: "anime", label: "Anime", icon: Sparkle },
  { value: "MUSIC", slug: "music", label: "Music", icon: MusicNotes },
  { value: "AUDIOBOOKS", slug: "audiobooks", label: "Audiobooks", icon: BookOpen },
  { value: "PHOTOS", slug: "photos", label: "Photos", icon: ImageIcon },
];

export type Category = (typeof CATEGORIES)[number];

/** The categories in `counts` (`contentCategories` rows), in display order, each with its `workCount`. */
export function availableCategories(counts: readonly { category: string; workCount: number }[]): (Category & { workCount: number })[] {
  return CATEGORIES.flatMap((category) => {
    const row = counts.find((item) => item.category === category.value);
    return row ? [{ ...category, workCount: row.workCount }] : [];
  });
}
