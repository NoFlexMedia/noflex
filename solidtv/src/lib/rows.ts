// SPDX-License-Identifier: AGPL-3.0-only

import type { ElementNode } from "@solidtv/solid";
import { scrollRow } from "@solidtv/solid/primitives";
import { originX } from "$lib/viewport.js";
import { SCREEN } from "../theme.js";

// A row's `width` is its content's once laid out, so what's in view is the screen's to say.
const RIGHT_MARGIN = 32;

/**
 * A pointer focuses a row's child directly, which the row's scroller never hears
 * about, so a card hovered under the nav would stay there. Step the row as the keys
 * would. Call it a tick after focus, when a key press has already had the row scroll.
 */
export function revealInRow(child: ElementNode): void {
  const row = child.parent;
  if (!row || row.scroll !== "auto") return;
  const index = row.children.indexOf(child);
  // Where the row's zero lands on screen, whatever it's scrolled to or animating towards.
  const zero = (row.lng.absX ?? 0) - (row.lng.x ?? 0);
  const right = originX + SCREEN.width - RIGHT_MARGIN;
  for (let step = 0; step < row.children.length; step++) {
    const left = (row.x ?? 0) + (child.x ?? 0);
    if (left < 0) scrollRow(index, row, child, index + 1);
    else if (zero + left + (child.width ?? 0) > right) scrollRow(index, row, child, index - 1);
    else return;
  }
}
