// SPDX-License-Identifier: AGPL-3.0-only
//
// A panel of choices from the right: versions, audio, subtitles. Takes focus while
// it's open and gives it back to whatever opened it.

import { Column } from "@solidtv/solid/primitives";
import { activeElement, type ElementNode } from "@solidtv/solid";
import { createSignal, For, onCleanup, onMount, Show, untrack } from "solid-js";
import type { Choice } from "$lib/tracks.js";
import { color, motion, radius, SCREEN, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

const WIDTH = 720;
const ROW = 92;

function Option(props: { choice: Choice; selected: boolean; onEnter: () => boolean }) {
  const [focused, setFocused] = createSignal(false);
  const hint = () => props.choice.hint ?? "";
  return (
    <view
      width={WIDTH - 80}
      height={ROW}
      borderRadius={radius.m}
      color={focused() ? color.light : color.clear}
      transition={{ color: motion.fast }}
      onFocusChanged={setFocused}
      onEnter={props.onEnter}
    >
      <text x={24} y={hint() ? 12 : 26} width={WIDTH - 180} contain="width" maxLines={1} fontSize={type.caption} fontWeight={weight.semibold} color={focused() ? color.lightInk : color.text}>{props.choice.label}</text>
      <Show when={hint()}>
        <text x={24} y={50} width={WIDTH - 180} contain="width" maxLines={1} fontSize={22} color={focused() ? 0x0b0c0fb3 : color.text3}>{hint()}</text>
      </Show>
      <Show when={props.selected}>
        <Icon name="check" size={32} x={WIDTH - 80 - 56} y={30} color={focused() ? color.lightInk : color.accent} />
      </Show>
    </view>
  );
}

export default function Sheet(props: { title: string; choices: Choice[]; value: string; onPick: (value: string) => void; onClose: () => void }) {
  let list: ElementNode | undefined;
  const previous = untrack(activeElement) as ElementNode | undefined;
  const selected = Math.max(0, props.choices.findIndex((choice) => choice.value === props.value));
  onMount(() => list?.setFocus());
  onCleanup(() => { if (previous && !previous.destroyed) previous.setFocus(); });
  return (
    <view width={SCREEN.width} height={SCREEN.height} zIndex={150} onBack={() => { props.onClose(); return true; }} onLeft={() => true} onRight={() => true} onUp={() => true} onDown={() => true} forwardFocus={2}>
      {/* On these quads, not just a parent: only a quad with a zIndex draws over text queued before it. */}
      <view width={SCREEN.width} height={SCREEN.height} zIndex={150} color={0x030406b8} />
      <view x={SCREEN.width - WIDTH} width={WIDTH} height={SCREEN.height} zIndex={150} color={0x16181eff}>
        <text x={40} y={56} fontSize={type.lead} fontWeight={weight.semibold}>{props.title}</text>
      </view>
      <Column ref={(node: ElementNode) => { list = node; }} x={SCREEN.width - WIDTH + 40} y={128} width={WIDTH - 80} height={SCREEN.height - 184} zIndex={151} selected={selected} gap={4} scroll="auto" clipping>
        <For each={props.choices}>
          {(choice) => <Option choice={choice} selected={choice.value === props.value} onEnter={() => { props.onPick(choice.value); return true; }} />}
        </For>
      </Column>
    </view>
  );
}
