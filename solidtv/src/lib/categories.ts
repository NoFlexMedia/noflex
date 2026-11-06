// SPDX-License-Identifier: AGPL-3.0-only
//
// The server's `ContentCategory` values, as the app names and draws them.

import type { IconName } from "$lib/icons.generated.js";

// `shelves`: opens on recommendations and new arrivals, with the full grid a step away.
export const CATEGORIES: readonly { value: string; slug: string; label: string; icon: IconName; shelves?: boolean }[] = [
  { value: "MOVIES", slug: "movies", label: "Movies", icon: "film-strip", shelves: true },
  { value: "SHOWS", slug: "shows", label: "TV Shows", icon: "television", shelves: true },
  { value: "ANIME", slug: "anime", label: "Anime", icon: "sparkle", shelves: true },
  { value: "MUSIC", slug: "music", label: "Music", icon: "music-notes" },
  { value: "AUDIOBOOKS", slug: "audiobooks", label: "Audiobooks", icon: "book-open" },
  { value: "PHOTOS", slug: "photos", label: "Photos", icon: "image" },
];

export const categoryBySlug = (slug: string) => CATEGORIES.find((category) => category.slug === slug);

export function availableCategories(counts: readonly { category: string; workCount: number }[]) {
  return CATEGORIES.filter((category) => counts.some((row) => row.category === category.value));
}
