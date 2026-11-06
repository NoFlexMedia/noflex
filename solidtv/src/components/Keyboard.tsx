// SPDX-License-Identifier: AGPL-3.0-only
//
// The on-screen keyboard, for every field on every remote. A hardware keyboard
// works too while it's on screen, which is how you'd want to type a password if
// the TV has one.

import { makeEventListener } from "@solid-primitives/event-listener";
import { Column, Row } from "@solidtv/solid/primitives";
import { createSignal, For, Show } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import type { IconName } from "$lib/icons.generated.js";
import { color, motion, radius, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";

type Layer = "lower" | "upper" | "symbols";

const LAYERS: Record<Layer, string[]> = {
  lower: ["1234567890", "qwertyuiop", "asdfghjkl-", "zxcvbnm._@"],
  upper: ["1234567890", "QWERTYUIOP", "ASDFGHJKL-", "ZXCVBNM._@"],
  symbols: ["1234567890", "!@#$%^&*()", "/\\:;?=+'\"~", "[]{}<>,|`_"],
};

const KEY = 76;
const GAP = 8;
export const KEYBOARD_WIDTH = KEY * 10 + GAP * 9;
export const KEYBOARD_HEIGHT = (KEY + GAP) * 5;

function Key(props: NodeProps & { label?: string; icon?: IconName; span?: number; active?: boolean }) {
  const [focused, setFocused] = createSignal(false);
  const width = () => KEY * (props.span ?? 1) + GAP * ((props.span ?? 1) - 1);
  const ink = () => (focused() ? color.lightInk : props.active ? color.accent : color.text);
  return (
    <view
      {...props}
      width={width()}
      height={KEY}
      borderRadius={radius.s}
      color={focused() ? color.light : color.surface2}
      scale={focused() ? 1.08 : 1}
      zIndex={focused() ? 2 : 1}
      transition={{ scale: motion.fast, color: motion.fast }}
      onFocusChanged={setFocused}
    >
      <Show when={props.icon}>
        <Icon name={props.icon!} size={36} x={(width() - 36) / 2} y={(KEY - 36) / 2} color={ink()} />
      </Show>
      <Show when={props.label}>
        <text width={width()} height={KEY} lineHeight={KEY} contain="both" textAlign="center" fontSize={props.span ? type.caption : type.body} fontWeight={weight.semibold} color={ink()}>{props.label}</text>
      </Show>
    </view>
  );
}

interface KeyboardProps extends NodeProps {
  value: string;
  onChange: (value: string) => void;
  /** What the last key says, and does. */
  submitLabel?: string;
  onSubmit?: () => void;
}

export default function Keyboard(props: KeyboardProps) {
  const [layer, setLayer] = createSignal<Layer>("lower");
  const type_ = (text: string) => {
    props.onChange(props.value + text);
    if (layer() === "upper") setLayer("lower");
  };
  const erase = () => props.onChange(props.value.slice(0, -1));

  makeEventListener(document as unknown as EventTarget, "keydown", ((event: KeyboardEvent) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key?.length === 1) { props.onChange(props.value + event.key); event.preventDefault(); }
  }) as EventListener);

  return (
    <Column
      {...props}
      width={KEYBOARD_WIDTH}
      height={KEYBOARD_HEIGHT}
      gap={GAP}
      scroll="none"
      // Only in a browser is Backspace a keyboard's: the Siri Remote's Menu arrives under that name too.
      onBack={(event: KeyboardEvent) => {
        if (__TARGET__ !== "browser" || event?.key !== "Backspace" || !props.value) return false;
        erase();
        return true;
      }}
    >
      <For each={LAYERS[layer()]}>
        {(row) => (
          <Row width={KEYBOARD_WIDTH} height={KEY} gap={GAP} scroll="none">
            <For each={row.split("")}>{(letter) => <Key label={letter} onEnter={() => { type_(letter); return true; }} />}</For>
          </Row>
        )}
      </For>
      <Row width={KEYBOARD_WIDTH} height={KEY} gap={GAP} scroll="none">
        <Key icon="arrow-up" active={layer() === "upper"} onEnter={() => { setLayer(layer() === "upper" ? "lower" : "upper"); return true; }} />
        <Key label={layer() === "symbols" ? "abc" : "#+="} span={2} onEnter={() => { setLayer(layer() === "symbols" ? "lower" : "symbols"); return true; }} />
        <Key label="Space" span={3} onEnter={() => { type_(" "); return true; }} />
        <Key icon="backspace" onEnter={() => { erase(); return true; }} />
        <Show when={props.onSubmit} fallback={<Key label="Clear" span={3} onEnter={() => { props.onChange(""); return true; }} />}>
          <Key label={props.submitLabel ?? "Done"} span={3} active onEnter={() => { props.onSubmit?.(); return true; }} />
        </Show>
      </Row>
    </Column>
  );
}
