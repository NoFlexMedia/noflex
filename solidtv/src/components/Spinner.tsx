// SPDX-License-Identifier: AGPL-3.0-only
//
// Shown only after a beat, so instant work shows none. Mounted only while waiting:
// a running animation keeps the renderer drawing every frame.

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import { color } from "../theme.js";
import Icon from "$components/Icon.js";

const DELAY_MS = 350;

export default function Spinner(props: NodeProps & { size?: number }) {
  const [shown, setShown] = createSignal(false);
  const timer = setTimeout(() => setShown(true), DELAY_MS);
  onCleanup(() => clearTimeout(timer));
  const start = (node: ElementNode) => {
    onMount(() => { node.animate({ rotation: Math.PI * 2 }, { duration: 900, easing: "linear", loop: true }).start(); });
  };
  return (
    <Show when={shown()}>
      <Icon {...props} ref={start} name="circle-notch" size={props.size ?? 72} color={color.text} skipFocus />
    </Show>
  );
}
