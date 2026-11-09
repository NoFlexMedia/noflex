// SPDX-License-Identifier: AGPL-3.0-only
//
// Who's watching. A TV is shared, so everyone who has signed in on it keeps a
// tile, and hopping between them is a press, or a PIN for whoever set one. The
// tiles are only ever the people who signed in here: the server never lists its
// users to a screen nobody has signed in on.

import { Row } from "@solidtv/solid/primitives";
import { createSignal, Index, Show } from "solid-js";
import type { JSX } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import type { IconName } from "$lib/icons.generated.js";
import { addProfile, app, chooseProfile, PinRefused, rejoin } from "$lib/session.js";
import { serverOrigin } from "$lib/tokens.js";
import type { Profile } from "$lib/types.js";
import { attempt } from "$lib/ui.js";
import { color, motion, SCREEN, type, weight } from "../theme.js";
import Avatar from "$components/Avatar.js";
import BrandMark from "$components/BrandMark.js";
import Icon from "$components/Icon.js";
import PinPad, { PIN_MIN, PIN_PAD_HEIGHT, PIN_PAD_WIDTH } from "$components/PinPad.js";

const EDGE = 140;
const TILE = { width: 260, height: 330, disc: 200 };
const GAP = 36;
const RING = 8;
const INSET = (TILE.width - TILE.disc) / 2;
const DISC_Y = 20;
const BADGE = 60;
const DOT = 26;
const DOT_GAP = 22;

function Tile(props: NodeProps & { label: string; caption?: string; badge?: IconName; children: JSX.Element }) {
  const [focused, setFocused] = createSignal(false);
  return (
    <view {...props} width={TILE.width} height={TILE.height} scale={focused() ? 1.08 : 1} transition={{ scale: motion.spring }} onFocusChanged={setFocused}>
      {/* White means focused: a ring, since the picture is the tile. */}
      <view x={INSET - RING} y={DISC_Y - RING} width={TILE.disc + RING * 2} height={TILE.disc + RING * 2} borderRadius={TILE.disc / 2 + RING} color={color.light} alpha={focused() ? 1 : 0} transition={{ alpha: motion.fast }} />
      <view x={INSET} y={DISC_Y} width={TILE.disc} height={TILE.disc} children={props.children} />
      <Show when={props.badge}>
        <view x={INSET + TILE.disc - BADGE + 4} y={DISC_Y + TILE.disc - BADGE + 4} width={BADGE} height={BADGE} borderRadius={BADGE / 2} color={color.bgRaised} border={{ width: 2, color: color.lineStrong }}>
          <Icon name={props.badge!} size={32} x={(BADGE - 32) / 2} y={(BADGE - 32) / 2} color={color.text} />
        </view>
      </Show>
      <text y={DISC_Y + TILE.disc + 26} width={TILE.width} contain="width" maxLines={1} textAlign="center" fontSize={type.body} fontWeight={weight.semibold} color={focused() ? color.text : color.text2}>{props.label}</text>
      <text y={DISC_Y + TILE.disc + 70} width={TILE.width} contain="width" maxLines={1} textAlign="center" fontSize={22} color={color.text3}>{props.caption ?? ""}</text>
    </view>
  );
}

function PinEntry(props: { profile: Profile; onClose: () => void }) {
  const [pin, setPin] = createSignal("");
  const [problem, setProblem] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const slots = () => Array.from({ length: Math.max(PIN_MIN, pin().length) }, (_, index) => index < pin().length);
  const dotsWidth = () => slots().length * DOT + (slots().length - 1) * DOT_GAP;

  const submit = async () => {
    if (busy()) return;
    setBusy(true);
    setProblem("");
    try {
      await chooseProfile(props.profile.id, pin());
    } catch (cause) {
      // Back among the tiles, which say why.
      if (cause instanceof PinRefused && cause.final) { props.onClose(); return; }
      if (cause instanceof PinRefused) setPin("");
      setProblem(cause instanceof Error ? cause.message : String(cause));
    }
    setBusy(false);
  };

  return (
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={0} onBack={() => { props.onClose(); return true; }}>
      <PinPad autofocus x={(SCREEN.width - PIN_PAD_WIDTH) / 2} y={SCREEN.height - PIN_PAD_HEIGHT - 110} value={pin()} onChange={(value) => { if (!busy()) { setPin(value); setProblem(""); } }} onSubmit={() => void submit()} />
      <Avatar x={(SCREEN.width - 140) / 2} y={90} size={140} name={props.profile.name} url={props.profile.avatarUrl} />
      <text y={256} width={SCREEN.width} contain="width" maxLines={1} textAlign="center" fontSize={type.title} fontWeight={weight.bold}>{`Enter ${props.profile.name}’s PIN`}</text>
      <view x={(SCREEN.width - dotsWidth()) / 2} y={344} width={dotsWidth()} height={DOT} display="flex" gap={DOT_GAP} skipFocus>
        <Index each={slots()}>
          {(filled) => <view width={DOT} height={DOT} borderRadius={DOT / 2} color={filled() ? color.accent : color.surface3} />}
        </Index>
      </view>
      <text y={400} width={SCREEN.width} contain="width" maxLines={1} textAlign="center" fontSize={type.caption} fontWeight={weight.semibold} color={problem() ? color.danger : color.text3}>{problem() || (busy() ? "Checking…" : "Back returns to who’s watching")}</text>
    </view>
  );
}

