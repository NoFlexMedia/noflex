// SPDX-License-Identifier: AGPL-3.0-only
//
// The art behind every page, crossfading as it changes, under scrims that keep
// text readable: chrome stays dark so artwork carries the color.

import { createEffect, createSignal, Index, on } from "solid-js";
import { ambientColor, sized } from "$lib/images.js";
import { ui } from "$lib/ui.js";
import { AMBIENT, color, duration, ease, gradient, SCREEN, toward } from "../theme.js";

const WIDTH = SCREEN.width;
const HEIGHT = SCREEN.height;
const fade = { alpha: { duration: duration.slow * 1.5, easing: ease.inOut } };

export default function Backdrop() {
  // Two layers trading places; the old one fades only after the new one has arrived.
  const [layers, setLayers] = createSignal<(string | null)[]>([null, null]);
  const [front, setFront] = createSignal(0);
  const [loaded, setLoaded] = createSignal(-1);

  createEffect(on(() => ui.backdrop, (url) => {
    const src = url ? sized(url, WIDTH, HEIGHT) : null;
    if (src === layers()[front()]) return;
    const next = front() === 0 ? 1 : 0;
    // A layer already holding this image has nothing left to load, so no `loaded` is coming.
    setLoaded(src && src === layers()[next] ? next : -1);
    setLayers((current) => { const copy = current.slice(); copy[next] = src; return copy; });
    setFront(next);
    if (!url) { ui.ambient = AMBIENT; return; }
    ambientColor(url).then((tint) => { if (ui.backdrop === url) ui.ambient = tint ?? AMBIENT; });
  }));

  return (
    <view width={WIDTH} height={HEIGHT} skipFocus>
      <view width={WIDTH} height={HEIGHT} color={color.bg} />
      <view width={WIDTH} height={HEIGHT / 1.6} linearGradient={gradient([((ui.ambient & 0xffffff00) | 0x33) >>> 0, (ui.ambient & 0xffffff00) >>> 0], toward.down)} />
      <Index each={layers()}>
        {(src, index) => (
          <view
            width={WIDTH}
            height={HEIGHT}
            src={src() ?? undefined}
            // The renderer only loads what it would draw, so a layer waiting on its image can't be at 0.
            alpha={front() !== index || !src() ? 0 : loaded() === index ? 1 : 0.001}
            transition={fade}
            onEvent={{ loaded: () => { if (front() === index) setLoaded(index); } }}
          />
        )}
      </Index>
      <view width={WIDTH} height={HEIGHT} color={0x07080b00 + (ui.backdropDim ? 0x99 : 0)} transition={fade} />
      <view width={WIDTH} height={HEIGHT} linearGradient={gradient([0x07080bfa, 0x07080bd9, 0x07080b73, 0x07080b00], toward.right, [0, 0.35, 0.62, 0.9])} />
      <view width={WIDTH} height={HEIGHT} linearGradient={gradient([0x07080bff, 0x07080bcc, 0x07080b00], toward.up, [0, 0.28, 0.62])} />
    </view>
  );
}
