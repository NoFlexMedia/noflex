// SPDX-License-Identifier: AGPL-3.0-only

const SCENES = 12;
const SHORTEST_SCENE_MS = 30_000;

export interface Stop { positionMs: number; title?: string | null }

/**
 * Where the player can jump: the file's chapters, or where it has none, evenly
 * spaced scenes wherever trickplay can show what's there. As `solidtv/`'s.
 */
export function chapterStops(chapters: Stop[] | null | undefined, hasTrickplay: boolean, runtimeMs: number | null | undefined): Stop[] {
  if (chapters?.length) return chapters;
  if (!hasTrickplay || !runtimeMs) return [];
  const spacing = Math.max(SHORTEST_SCENE_MS, runtimeMs / SCENES);
  const stops: Stop[] = [];
  for (let at = 0; at < runtimeMs - spacing / 2; at += spacing) stops.push({ positionMs: Math.round(at), title: null });
  return stops.length > 1 ? stops : [];
}
