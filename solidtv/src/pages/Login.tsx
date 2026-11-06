// SPDX-License-Identifier: AGPL-3.0-only

import { Column } from "@solidtv/solid/primitives";
import { createSignal, Show } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import { app, changeServer, signIn } from "$lib/session.js";
import { serverOrigin } from "$lib/tokens.js";
import { attempt } from "$lib/ui.js";
import { color, SCREEN, type, weight } from "../theme.js";
import BrandMark from "$components/BrandMark.js";
import Button from "$components/Button.js";
import Keyboard, { KEYBOARD_HEIGHT, KEYBOARD_WIDTH } from "$components/Keyboard.js";
import TextField from "$components/TextField.js";

const LEFT = 140;
const FIELD = 720;

function Field(props: NodeProps & { label: string; value: string; secret?: boolean; active: boolean }) {
  return (
    <view {...props} width={FIELD} height={112}>
      <text fontSize={type.caption} fontWeight={weight.semibold} color={color.text2}>{props.label}</text>
      <TextField y={36} width={FIELD} height={76} value={props.value} secret={props.secret} active={props.active} skipFocus />
    </view>
  );
}

export default function Login() {
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [active, setActive] = createSignal<"username" | "password">("username");
  const [busy, setBusy] = createSignal(false);
  let keyboard: ElementNode | undefined;

  const submit = async () => {
    if (busy()) return;
    if (!username()) { setActive("username"); return; }
    if (!password() && active() === "username") { setActive("password"); return; }
    setBusy(true);
    await attempt(() => signIn(username(), password()));
    setBusy(false);
  };
  const edit = (field: "username" | "password") => { setActive(field); keyboard?.setFocus(); return true; };

  return (
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={5}>
      <BrandMark x={LEFT} y={110} size={80} />
      <text x={LEFT} y={214} fontSize={type.heading} fontWeight={weight.bold}>Sign in</text>
      <text x={LEFT} y={290} width={FIELD} contain="width" maxLines={1} fontSize={type.caption} color={color.text3}>{app.server?.name ? `${app.server.name} · ${serverOrigin()}` : serverOrigin() ?? ""}</text>
      <Show when={app.authError} fallback={<view skipFocus />}>
        <text x={LEFT} y={330} width={FIELD} contain="width" maxLines={2} fontSize={type.caption} color={color.danger}>{app.authError}</text>
      </Show>
      <Column x={LEFT} y={400} width={FIELD} height={520} gap={24} scroll="none" onRight={() => { keyboard?.setFocus(); return true; }}>
        <Field label="Username" value={username()} active={active() === "username"} onEnter={() => edit("username")} />
        <Field label="Password" value={password()} secret active={active() === "password"} onEnter={() => edit("password")} />
        <Button width={280} label="Change server" onEnter={() => { changeServer(); return true; }} />
      </Column>
      <Keyboard
        ref={(node: ElementNode) => { keyboard = node; }}
        autofocus
        x={SCREEN.width - KEYBOARD_WIDTH - 120}
        y={(SCREEN.height - KEYBOARD_HEIGHT) / 2}
        value={active() === "username" ? username() : password()}
        onChange={(value) => (active() === "username" ? setUsername(value) : setPassword(value))}
        submitLabel={busy() ? "Signing in…" : active() === "username" ? "Next" : "Sign in"}
        onSubmit={submit}
        onTab={() => setActive(active() === "username" ? "password" : "username")}
      />
    </view>
  );
}
