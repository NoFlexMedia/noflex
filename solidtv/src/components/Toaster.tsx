// SPDX-License-Identifier: AGPL-3.0-only

import { Show } from "solid-js";
import { ui } from "$lib/ui.js";
import { color, radius, SCREEN, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

const WIDTH = 840;

export default function Toaster() {
  const icon = () => (ui.toast?.kind === "error" ? "warning-circle" : ui.toast?.kind === "success" ? "check-circle" : "info");
  const tint = () => (ui.toast?.kind === "error" ? color.danger : ui.toast?.kind === "success" ? color.success : color.text2);
  return (
    <Show when={ui.toast}>
      <view x={SCREEN.width - WIDTH - 64} y={56} width={WIDTH} height={108} zIndex={200} color={0x16181eff} borderRadius={radius.l} border={{ width: 2, color: color.lineStrong }} skipFocus>
        <Icon name={icon()} size={40} x={32} y={34} color={tint()} />
        <text x={92} y={12} width={WIDTH - 124} height={84} contain="both" verticalAlign="middle" maxLines={2} lineHeight={36} fontSize={type.caption} fontWeight={weight.semibold}>{ui.toast?.message ?? ""}</text>
      </view>
    </Show>
  );
}
