// SPDX-License-Identifier: AGPL-3.0-only

import { VirtualGrid } from "@solidtv/solid/primitives";
import type { NodeProps } from "@solidtv/solid";
import type { Work } from "$lib/types.js";
import WorkCard, { POSTER } from "$components/WorkCard.js";

const GAP = 36;

/** Only what's near the viewport is ever built, so a library of thousands costs what a screenful does. */
export default function WorkGrid(props: NodeProps & { items: Work[]; columns?: number; onEndReached?: () => void }) {
  const columns = () => props.columns ?? 6;
  return (
    <VirtualGrid
      {...props}
      width={columns() * (POSTER.width + GAP)}
      columns={columns()}
      rows={2}
      buffer={2}
      gap={GAP}
      scroll="always"
      each={props.items}
      onEndReached={props.onEndReached}
      onEndReachedThreshold={columns() * 2}
    >
      {(work) => <WorkCard work={work()} />}
    </VirtualGrid>
  );
}
