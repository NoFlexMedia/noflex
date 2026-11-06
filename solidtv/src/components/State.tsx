// SPDX-License-Identifier: AGPL-3.0-only
//
// Every state is designed: what happened, and the next step.

import { Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import type { IconName } from "$lib/icons.generated.js";
import { color, radius, SCREEN, type, weight } from "../theme.js";
import Button from "$components/Button.js";
import Icon from "$components/Icon.js";

interface StateProps extends NodeProps {
  kind?: "empty" | "error";
  icon?: IconName;
  title: string;
  message?: string;
  action?: () => void;
  actionLabel?: string;
  /** The space to center in, when it isn't the whole screen. */
  areaWidth?: number;
}

export default function State(props: StateProps) {
  const error = () => props.kind === "error";
  const area = () => props.areaWidth ?? SCREEN.width;
  const WIDTH = 900;
  const x = () => (area() - WIDTH) / 2;
  const actionLabel = () => props.actionLabel ?? "Try again";
  return (
    <view {...props} x={x()} y={SCREEN.height / 2 - 220} width={WIDTH} height={440} forwardFocus={3}>
      <view x={WIDTH / 2 - 56} width={112} height={112} borderRadius={radius.xl} color={error() ? color.dangerSoft : color.surface2}>
        <Icon name={props.icon ?? (error() ? "warning-circle" : "sparkle")} size={56} x={28} y={28} color={error() ? color.danger : color.text2} />
      </view>
      <text y={140} width={WIDTH} contain="width" textAlign="center" fontSize={type.title} fontWeight={weight.semibold}>{props.title}</text>
      <text y={204} width={WIDTH} contain="width" textAlign="center" maxLines={3} lineHeight={42} fontSize={type.body} color={color.text2}>{props.message ?? ""}</text>
      <Show when={props.action} fallback={<view skipFocus />}>
        <Button y={340} x={(WIDTH - 280) / 2} width={280} label={actionLabel()} kind="primary" onEnter={() => { props.action?.(); return true; }} />
      </Show>
    </view>
  );
}
