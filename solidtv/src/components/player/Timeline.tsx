// SPDX-License-Identifier: AGPL-3.0-only
//
// The web player's scrub rail: what's buffered, what's played, where chapters fall and
// where the intro runs. The knob shows while the rail is what Left and Right move.

import { For, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { color, motion } from "../../theme.js";

const HIT = 56;
const REST = 8;
const ACTIVE = 14;
const KNOB = 28;

interface TimelineProps extends NodeProps {
  width: number;
  played: number;
  buffered: number;
  /** Fractions of the length where a chapter starts. */
  marks: number[];
  intro?: { from: number; to: number } | null;
  active: boolean;
}

export default function Timeline(props: TimelineProps) {
  const height = () => (props.active ? ACTIVE : REST);
  const top = () => (HIT - height()) / 2;
  return (
    <view {...props} width={props.width} height={HIT} skipFocus>
      <view y={top()} width={props.width} height={height()} borderRadius={height() / 2} color={0xffffff2e} transition={{ y: motion.fast, height: motion.fast }}>
        <view width={props.width * props.buffered} height={height()} borderRadius={height() / 2} color={0xffffff47} transition={{ height: motion.fast }} />
        <Show when={props.intro}>
          {(zone) => <view x={props.width * zone().from} width={props.width * (zone().to - zone().from)} height={height()} color={0xffffff33} />}
        </Show>
        <view width={props.width * props.played} height={height()} borderRadius={height() / 2} color={color.accent} transition={{ height: motion.fast }} />
        <For each={props.marks}>
          {(mark) => <view x={props.width * mark - 1.5} width={3} height={height()} color={0x000000a6} />}
        </For>
      </view>
      <view x={props.width * props.played - KNOB / 2} y={(HIT - KNOB) / 2} width={KNOB} height={KNOB} borderRadius={KNOB / 2} color={color.light} alpha={props.active ? 1 : 0} transition={{ alpha: motion.fast }} />
    </view>
  );
}
