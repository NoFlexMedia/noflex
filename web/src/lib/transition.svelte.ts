// SPDX-License-Identifier: AGPL-3.0-only
//
// Shared-element morph between a poster card and the detail page. The
// clicked card records its key; the matching card (and only that one —
// view-transition names must be unique per snapshot) carries the name, so
// navigating back morphs the poster home again.

export const morph = $state<{ key: string | null }>({ key: null });

export function markMorph(key: string, node: Element | null): void {
  morph.key = key;
  const art = node?.querySelector<HTMLElement>(".art");
  if (art) art.style.setProperty("view-transition-name", "poster");
}
