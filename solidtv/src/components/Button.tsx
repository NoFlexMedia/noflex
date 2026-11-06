// SPDX-License-Identifier: AGPL-3.0-only

import { createEffect, createSignal, on, Show } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import type { IconName } from "$lib/icons.generated.js";
import { color, motion, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

export type ButtonKind = "primary" | "glass" | "danger";

interface ButtonProps extends NodeProps {
  label?: string;
  icon?: IconName;
  kind?: ButtonKind;
  size?: "md" | "lg";
  busy?: boolean;
  /** What focus on this button means, for whatever says so nearby. */
  onFocused?: (has: boolean) => void;
  /** Fixes the width, and ellipsizes a label that won't fit. Otherwise the button fits its label. */
  width?: number;
}

const FILL: Record<ButtonKind, [number, number]> = {
  primary: [color.accent, color.accentInk],
  glass: [0xffffff24, color.text],
  danger: [color.dangerSoft, color.danger],
};

/** Roughly what a label needs at Inter's average advance, until the text has measured itself. */
export const labelWidth = (label: string, fontSize: number) => Math.ceil(label.length * fontSize * 0.56);

export default function Button(props: ButtonProps) {
  const [focused, setFocused] = createSignal(false);
  const kind = () => props.kind ?? "glass";
  const height = () => (props.size === "lg" ? 76 : 64);
  const fontSize = () => (props.size === "lg" ? type.body : type.caption);
  const iconSize = () => (props.size === "lg" ? 34 : 30);
  const pad = () => (props.size === "lg" ? 34 : 26);
  const hasLabel = () => !!props.label;
  const [measured, setMeasured] = createSignal(0);
  const width = () => props.width ?? (hasLabel()
    ? pad() * 2 + (measured() || labelWidth(props.label!, fontSize())) + (props.icon ? iconSize() + 14 : 0)
    : height());
  // White means focused, and nothing else: anything at rest wearing it reads as where you are.
  const fill = () => (focused() ? color.light : FILL[kind()][0]);
  const ink = () => (focused() ? color.lightInk : FILL[kind()][1]);
  const iconX = () => (hasLabel() ? pad() : (height() - iconSize()) / 2);

  // A flex row lays out when its children change, not when one changes width.
  let node: ElementNode | undefined;
  createEffect(on(width, () => { node?.parent?.updateLayout(); }, { defer: true }));

  return (
    <view
      {...props}
      ref={(element: ElementNode) => { node = element; (props.ref as ((element: ElementNode) => void) | undefined)?.(element); }}
      width={width()}
      height={height()}
      color={fill()}
      borderRadius={height() / 2}
      scale={focused() ? 1.06 : 1}
      transition={{ scale: motion.spring, color: motion.fast }}
      onFocusChanged={(has: boolean) => { setFocused(has); props.onFocused?.(has); }}
    >
      <Show when={props.icon}>
        <Icon name={props.icon!} size={iconSize()} x={iconX()} y={(height() - iconSize()) / 2} color={ink()} alpha={props.busy ? 0.4 : 1} />
      </Show>
      <Show when={hasLabel()}>
        <text
          x={props.icon ? pad() + iconSize() + 14 : props.width ? 0 : pad()}
          y={0}
          width={props.width ? (props.icon ? props.width - pad() - iconSize() - 14 : props.width) : undefined}
          height={height()}
          lineHeight={height()}
          contain={props.width ? "both" : undefined}
          maxLines={props.width ? 1 : undefined}
          textAlign={props.width && !props.icon ? "center" : undefined}
          verticalAlign="middle"
          fontSize={fontSize()}
          fontWeight={weight.semibold}
          color={ink()}
          onEvent={{ loaded: (_node, event) => { if (event) setMeasured(event.dimensions.w); } }}
        >
          {props.label}
        </text>
      </Show>
    </view>
  );
}

