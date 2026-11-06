// SPDX-License-Identifier: AGPL-3.0-only
//
// Holds the content's shape while it loads. No shimmer: a moving highlight keeps
// the renderer drawing, and pages here paint from cache often enough not to need it.

import { For } from "solid-js";
import { color, GUTTER, PAGE_X, radius, SCREEN } from "../theme.js";
import { POSTER } from "$components/WorkCard.js";

export default function Skeleton(props: { y?: number }) {
  const count = Math.floor((SCREEN.width - PAGE_X - GUTTER) / (POSTER.width + 32));
  return (
    <view y={props.y ?? 120} skipFocus>
      <view width={320} height={36} borderRadius={radius.s} color={color.surface2} />
      <For each={Array.from({ length: count }, (_, index) => index)}>
        {(index) => <view x={index * (POSTER.width + 32)} y={72} width={POSTER.width} height={POSTER.height} borderRadius={radius.m} color={color.surface2} />}
      </For>
    </view>
  );
}
