// SPDX-License-Identifier: AGPL-3.0-only
//
// Down from the controls: the title's chapters as pictures to jump to.

import { Row } from "@solidtv/solid/primitives";
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import { timecode } from "$lib/format.js";
import { revealInRow } from "$lib/rows.js";
import { thumbAt, type Stop } from "$lib/player/trickplay.js";
import type { Trickplay } from "$lib/types.js";
import { color, motion, radius, type, weight } from "../../theme.js";
import Icon from "$components/Icon.js";

export const CHAPTER_WIDTH = 320;
export const CHAPTER_THUMB = 180;
export const CHAPTER_HEIGHT = CHAPTER_THUMB + 76;
const RING = 6;

function ChapterCard(props: NodeProps & { stop: Stop; label: string; trick?: Trickplay | null; current: boolean; onFocused: () => void; onChosen: () => void }) {
  const [focused, setFocused] = createSignal(false);
  // Made once: each is a tile of a sheet the renderer only loads while this is on screen.
  const texture = thumbAt(props.trick, props.stop.positionMs);
  let node: ElementNode | undefined;
  let revealing: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(revealing));
  return (
    <view
      {...props}
      ref={(element: ElementNode) => { node = element; }}
      width={CHAPTER_WIDTH}
      height={CHAPTER_HEIGHT}
      scale={focused() ? 1.05 : 1}
      zIndex={focused() ? 5 : 1}
      transition={{ scale: motion.spring }}
      onFocusChanged={(has: boolean) => {
        setFocused(has);
        clearTimeout(revealing);
        if (!has) return;
        props.onFocused();
        revealing = setTimeout(() => { if (node) revealInRow(node); }, 0);
      }}
      onEnter={() => { props.onChosen(); return true; }}
    >
      <view width={CHAPTER_WIDTH} height={CHAPTER_THUMB} borderRadius={radius.m} color={color.surface3}>
        <Show when={texture} fallback={<Icon name="film-strip" size={48} x={(CHAPTER_WIDTH - 48) / 2} y={(CHAPTER_THUMB - 48) / 2} color={color.text3} />}>
          <view width={CHAPTER_WIDTH} height={CHAPTER_THUMB} borderRadius={radius.m} texture={texture!} />
        </Show>
      </view>
      <view x={-RING} y={-RING} width={CHAPTER_WIDTH + RING * 2} height={CHAPTER_THUMB + RING * 2} color={color.clear} borderRadius={radius.m + RING} border={{ width: 4, color: focused() ? color.focus : props.current ? color.accent : color.clear }} />
      <text y={CHAPTER_THUMB + 12} width={CHAPTER_WIDTH} contain="width" maxLines={1} fontSize={type.caption} fontWeight={weight.semibold} color={focused() ? color.text : color.text2}>{props.label}</text>
      <text y={CHAPTER_THUMB + 46} fontSize={22} color={color.text3}>{timecode(props.stop.positionMs)}</text>
    </view>
  );
}

export default function ChapterRail(props: NodeProps & { width: number; stops: Stop[]; scenes: boolean; trick?: Trickplay | null; position: number; onFocused: (stop: Stop) => void; onChosen: (stop: Stop) => void }) {
  const current = () => {
    let found = 0;
    props.stops.forEach((stop, index) => { if (stop.positionMs <= props.position * 1000) found = index; });
    return found;
  };
  const start = current();
  let row: ElementNode | undefined;
  onMount(() => row?.setFocus());
  return (
    // Left and Right off its ends mustn't reach the scrub handlers around it.
    <view {...props} width={props.width} height={CHAPTER_HEIGHT} forwardFocus={0} onLeft={() => true} onRight={() => true}>
      <Row ref={(node: ElementNode) => { row = node; }} width={props.width} height={CHAPTER_HEIGHT} gap={24} scroll="auto" selected={start}>
        <For each={props.stops}>
          {(stop, index) => (
            <ChapterCard
              stop={stop}
              label={stop.title || `${props.scenes ? "Scene" : "Chapter"} ${index() + 1}`}
              trick={props.trick}
              current={index() === current()}
              onFocused={() => props.onFocused(stop)}
              onChosen={() => props.onChosen(stop)}
            />
          )}
        </For>
      </Row>
    </view>
  );
}
