// SPDX-License-Identifier: AGPL-3.0-only

import { createSignal, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { color, motion, radius, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

const SWITCH = { width: 68, height: 38, knob: 28 };

interface SettingRowProps extends NodeProps {
  width: number;
  label: string;
  detail?: string;
  /** What's chosen, for a row that opens a list. */
  value?: string;
  /** On or off, for a row Enter flips. */
  checked?: boolean;
}

export const SETTING_ROW = 100;

export default function SettingRow(props: SettingRowProps) {
  const [focused, setFocused] = createSignal(false);
  const ink = () => (focused() ? color.lightInk : color.text);
  const dim = () => (focused() ? 0x0b0c0fb3 : color.text3);
  const switched = () => props.checked !== undefined;
  const textWidth = () => props.width - (switched() ? 170 : 460);
  return (
    <view
      {...props}
      width={props.width}
      height={SETTING_ROW}
      borderRadius={radius.m}
      color={focused() ? color.light : color.surface}
      transition={{ color: motion.fast }}
      onFocusChanged={setFocused}
    >
      <text x={28} y={props.detail ? 16 : 34} width={textWidth()} contain="width" maxLines={1} fontSize={type.caption} fontWeight={weight.semibold} color={ink()}>{props.label}</text>
      <Show when={props.detail}>
        <text x={28} y={54} width={textWidth()} contain="width" maxLines={1} fontSize={21} color={dim()}>{props.detail}</text>
      </Show>
      <Show when={switched()}>
        <view x={props.width - SWITCH.width - 28} y={(SETTING_ROW - SWITCH.height) / 2} width={SWITCH.width} height={SWITCH.height} borderRadius={SWITCH.height / 2} color={props.checked ? color.accent : focused() ? 0x0b0c0f33 : color.surface3} transition={{ color: motion.fast }}>
          <view x={props.checked ? SWITCH.width - SWITCH.knob - 5 : 5} y={5} width={SWITCH.knob} height={SWITCH.knob} borderRadius={SWITCH.knob / 2} color={props.checked ? color.accentInk : focused() ? color.lightInk : color.text} transition={{ x: motion.fast }} />
        </view>
      </Show>
      <Show when={!switched() && props.value !== undefined}>
        <text x={props.width - 420} y={34} width={350} contain="width" maxLines={1} textAlign="right" fontSize={type.caption} color={focused() ? 0x0b0c0fcc : color.text2}>{props.value}</text>
        <Icon name="caret-right" size={26} x={props.width - 54} y={37} color={dim()} />
      </Show>
    </view>
  );
}
