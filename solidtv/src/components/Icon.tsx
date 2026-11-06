// SPDX-License-Identifier: AGPL-3.0-only
//
// A Phosphor icon or Simple Icons mark, from the one white sprite sheet the renderer tints.

import { renderer, type NodeProps } from "@solidtv/solid";
import { ICON_CELL, ICON_CELLS, type IconName } from "$lib/icons.generated.js";
import { color } from "../theme.js";

let sheet: any;
const sprites = new Map<IconName, any>();

function sprite(name: IconName) {
  let texture = sprites.get(name);
  if (!texture) {
    sheet ??= renderer.createTexture("ImageTexture", { src: `${import.meta.env.BASE_URL}icons.png` });
    const [x, y] = ICON_CELLS[name];
    texture = renderer.createTexture("SubTexture", { texture: sheet, x, y, w: ICON_CELL, h: ICON_CELL });
    sprites.set(name, texture);
  }
  return texture;
}

export default function Icon(props: NodeProps & { name: IconName; size?: number }) {
  return <view {...props} texture={sprite(props.name)} width={props.size ?? 36} height={props.size ?? 36} color={props.color ?? color.text} />;
}
