// SPDX-License-Identifier: AGPL-3.0-only

import { useNavigate } from "@solidjs/router";
import { createEffect, Match, onCleanup, Show, Switch, type ParentProps } from "solid-js";
import { useFocusManager, useMouse } from "@solidtv/solid/primitives";
import { clearKeepAliveRoute } from "@solidtv/solid/primitives/router";
import type { KeyMap } from "@solidtv/solid";
import { originX, originY } from "$lib/viewport.js";
import { SCREEN } from "../theme.js";
import { host } from "$lib/host.js";
import { closePlayer, player } from "$lib/playback.js";
import { startRemote } from "$lib/remote.js";
import { app, start } from "$lib/session.js";
import { ui } from "$lib/ui.js";
import Backdrop from "$components/Backdrop.js";
import BrandMark from "$components/BrandMark.js";
import Player from "$components/player/Player.js";
import State from "$components/State.js";
import Toaster from "$components/Toaster.js";
import Connect from "$pages/Connect.js";
import Login from "$pages/Login.js";
import Profiles from "$pages/Profiles.js";

// The framework reads `event.key` before `event.keyCode`, so the names TVs report
// have to be listed, not just their codes. Back: Tizen 10009, webOS 461. Media keys
// are the same codes on both; the Siri Remote and WebView name them instead.
const KEYS: Partial<KeyMap> = {
  Back: ["Back", "GoBack", "Backspace", "BrowserBack", "Escape", 8, 27, 461, 10009],
  Left: ["ArrowLeft", "Left", 37],
  Right: ["ArrowRight", "Right", 39],
  Up: ["ArrowUp", "Up", 38],
  Down: ["ArrowDown", "Down", 40],
  Enter: ["Enter", "Select", 13],
  Play: ["MediaPlay", "Play", 415],
  Pause: ["MediaPause", "Pause", 19],
  PlayPause: ["MediaPlayPause", "PlayPause", 10252, 179],
  Stop: ["MediaStop", "Stop", 413],
  FastForward: ["MediaFastForward", "FastForward", 417],
  Rewind: ["MediaRewind", "Rewind", 412],
};

export default function App(props: ParentProps) {
  useFocusManager(KEYS);
  // A desktop mouse, and LG's Magic Remote, which points: hovering focuses, a click is Enter.
  if (__TARGET__ === "browser" || __TARGET__ === "webos") useMouse();
  // Whoever watches next starts at Home, and not with what kept-alive pages or the player held for the last viewer.
  const navigate = useNavigate();
  createEffect(() => {
    if (app.phase !== "auth" && app.phase !== "profiles") return;
    clearKeepAliveRoute();
    closePlayer();
    ui.backdrop = null;
    navigate("/", { replace: true });
  });
  // Other devices of the viewer's can play on this one while they're the one watching.
  createEffect(() => {
    if (app.phase === "app") onCleanup(startRemote());
  });
  return (
    // Whatever nothing else took: Home's navigation, or a screen with none, like signing in.
    // Returning false too, so tvOS's system takes the press where there's no `exit`.
    <view x={originX} y={originY} width={SCREEN.width} height={SCREEN.height} onBack={() => { host.exit?.(); return false; }}>
      {/* Hidden rather than unmounted under the player: Back lands on the same card. */}
      <view width={SCREEN.width} height={SCREEN.height} alpha={player.current ? 0 : 1} forwardFocus={1}>
        <Backdrop />
        <Switch>
          <Match when={app.phase === "app"}>{props.children}</Match>
          <Match when={app.phase === "connect"}><Connect /></Match>
          <Match when={app.phase === "profiles"}><Profiles /></Match>
          <Match when={app.phase === "auth"}><Login /></Match>
          <Match when={app.phase === "setup"}>
            <State autofocus title="Almost there" message="This server hasn’t been set up yet. Open it in a browser to create the first account, then try again here." action={start} actionLabel="Try again" />
          </Match>
          <Match when={app.phase === "error"}>
            <State autofocus kind="error" title="The server is out of reach" message={app.error} action={start} />
          </Match>
          <Match when={app.phase === "boot"}>
            <BrandMark x={SCREEN.width / 2 - 56} y={SCREEN.height / 2 - 56} size={112} />
          </Match>
        </Switch>
      </view>
      <Show when={player.current}>{(current) => <Player playback={current()} />}</Show>
      <Toaster />
    </view>
  );
}
