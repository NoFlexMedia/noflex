// SPDX-License-Identifier: AGPL-3.0-only
//
// First launch: the web client is served by its server, a TV has to be told where it is.

import { createSignal } from "solid-js";
import { connect } from "$lib/session.js";
import { attempt } from "$lib/ui.js";
import { color, SCREEN, type, weight } from "../theme.js";
import BrandMark from "$components/BrandMark.js";
import Keyboard, { KEYBOARD_HEIGHT, KEYBOARD_WIDTH } from "$components/Keyboard.js";
import TextField from "$components/TextField.js";

const LEFT = 140;
const FIELD = 720;

export default function Connect() {
  const [address, setAddress] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const submit = async () => {
    if (busy() || !address().trim()) return;
    setBusy(true);
    await attempt(() => connect(address()));
    setBusy(false);
  };

  return (
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={4}>
      <BrandMark x={LEFT} y={200} size={96} />
      <text x={LEFT} y={330} fontSize={type.heading} fontWeight={weight.bold}>Find your server</text>
      <text x={LEFT} y={416} width={FIELD} contain="width" lineHeight={42} fontSize={type.body} color={color.text2}>The address you use to reach NoFlex from a browser. The port is 8096 unless you say otherwise.</text>
      <TextField x={LEFT} y={540} width={FIELD} height={88} fontSize={type.lead} value={address()} placeholder={busy() ? "Looking…" : "192.168.1.20 or noflex.example.com"} active skipFocus />
      <Keyboard autofocus x={SCREEN.width - KEYBOARD_WIDTH - 120} y={(SCREEN.height - KEYBOARD_HEIGHT) / 2} value={address()} onChange={setAddress} submitLabel="Connect" onSubmit={submit} />
    </view>
  );
}
