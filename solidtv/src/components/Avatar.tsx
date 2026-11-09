// SPDX-License-Identifier: AGPL-3.0-only
//
// Someone's picture, or their initials: until they choose one, and whenever it won't load.

import { createEffect, createSignal, on, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { mediaUrl } from "$lib/tokens.js";
import { color, weight } from "../theme.js";

/** "Vera Viewer" is VV, "vera" is VE, as on the other clients. */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words[0]!.charAt(0) + words[words.length - 1]!.charAt(0) : (words[0] ?? "").slice(0, 2);
  return letters.toUpperCase();
}

export default function Avatar(props: NodeProps & { name: string; url?: string | null; size: number; dim?: boolean }) {
  const [failed, setFailed] = createSignal(false);
  createEffect(on(() => props.url, () => setFailed(false), { defer: true }));
  return (
    <view {...props} width={props.size} height={props.size} borderRadius={props.size / 2} color={props.dim ? color.surface3 : color.accent}>
      <Show
        when={failed() ? null : props.url}
        fallback={<text width={props.size} height={props.size} lineHeight={props.size} contain="both" textAlign="center" fontSize={Math.round(props.size / 3)} fontWeight={weight.bold} color={props.dim ? color.text2 : color.accentInk}>{initials(props.name)}</text>}
      >
        {(url) => <view width={props.size} height={props.size} borderRadius={props.size / 2} src={mediaUrl(url())} alpha={props.dim ? 0.45 : 1} onEvent={{ failed: () => setFailed(true) }} />}
      </Show>
    </view>
  );
}
