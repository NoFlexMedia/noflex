// SPDX-License-Identifier: AGPL-3.0-only
//
// Signing in. A code to approve from a phone or computer comes first, because a
// password on a remote is the worst thing this app asks of anyone. The password is
// there for whoever has nothing else signed in.

import { Column, Row } from "@solidtv/solid/primitives";
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import { app, changeServer, checkDeviceCode, requestDeviceCode, signIn } from "$lib/session.js";
import { serverOrigin } from "$lib/tokens.js";
import { attempt } from "$lib/ui.js";
import { color, radius, SCREEN, type, weight } from "../theme.js";
import BrandMark from "$components/BrandMark.js";
import Button from "$components/Button.js";
import Keyboard, { KEYBOARD_HEIGHT, KEYBOARD_WIDTH } from "$components/Keyboard.js";
import QrCode from "$components/QrCode.js";
import TextField from "$components/TextField.js";

const LEFT = 140;
const FIELD = 720;
const POLL_MS = 3000;
const CODE_LIFE_MS = 10 * 60 * 1000;
// Half an hour of codes. After that a TV left on this screen stops asking the server.
const ROUNDS = 3;
const CODE_LENGTH = 6;
const CELL = { width: 112, height: 144, gap: 14 };
const GUIDE_X = LEFT + (CELL.width + CELL.gap) * CODE_LENGTH + 80;
const GUIDE_WIDTH = SCREEN.width - LEFT - GUIDE_X;
const QR_SIZE = 264;
const approveUrl = () => `${serverOrigin() ?? ""}/account?tab=devices`;

function Heading() {
  return (
    <>
      <BrandMark x={LEFT} y={110} size={80} />
      <text x={LEFT} y={214} fontSize={type.heading} fontWeight={weight.bold}>Sign in</text>
      <text x={LEFT} y={290} width={FIELD} contain="width" maxLines={1} fontSize={type.caption} color={color.text3}>{app.server?.name ? `${app.server.name} · ${serverOrigin() ?? ""}` : serverOrigin() ?? ""}</text>
      <Show when={app.authError}>
        <text x={LEFT} y={330} width={SCREEN.width - LEFT * 2} contain="width" maxLines={2} fontSize={type.caption} color={color.danger}>{app.authError}</text>
      </Show>
    </>
  );
}

