// SPDX-License-Identifier: AGPL-3.0-only

import { createSignal, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import type { IconName } from "$lib/icons.generated.js";
import { color, motion, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

export type ButtonKind = "primary" | "light" | "glass" | "danger";

interface ButtonProps extends NodeProps {
  label?: string;
  icon?: IconName;
  kind?: ButtonKind;
  size?: "md" | "lg";
  busy?: boolean;
  /** Width isn't measured from text ahead of the font loading, so a button states its own. */
  width?: number;
}

const FILL: Record<ButtonKind, [number, number]> = {
  primary: [color.accent, color.accentInk],
  light: [color.light, color.lightInk],
  glass: [0xffffff24, color.text],
  danger: [color.dangerSoft, color.danger],
};

/** Roughly what a label needs at Inter's average advance; good enough for a pill with padding. */
export const labelWidth = (label: string, fontSize: number) => Math.ceil(label.length * fontSize * 0.56);

export default function Button(props: ButtonProps) {
  const [focused, setFocused] = createSignal(false);
  const kind = () => props.kind ?? "glass";
  const height = () => (props.size === "lg" ? 76 : 64);
  const fontSize = () => (props.size === "lg" ? type.body : type.caption);
  const iconSize = () => (props.size === "lg" ? 34 : 30);
  const pad = () => (props.size === "lg" ? 34 : 26);
  const hasLabel = () => !!props.label;
  const width = () => props.width ?? (hasLabel()
    ? pad() * 2 + labelWidth(props.label!, fontSize()) + (props.icon ? iconSize() + 14 : 0)
    : height());
  const fill = () => (focused() ? (kind() === "primary" ? color.accent : color.light) : FILL[kind()][0]);
  const ink = () => (focused() ? (kind() === "primary" ? color.accentInk : color.lightInk) : FILL[kind()][1]);
  const iconX = () => (hasLabel() ? pad() : (height() - iconSize()) / 2);

  return (
    <view
      {...props}
      width={width()}
      height={height()}
      color={fill()}
      borderRadius={height() / 2}
      scale={focused() ? 1.06 : 1}
      transition={{ scale: motion.spring, color: motion.fast }}
      onFocusChanged={setFocused}
    >
      <Show when={props.icon}>
        <Icon name={props.icon!} size={iconSize()} x={iconX()} y={(height() - iconSize()) / 2} color={ink()} alpha={props.busy ? 0.4 : 1} />
      </Show>
      <Show when={hasLabel()}>
        <text
          x={props.icon ? pad() + iconSize() + 14 : 0}
          y={0}
          width={props.icon ? width() - pad() - iconSize() - 14 : width()}
          height={height()}
          lineHeight={height()}
          contain="both"
          maxLines={1}
          textAlign={props.icon ? "left" : "center"}
          verticalAlign="middle"
          fontSize={fontSize()}
          fontWeight={weight.semibold}
          color={ink()}
        >
          {props.label}
        </text>
      </Show>
    </view>
  );
}

