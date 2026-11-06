// SPDX-License-Identifier: AGPL-3.0-only

import { renderer } from "@solidtv/solid";
import { mediaUrl } from "$lib/tokens.js";
import type { Trickplay } from "$lib/types.js";

/** The sprite tile nearest a position, as a texture, or null where the sheets don't reach. */
export function thumbAt(trick: Trickplay | null | undefined, positionMs: number) {
  if (!trick?.sheets?.length) return null;
  const perSheet = trick.tileColumns * trick.tileRows;
  const index = Math.floor(positionMs / trick.intervalMs);
  const page = trick.sheets.find((item) => item.index === Math.floor(index / perSheet));
  if (!page) return null;
  const within = index % perSheet;
  return renderer.createTexture("SubTexture", {
    texture: renderer.createTexture("ImageTexture", { src: mediaUrl(page.url) }),
    x: (within % trick.tileColumns) * trick.thumbWidth,
    y: Math.floor(within / trick.tileColumns) * trick.thumbHeight,
    w: trick.thumbWidth,
    h: trick.thumbHeight,
  });
}

const SCENES = 12;
const SHORTEST_SCENE_MS = 30_000;

export interface Stop { positionMs: number; title?: string | null }

/**
 * Where Down in the player can jump: the file's chapters, or where it has none,
 * evenly spaced scenes wherever trickplay can show what's there.
 */
export function chapterStops(chapters: Stop[] | null | undefined, trick: Trickplay | null | undefined, runtimeMs: number | null | undefined): Stop[] {
  if (chapters?.length) return chapters;
  if (!trick?.sheets?.length || !runtimeMs) return [];
  const spacing = Math.max(SHORTEST_SCENE_MS, runtimeMs / SCENES);
  const stops: Stop[] = [];
  for (let at = 0; at < runtimeMs - spacing / 2; at += spacing) stops.push({ positionMs: Math.round(at), title: null });
  return stops.length > 1 ? stops : [];
}
