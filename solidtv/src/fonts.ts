// SPDX-License-Identifier: AGPL-3.0-only
//
// The renderer draws SDF text only. See scripts/build-fonts.mjs. A weight is its
// own family, `${family}${weight}`, which is how `fontWeight` resolves.

import type { FontLoadOptions } from "@solidtv/solid";

const base = import.meta.env.BASE_URL;
// Inter's own, from its OS/2 and hhea tables.
const metrics = { ascender: 2728, descender: -680, lineGap: 0, unitsPerEm: 2816 };

const face = (fontFamily: string, weight: number): FontLoadOptions => ({
  type: "msdf",
  fontFamily,
  atlasUrl: `${base}fonts/Inter-${weight}.msdf.png`,
  atlasDataUrl: `${base}fonts/Inter-${weight}.msdf.json`,
  metrics,
});

export const fonts: FontLoadOptions[] = [face("Inter", 400), face("Inter600", 600), face("Inter700", 700)];
