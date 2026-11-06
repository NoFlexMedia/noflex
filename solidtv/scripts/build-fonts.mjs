// SPDX-License-Identifier: AGPL-3.0-only
//
// Renders Inter into the MSDF atlases in public/fonts, which are committed: the
// renderer draws SDF text only, and they change only with the character set.

import { createRequire } from "node:module";
import { promisify } from "node:util";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const generate = promisify((font, options, done) => require("msdf-bmfont-xml")(font, options, (error, textures, json) => done(error, { textures, json })));

const OUT = "public/fonts";
const WEIGHTS = { Regular: 400, SemiBold: 600, Bold: 700 };
// What metadata titles are written in that one atlas can hold. CJK can't fit: see AGENTS.md.
const RANGES = [[0x20, 0x7e], [0xa0, 0x17f], [0x370, 0x3ff], [0x400, 0x45f], [0x2010, 0x2027], [0x2030, 0x203a], [0x20ac, 0x20ac], [0x2122, 0x2122], [0x2190, 0x2193], [0x2212, 0x2212], [0xd7, 0xd7]];
const charset = RANGES.flatMap(([from, to]) => Array.from({ length: to - from + 1 }, (_, index) => String.fromCodePoint(from + index))).join("");

const round = (value) => Math.round(value * 100) / 100;

// A TV parses this on the main thread at launch, and Inter's whole kerning table
// is most of a megabyte: keep what the renderer reads, and pairs that show.
function slim(data, file) {
  const known = new Set(data.chars.map((char) => char.id));
  return {
    pages: [`${file}.msdf.png`],
    info: { size: data.info.size },
    common: { lineHeight: data.common.lineHeight, base: data.common.base, scaleW: data.common.scaleW, scaleH: data.common.scaleH },
    distanceField: data.distanceField,
    chars: data.chars.map(({ id, x, y, width, height, xoffset, yoffset, xadvance }) => ({ id, x, y, width, height, xoffset: round(xoffset), yoffset: round(yoffset), xadvance: round(xadvance) })),
    kernings: data.kernings
      .filter(({ first, second, amount }) => first < 0x250 && second < 0x250 && known.has(first) && known.has(second) && Math.abs(amount) >= 0.5)
      .map(({ first, second, amount }) => ({ first, second, amount: round(amount) })),
  };
}

mkdirSync(OUT, { recursive: true });
for (const [name, weight] of Object.entries(WEIGHTS)) {
  const font = readFileSync(`node_modules/inter-ui/Inter (web)/Inter-${name}.woff`);
  const file = `Inter-${weight}`;
  const { textures, json } = await generate(font, {
    filename: join(OUT, file),
    outputType: "json",
    fieldType: "msdf",
    fontSize: 42,
    distanceRange: 6,
    textureSize: [2048, 2048],
    texturePadding: 2,
    smartSize: true,
    pot: false,
    charset,
  });
  if (textures.length !== 1) throw new Error(`${name} spilled onto ${textures.length} pages; the renderer reads one`);
  writeFileSync(join(OUT, `${file}.msdf.png`), textures[0].texture);
  const data = slim(JSON.parse(json.data), file);
  writeFileSync(join(OUT, `${file}.msdf.json`), JSON.stringify(data));
  rmSync(join(OUT, `${file}.cfg`), { force: true });
  console.log(`${file}: ${data.chars.length} glyphs, ${data.common.scaleW}×${data.common.scaleH}`);
}
