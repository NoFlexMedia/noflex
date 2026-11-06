// SPDX-License-Identifier: AGPL-3.0-only
//
// A destination: its heading stays put, and what's under it scrolls and clips below.

import type { JSX } from "solid-js";
import { PAGE_WIDTH, PAGE_X, SCREEN, type, weight } from "../theme.js";

export const BODY_TOP = 136;
/** Room inside the clip for a focused card to lift into. */
export const BODY_INSET = 28;
export const BODY_HEIGHT = SCREEN.height - BODY_TOP;

export default function Page(props: { title: string; children: JSX.Element }) {
  return (
    <view x={PAGE_X} width={PAGE_WIDTH} height={SCREEN.height} forwardFocus={1}>
      <text y={48} fontSize={type.heading} fontWeight={weight.bold}>{props.title}</text>
      <view y={BODY_TOP} x={-BODY_INSET} width={PAGE_WIDTH + BODY_INSET} height={BODY_HEIGHT} clipping forwardFocus={0}>
        <view x={BODY_INSET} y={BODY_INSET} width={PAGE_WIDTH} height={BODY_HEIGHT - BODY_INSET} forwardFocus={0} children={props.children} />
      </view>
    </view>
  );
}