function CodeSignIn(props: { onPassword: () => void }) {
  const [code, setCode] = createSignal("");
  const [problem, setProblem] = createSignal("");
  let round = 0;
  let poll: ReturnType<typeof setInterval> | undefined;
  let renew: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;

  const stop = () => { clearInterval(poll); clearTimeout(renew); };
  async function issue(): Promise<void> {
    stop();
    setProblem("");
    try {
      const info = await requestDeviceCode();
      if (disposed) return;
      setCode(info.code);
      // A blip doesn't matter: the code outlives it, and the next poll asks again.
      poll = setInterval(() => { void checkDeviceCode(info.code).catch(() => {}); }, POLL_MS);
      // TV clocks are often wrong, so a time that makes no sense means the code's whole life.
      const left = Date.parse(info.expiresAt) - Date.now();
      renew = setTimeout(() => {
        round += 1;
        if (round < ROUNDS) { void issue(); return; }
        stop();
        setCode("");
        setProblem("That code expired. Choose New code when you’re ready.");
      }, left > 0 && left <= CODE_LIFE_MS ? left : CODE_LIFE_MS);
    } catch (cause) {
      if (!disposed) setProblem(cause instanceof Error ? cause.message : String(cause));
    }
  }
  onMount(() => void issue());
  onCleanup(() => { disposed = true; stop(); });

  const cells = () => {
    const letters = code().split("");
    return Array.from({ length: CODE_LENGTH }, (_, index) => letters[index] ?? "");
  };

  return (
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={0}>
      <Row autofocus x={LEFT} y={850} width={SCREEN.width - LEFT * 2} height={80} gap={20} scroll="none">
        <Button size="lg" kind="primary" icon="lock-key" label="Use a password instead" onEnter={() => { props.onPassword(); return true; }} />
        <Button size="lg" icon="arrow-clockwise" label="New code" onEnter={() => { round = 0; void issue(); return true; }} />
        <Button size="lg" icon="globe" label="Change server" onEnter={() => { changeServer(); return true; }} />
      </Row>
      <Heading />
      <view x={LEFT} y={400} width={(CELL.width + CELL.gap) * CODE_LENGTH} height={CELL.height} display="flex" gap={CELL.gap} skipFocus>
        <For each={cells()}>
          {(letter) => (
            <view width={CELL.width} height={CELL.height} borderRadius={radius.m} color={color.surface2} border={{ width: 2, color: color.lineStrong }}>
              <text width={CELL.width} height={CELL.height} lineHeight={CELL.height} contain="both" textAlign="center" fontSize={type.display} fontWeight={weight.bold}>{letter}</text>
            </view>
          )}
        </For>
      </view>
      <text x={LEFT} y={572} fontSize={type.caption} fontWeight={weight.semibold} color={problem() ? color.danger : color.text2}>{problem() || (code() ? "Waiting for you to approve it" : "Getting a code…")}</text>
      <view x={GUIDE_X} y={400} width={GUIDE_WIDTH} height={420} skipFocus>
        <text fontSize={type.lead} fontWeight={weight.semibold}>On a phone or computer</text>
        <text y={64} width={GUIDE_WIDTH - QR_SIZE - 48} contain="width" lineHeight={42} fontSize={type.caption} color={color.text2}>Open NoFlex where you’re already signed in, go to Devices in your account, and enter this code under Link a device.</text>
        <text y={314} fontSize={type.caption} color={color.text3}>Or in a browser, go to</text>
        <text y={354} width={GUIDE_WIDTH} contain="width" maxLines={2} lineHeight={36} fontSize={type.caption} fontWeight={weight.semibold}>{approveUrl()}</text>
        <Show when={code()}>
          <QrCode x={GUIDE_WIDTH - QR_SIZE} text={`${approveUrl()}&code=${code()}`} size={QR_SIZE} />
          <text x={GUIDE_WIDTH - QR_SIZE} y={QR_SIZE + 10} width={QR_SIZE} contain="width" textAlign="center" fontSize={22} color={color.text3}>Scan to fill the code in</text>
        </Show>
      </view>
      <text x={LEFT} y={640} width={CELL.width * CODE_LENGTH} contain="width" lineHeight={38} fontSize={type.caption} color={color.text3}>This TV signs in by itself as soon as you do.</text>
    </view>
  );
}

function Field(props: NodeProps & { label: string; value: string; secret?: boolean; active: boolean }) {
  return (
    <view {...props} width={FIELD} height={112}>
      <text fontSize={type.caption} fontWeight={weight.semibold} color={color.text2}>{props.label}</text>
      <TextField y={36} width={FIELD} height={76} value={props.value} secret={props.secret} active={props.active} skipFocus />
    </view>
  );
}

function PasswordSignIn(props: { onCode: () => void }) {
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
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={1} onBack={() => { props.onCode(); return true; }}>
      <Column x={LEFT} y={400} width={FIELD} height={520} gap={24} scroll="none" onRight={() => { keyboard?.setFocus(); return true; }}>
        <Field label="Username" value={username()} active={active() === "username"} onEnter={() => edit("username")} />
        <Field label="Password" value={password()} secret active={active() === "password"} onEnter={() => edit("password")} />
        <Row width={FIELD} height={80} gap={20} scroll="none">
          <Button icon="television" label="Use a code instead" onEnter={() => { props.onCode(); return true; }} />
          <Button icon="globe" label="Change server" onEnter={() => { changeServer(); return true; }} />
        </Row>
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
      <Heading />
    </view>
  );
}

export default function Login() {
  const [usePassword, setUsePassword] = createSignal(false);
  return (
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={0}>
      <Show when={usePassword()} fallback={<CodeSignIn onPassword={() => setUsePassword(true)} />}>
        <PasswordSignIn onCode={() => setUsePassword(false)} />
      </Show>
    </view>
  );
}