export default function Profiles() {
  const [asking, setAsking] = createSignal<Profile | null>(null);
  const [selected, setSelected] = createSignal(0);
  const [busy, setBusy] = createSignal(false);

  const count = () => app.profiles.length + 1;
  const rowWidth = () => Math.min(count() * TILE.width + (count() - 1) * GAP, SCREEN.width - EDGE * 2);

  const choose = (profile: Profile, index: number) => {
    setSelected(index);
    if (busy()) return true;
    if (profile.signedOut) { rejoin(profile); return true; }
    if (profile.hasPin) { setAsking(profile); return true; }
    setBusy(true);
    void attempt(() => chooseProfile(profile.id)).then(() => setBusy(false));
    return true;
  };

  return (
    <view width={SCREEN.width} height={SCREEN.height} forwardFocus={0}>
      <Show
        when={asking()}
        keyed
        fallback={(
          <view width={SCREEN.width} height={SCREEN.height} forwardFocus={0}>
            <Row autofocus x={(SCREEN.width - rowWidth()) / 2} y={400} width={rowWidth()} height={TILE.height} gap={GAP} selected={selected()} scroll="auto">
              {/* By position: each refresh brings new objects, and the tile holding focus has to survive them. */}
              <Index each={app.profiles}>
                {(profile, index) => (
                  <Tile
                    label={profile().name}
                    caption={profile().signedOut ? "Signed out" : ""}
                    badge={profile().signedOut ? undefined : profile().hasPin ? "lock-key" : undefined}
                    onEnter={() => choose(profile(), index)}
                  >
                    <Avatar size={TILE.disc} name={profile().name} url={profile().avatarUrl} dim={profile().signedOut} />
                  </Tile>
                )}
              </Index>
              <Tile label="Add someone" onEnter={() => { addProfile(); return true; }}>
                <view width={TILE.disc} height={TILE.disc} borderRadius={TILE.disc / 2} color={color.surface2} border={{ width: 2, color: color.lineStrong }}>
                  <Icon name="plus" size={72} x={(TILE.disc - 72) / 2} y={(TILE.disc - 72) / 2} color={color.text2} />
                </view>
              </Tile>
            </Row>
            <BrandMark x={(SCREEN.width - 72) / 2} y={96} size={72} />
            <text y={204} width={SCREEN.width} contain="width" textAlign="center" fontSize={type.heading} fontWeight={weight.bold}>Who’s watching?</text>
            <Show when={app.authError}>
              <text x={EDGE} y={300} width={SCREEN.width - EDGE * 2} contain="width" maxLines={2} textAlign="center" fontSize={type.caption} color={color.danger}>{app.authError}</text>
            </Show>
            <text x={EDGE} y={820} width={SCREEN.width - EDGE * 2} contain="width" maxLines={2} textAlign="center" lineHeight={34} fontSize={22} color={color.text3}>A profile with a PIN asks for it here. Set yours under Profile, in the web client or the apps.</text>
            <text x={EDGE} y={960} width={SCREEN.width - EDGE * 2} contain="width" maxLines={1} textAlign="center" fontSize={22} color={color.text3}>{app.server?.name ? `${app.server.name} · ${serverOrigin() ?? ""}` : serverOrigin() ?? ""}</text>
          </view>
        )}
      >
        {(profile) => <PinEntry profile={profile} onClose={() => setAsking(null)} />}
      </Show>
    </view>
  );
}
