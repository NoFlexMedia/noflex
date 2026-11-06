// SPDX-License-Identifier: AGPL-3.0-only
//
// What a TV needs of the web's Account page: who this is, where they're signed in,
// and the way out. Everything else is easier done with a real keyboard.

import { Row } from "@solidtv/solid/primitives";
import { logout, app, changeServer } from "$lib/session.js";
import { serverOrigin } from "$lib/tokens.js";
import { attempt } from "$lib/ui.js";
import { color, PAGE_WIDTH, PAGE_X, SCREEN, type, weight } from "../theme.js";
import Button from "$components/Button.js";

export default function Account() {
  const name = () => app.viewer?.name ?? "";
  const initials = () => name().slice(0, 2).toUpperCase();
  return (
    <view x={PAGE_X} width={PAGE_WIDTH} height={SCREEN.height} forwardFocus={0}>
      <Row autofocus y={520} width={PAGE_WIDTH} height={80} gap={20} scroll="none">
        <Button size="lg" icon="sign-out" label="Sign out" onEnter={() => { void attempt(logout); return true; }} />
        <Button size="lg" icon="globe" label="Change server" onEnter={() => { changeServer(); return true; }} />
      </Row>
      <text y={48} fontSize={type.heading} fontWeight={weight.bold}>Account</text>
      <view y={180} width={144} height={144} borderRadius={72} color={color.accent}>
        <text width={144} height={144} lineHeight={144} contain="both" textAlign="center" fontSize={56} fontWeight={weight.bold} color={color.accentInk}>{initials()}</text>
      </view>
      <text x={184} y={190} fontSize={type.title} fontWeight={weight.semibold}>{name()}</text>
      <text x={184} y={248} fontSize={type.body} color={color.text2}>{app.viewer?.isAdmin ? "Administrator" : "Member"}</text>
      <text y={380} fontSize={type.body} color={color.text2}>{`${app.server?.name ?? "NoFlex"} · v${app.server?.version ?? ""}`}</text>
      <text y={424} fontSize={type.caption} color={color.text3}>{serverOrigin() ?? ""}</text>
      <text y={660} width={1200} contain="width" lineHeight={38} fontSize={type.caption} color={color.text3}>Profile, downloads, devices and server administration are in the web client and the apps.</text>
    </view>
  );
}
