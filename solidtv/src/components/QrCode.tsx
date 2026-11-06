// SPDX-License-Identifier: AGPL-3.0-only
//
// A QR code in rectangles: there's no canvas or DOM to draw one with on every host,
// and a run of dark modules as one quad keeps the node count down. Dark on white, with
// the quiet zone scanners need, whatever's behind it.

import qrcode from "qrcode-generator";
import { createMemo, For } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { radius } from "../theme.js";

const QUIET_MODULES = 3;
const WHITE = 0xffffffff;
const DARK = 0x000000ff;

export default function QrCode(props: NodeProps & { text: string; size: number }) {
  const code = createMemo(() => {
    const qr = qrcode(0, "M");
    qr.addData(props.text);
    qr.make();
    const count = qr.getModuleCount();
    // A whole number of pixels each, or modules blur into their neighbours.
    const module = Math.floor(props.size / (count + QUIET_MODULES * 2));
    const inset = Math.floor((props.size - module * count) / 2);
    const runs: { x: number; y: number; width: number }[] = [];
    for (let row = 0; row < count; row++) {
      for (let column = 0; column < count; column++) {
        if (!qr.isDark(row, column)) continue;
        const start = column;
        while (column + 1 < count && qr.isDark(row, column + 1)) column++;
        runs.push({ x: inset + start * module, y: inset + row * module, width: (column - start + 1) * module });
      }
    }
    return { module, runs };
  });
  return (
    <view {...props} width={props.size} height={props.size} color={WHITE} borderRadius={radius.m} skipFocus>
      <For each={code().runs}>
        {(run) => <view x={run.x} y={run.y} width={run.width} height={code().module} color={DARK} />}
      </For>
    </view>
  );
}
