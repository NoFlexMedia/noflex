// SPDX-License-Identifier: AGPL-3.0-only
//
// A profile's PIN, on the keys a remote has. Digits only, so a remote with number
// keys can type it without the pad, as a keyboard can.

import { makeEventListener } from "@solid-primitives/event-listener";
import { Column, Row } from "@solidtv/solid/primitives";
import { createSignal, For, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import type { IconName } from "$lib/icons.generated.js";
import { color, motion, radius, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

/** The server's bounds: a PIN is 4 to 6 digits. */
export const PIN_MIN = 4;
export const PIN_MAX = 6;

const KEY = { width: 132, height: 96 };
const GAP = 12;
export const PIN_PAD_WIDTH = KEY.width * 3 + GAP * 2;
export const PIN_PAD_HEIGHT = KEY.height * 4 + GAP * 3;
const ROWS = ["123", "456", "789"];

function Key(props: NodeProps & { label?: string; icon?: IconName; primary?: boolean }) {
  const [focused, setFocused] = createSignal(false);
  // White means focused, and nothing else: the key that submits rests in the accent.
  const fill = () => (focused() ? color.light : props.primary ? color.accent : color.surface2);
  const ink = () => (focused() ? color.lightInk : props.primary ? color.accentInk : color.text);
  return (
    <view
      {...props}
      width={KEY.width}
      height={KEY.height}
      borderRadius={radius.m}
      color={fill()}
      scale={focused() ? 1.06 : 1}
      zIndex={focused() ? 2 : 1}
      transition={{ scale: motion.fast, color: motion.fast }}
      onFocusChanged={setFocused}
    >
      <Show when={props.icon}>
        <Icon name={props.icon!} size={40} x={(KEY.width - 40) / 2} y={(KEY.height - 40) / 2} color={ink()} />
      </Show>
      <Show when={props.label}>
        <text width={KEY.width} height={KEY.height} lineHeight={KEY.height} contain="both" textAlign="center" fontSize={type.title} fontWeight={weight.semibold} color={ink()}>{props.label}</text>
      </Show>
    </view>
  );
}

interface PinPadProps extends NodeProps {
  value: string;
  onChange: (value: string) => void;
  /** Called with a PIN long enough to be one. */
  onSubmit: () => void;
}

export default function PinPad(props: PinPadProps) {
  const type_ = (digit: string) => { if (props.value.length < PIN_MAX) props.onChange(props.value + digit); };
  const erase = () => props.onChange(props.value.slice(0, -1));
  const ready = () => props.value.length >= PIN_MIN;
  const submit = () => { if (ready()) props.onSubmit(); };

  // Number keys, on a remote or a keyboard. In the capture phase, as in `Keyboard`:
  // spoken for before focus treats Backspace as Back or Enter as the focused key.
  let typing = false;
  const claim = (event: KeyboardEvent) => { event.preventDefault(); event.stopPropagation(); };
  makeEventListener(document as unknown as EventTarget, "keydown", ((event: KeyboardEvent) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const key = event.key;
    if (key?.length === 1 && key >= "0" && key <= "9") {
      typing = true;
      type_(key);
      claim(event);
    } else if (key === "Backspace" || key === "Delete") {
      // The Siri Remote's Menu arrives as Backspace. An empty field lets it be Back.
      if (__TARGET__ !== "nativescript" && props.value) { typing = true; erase(); claim(event); }
    } else if (key === "Enter") {
      if (typing && ready()) { props.onSubmit(); claim(event); }
    } else {
      typing = false;
    }
  }) as EventListener, { capture: true });

  return (
    <Column {...props} width={PIN_PAD_WIDTH} height={PIN_PAD_HEIGHT} gap={GAP} scroll="none">
      <For each={ROWS}>
        {(row) => (
          <Row width={PIN_PAD_WIDTH} height={KEY.height} gap={GAP} scroll="none">
            <For each={row.split("")}>{(digit) => <Key label={digit} onEnter={() => { type_(digit); return true; }} />}</For>
          </Row>
        )}
      </For>
      <Row width={PIN_PAD_WIDTH} height={KEY.height} gap={GAP} scroll="none">
        <Key icon="backspace" onEnter={() => { erase(); return true; }} />
        <Key label="0" onEnter={() => { type_("0"); return true; }} />
        <Key icon="check" primary={ready()} onEnter={() => { submit(); return true; }} />
      </Row>
    </Column>
  );
}
