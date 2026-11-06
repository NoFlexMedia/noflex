// SPDX-License-Identifier: AGPL-3.0-only

import { Row } from "@solidtv/solid/primitives";
import { For } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import type { Work } from "$lib/types.js";
import { GUTTER, PAGE_WIDTH, type, weight } from "../theme.js";
import WorkCard, { POSTER, WIDE } from "$components/WorkCard.js";

const HEADING = 72;
export const railHeight = (wide?: boolean) => HEADING + (wide ? WIDE.total : POSTER.total) + 48;

export default function Rail(props: NodeProps & { title: string; items: Work[]; wide?: boolean; onChosen?: (work: Work) => void }) {
  return (
    <view {...props} width={PAGE_WIDTH} height={railHeight(props.wide)} forwardFocus={1}>
      <text height={56} lineHeight={56} fontSize={type.lead} fontWeight={weight.semibold}>{props.title}</text>
      <Row y={HEADING} width={PAGE_WIDTH - GUTTER} height={props.wide ? WIDE.total : POSTER.total} gap={32} scroll="auto">
        <For each={props.items}>{(work) => <WorkCard work={work} wide={props.wide} onChosen={props.onChosen} />}</For>
      </Row>
    </view>
  );
}
