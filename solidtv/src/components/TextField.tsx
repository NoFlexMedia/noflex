// SPDX-License-Identifier: AGPL-3.0-only
//
// What the Keyboard is typing into, with the caret where the next letter lands.
// Input is always at the end, so a long value slides left to keep the caret in view.

import { createSignal, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import type { IconName } from "$lib/icons.generated.js";
import { color, radius, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

const CARET = 3;

interface TextFieldProps extends NodeProps {
  value: string;
  placeholder?: string;
  secret?: boolean;
  /** Where typing goes right now. */
  active: boolean;
  width: number;
  height: number;
  fontSize?: number;
  icon?: IconName;
}

export default function TextField(props: TextFieldProps) {
  const [measured, setMeasured] = createSignal(0);
  const pad = () => (props.icon ? 80 : 24);
  const inner = () => props.width - pad() - 24;
  const shown = () => (props.secret ? "•".repeat(props.value.length) : props.value);
  const size = () => props.fontSize ?? type.body;
  const textWidth = () => (shown() ? measured() : 0);
  const slide = () => Math.min(0, inner() - textWidth() - CARET);
  return (
    <view {...props} borderRadius={radius.m} color={color.surface2} border={{ width: 3, color: props.active ? color.focus : color.line }}>
      <Show when={props.icon}>
        <Icon name={props.icon!} size={36} x={26} y={(props.height - 36) / 2} color={color.text3} />
      </Show>
      <view x={pad()} width={inner()} height={props.height} clipping>
        <Show when={shown()} fallback={<text height={props.height} lineHeight={props.height} fontSize={size()} color={color.text3}>{props.placeholder ?? ""}</text>}>
          <text
            x={slide()}
            height={props.height}
            lineHeight={props.height}
            fontSize={size()}
            fontWeight={weight.regular}
            onEvent={{ loaded: (_node, event) => { if (event) setMeasured(event.dimensions.w); } }}
          >
            {shown()}
          </text>
        </Show>
        <Show when={props.active}>
          <view x={slide() + textWidth() + 1} y={(props.height - size() * 1.25) / 2} width={CARET} height={size() * 1.25} borderRadius={1} color={color.accent} />
        </Show>
      </view>
    </view>
  );
}
