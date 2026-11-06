# TV App

`solidtv/` is the NoFlex app for TVs: [SolidTV](https://solid-tv.github.io/solid/) (SolidJS over a WebGL renderer, with no DOM) from one `src/`, wrapped by a host per device family. Like every client it talks to the server only through GraphQL.

## Commands

Run inside `nix develop` from the repo root:

```bash
just tv install          # add `apple` for the NativeScript project's too
just tv dev              # http://localhost:5174, pointed at the demo server's address
just tv check            # tsc, GraphQL documents against the schema, tests: the CI gate
just tv build [target]   # solidtv/dist/<target>; default: every web-based target
just tv package <target> # dist/tv/: tizen, webos, android, browser, firebolt, tvos, ios
just tv run [tvos|ios]   # simulator or paired device; needs a Mac
just tv assets           # regenerate the committed sprites, icons and font atlases
```

## Targets

One `src/`. A Vite mode per way a browser engine loads it, `__TARGET__` folding away the rest, and NativeScript where there's no browser at all.

| Target | Host | Build | Plays through | Verified |
| ------ | ---- | ----- | ------------- | -------- |
| `browser` | any page, including TV browsers (Vizio, Hisense, Titan OS, Xbox) as a hosted app | modern + legacy | `<video>`, hls.js | Chrome, against the demo server: browse, direct play, HLS, seek |
| `tizen` | `hosts/tizen`: Samsung, 2017 on (Tizen 3+) | one classic bundle | same | The bundle from `file://` in Chrome. No TV yet |
| `webos` | `hosts/webos`: LG, 2016 on (webOS 3+) | one classic bundle | same | The bundle from `file://` in Chrome; `.ipk` builds. No TV yet |
| `android` | `hosts/android`: Android TV, Google TV, Fire TV | modern + legacy | same, in the system WebView | APK on an Android 16 phone emulator, against the demo server: browse, direct play, D-pad, Back, exit. No TV yet |
| `firebolt` | hosted URL on RDK (Comcast, Sky, Xumo) | modern + legacy | same | Builds. No device yet |
| `tvos`, `ios` | `hosts/nativescript` | webpack | AVPlayerLayer | Bundles on Linux and type-checks against the Apple SDK types. **Never run**: needs a Mac |

- **Legacy chunks parse as ES5**, checked with acorn. They have not run on a real Chrome 38.
- **PlayStation isn't a target.** SolidTV's docs mention it, but it needs Canvas2D and renderer 2.x is WebGL only.

## Layout

Deliberately `web/`'s shape: the same aliases, and `pages/` for its `routes/`.

```
src/
├── index.tsx        # renderer setup, routes
├── theme.ts         # tokens ported from web/src/app.css, as 0xRRGGBBAA
├── fonts.ts
├── lib/             # session, settings, playback, tokens, graphql, watch, ui, host, viewport, rows
│   ├── graphql/documents.ts  # every operation; graphql.test.ts validates them
│   └── player/      # VideoPlayer, the <video> one, the capability probe, WebVTT, trickplay
├── components/      # App, Shell, Nav, Rail, WorkCard, Keyboard, Sheet, SettingRow, player/
└── pages/
hosts/               # tizen, webos, android, nativescript
scripts/             # asset generators
public/              # committed: Inter atlases, icon sprite, brand mark, their licenses
```

- `$codecs` is `web/src/lib/codecSupport.ts`, imported rather than ported: a browser engine's probe is the same job, and the Apple host reuses its codec strings and level walks. It has to stay free of `$lib` imports.
- `format.ts`, `tracks.ts`, `tokens.ts` and `playback.ts` are ports. A fix to timing or track logic in `web/` probably belongs here too.

## Host seam

Everything that differs goes through `host` (`lib/host.ts`): storage, capabilities, `createPlayer`, `exit`, `keepAwake`.

- Vite targets branch on `__TARGET__` inside `host.ts`, so another target's code is dropped.
- NativeScript has no DOM: `hosts/nativescript/app/app.ts` calls `configureHost` before importing the app. Nothing in `src/` may touch `document` or `window` at import.
- Sessions are stored durably here, unlike the web's tab-scoped ones: a TV is an appliance. Two browser tabs of the `browser` target would share a refresh token and trip reuse detection; the web's tab locks weren't ported.

## Reactivity

Signals only, no stores. Solid's stores are Proxies, and webOS 3's Chrome 38 has none.

- `reactive({...})` (`lib/state.ts`) is the web's `$state`: an object of signals read and written as plain properties. `app`, `player` and `ui` are these.
- `createQuery` paints from the cache, then the network. `preload` warms it, from a card holding focus.
- `userDataOf(work)` reads play state. The cache holds whole responses, so changes live beside it (`lib/watch.ts`).
- `latest(work)` is the same for a title still indexing (`lib/indexing.ts`). `Artwork` watches what it shows, so one `worksById` every 4 s covers the screen, and stops when nothing's left.

## Rendering

- **No DOM, no CSS.** `<view>` and `<text>`, in a 1920×1080 space fitted whole inside the window (`lib/viewport.ts`). Colors are `0xRRGGBBAA` numbers from `theme.ts`.
- **The canvas's size is fixed at startup**, so the `browser` target reloads on resize. A window that isn't 16:9 gets bands, drawn by centering the root node: `useMouse` reads window coordinates, so moving the canvas would offset every hit.
- **A flex row doesn't lay out when a child changes width.** `Button` asks its parent to, for labels that change.
- **Text measures itself a frame late**, through `onEvent={{ loaded }}`'s `dimensions`. `Button` and `TextField` use that. Don't estimate a width from the character count where text gets clipped to it.
- **A row's `width` is its content's once laid out.** To ask what's in view, compare against the screen (`revealInRow`).
- **Don't name a prop `on`** on anything spread onto a node: it replaces the node's event method.
- **A node given an image it already has fires no `loaded`.** `Backdrop` allows for it.
- **White means focused, and nothing else.** The primary action rests in the accent (`Button`).
- **The renderer only loads textures it would draw.** A node at `alpha={0}` never fires `loaded`, so anything fading in on load starts at `0.001` (`Artwork`, `Backdrop`).
- **Text draws after quads**, deferred until a quad with a non-zero `zIndex` flushes it. So whatever covers text needs `zIndex` on the quad that does the covering, not on a parent (`Sheet`, `Nav`). Otherwise text shows through.
- **Anything moving keeps the renderer drawing every frame.** So no shimmer on skeletons, the status rings don't spin, and `Spinner` mounts only while waiting and after a delay. Let an idle TV idle. The one lasting exception is `indexingPulse`: one shared pulse, only while a title on screen is indexing, stopped under the player.
- **SDF text only**, in atlases covering Latin, Latin Extended-A, Greek and Cyrillic. **CJK, Arabic, Hebrew and Indic titles don't render.** One atlas can't hold them, and there's no Canvas2D text path on tvOS. Fixing it means per-script atlases or drawing those strings to textures on hosts that can.
- Atlases and sprites are generated and committed (`just tv assets`). Inter's `cv11`/`ss01`/`ss03` aren't applied: the generator can't select stylistic sets.
- `linearGradient` needs `stops`; use `gradient(colors, toward.down)` from `theme.ts`. Directions are the shader's: π/2 runs right to left, so use `toward`, not a bare angle.
- Ask the server for the size a box needs (`sized`). Texture memory is the budget on a TV.

## Signing in

- A code comes first (`createDevicePin`, then `pollDevicePin` every 3 s, as Flutter does); the password is the fallback. Approval is Devices → Link a device on the web and in the apps.
- `QrCode` draws the approval URL with the code in it (`&pin=`), which the web's Devices tab fills in. It's rectangles, not a canvas: no host-specific drawing, and one quad per run of dark modules.
- A code lives 10 minutes and renews itself three times, then the screen stops asking the server. A TV's clock can be wrong, so an `expiresAt` that makes no sense counts as the whole 10 minutes.

## Focus and keys

- `App.tsx` maps keys. The framework reads `event.key` before `keyCode`, so TV-reported names must be listed, not just codes.
- `useMouse` focuses a child directly, which a row's scroller never hears about: whatever sits in a row calls `revealInRow` a tick after focus.
- A real keyboard is taken in `Keyboard`'s capture-phase listener, ahead of focus, so Backspace erases until a field is empty and then means Back again. The Siri Remote's Menu arrives as Backspace, so tvOS skips it.
- Back does double duty (`Shell.tsx`): a title goes back; a destination opens the nav; the nav goes Home; Home's nav leaves the app. On tvOS leaving means returning false, so the system takes the press, as review expects.
- Anything that takes focus gives it back: `Sheet`, `Player` store `activeElement` and restore it. The page stays mounted under the player, at `alpha={0}`, so Back lands on the same card.
- Don't let focus jump when data arrives: `Title` shows Play for a series from the start rather than inserting it once the next episode is worked out.
- A `Row`'s or `Column`'s children need sizes. Rails state `height` explicitly.
- `KeepAliveRoute` keeps Home and Browse with their focus and scroll. Their effects keep running while hidden; `props.isAlive` says whether they're showing.
- **A hidden kept page still reacts.** `useParams` goes blank once its route is gone (`Browse` holds its last value), and data arriving late still autofocuses. `Shell` moves focus off any hidden page, and tells one by `page.children`: it leaves that but keeps its `parent`.
- **Don't hold focus in `<For>` over refetched data.** New objects rebuild every row, destroying the focused one with no event to catch. Use `<Index>` (`Account`'s marks).
- A numbered `forwardFocus` counts children as they stand: dynamic ones move it. Put what takes focus first and use `0` (`Search`, `Login`).
- Signing out clears kept pages and returns to `/`, so the next sign-in starts at Home with nothing of the last viewer's (`App`).

## Playback

The transport is one component (`player/Player.tsx`) over `VideoPlayer` (`lib/player/types.ts`). Timing rules are `web/src/components/Player.svelte`'s.

- **Up and Down walk a stack:** the rail (focus on the player itself; Left and Right scrub), the buttons, the chapters. Up off the rail dismisses the overlay. Hiding it moves focus back to the player, or a key would act on buttons nobody can see.
- **Chapters are `chapterStops`:** the file's, or evenly spaced scenes where it has none but trickplay can show them. `web/src/lib/chapters.ts` is the same logic.
- Skip intro floats outside the overlay, and OK on the player takes it. Same for the up-next card.
- Settings split on who they belong to: the `playback` preference follows the viewer and every client shares it, so save through `savePlaybackPrefs`, which keeps other clients' fields. `lib/settings.ts` is this TV's room and remote.
- **Streaming quality** lowers `maxHeight` in the capabilities sent to `startPlaybackSession`, and HDR off clears `videoRanges`. Measured: 480p on a 4K VP9 file gets `av1_nvenc` at 1146×480, with the reason in the info panel.
- **A TV doesn't download:** nothing plays local files here, and tvOS has no persistent storage. Smart Downloads is the phone app's.
- **Smart transcode marks** (`Title`, Settings) are the hard signal: "Mark for smart transcoding" is for every device with it on, "Smart transcode for this TV" for this one whatever its setting. Both are independent.

- **Video sits behind the canvas, which clears to transparent.** `clearColor: 0` plus the renderer's alpha, premultiplied context. On the web hosts it's a `<video>` under `#app`, measured working in Chrome with a 4K stream under the UI. Don't draw video into a GL texture: it loses HDR and hardware composition.
- Subtitles draw on the canvas (`lib/player/vtt.ts`) on every host: `<track>` doesn't exist on tvOS, and AVPlayer takes no sidecar without a manifest rewrite. Image formats are burned in by the server.
- Web hosts use hls.js wherever there's MSE, as the web client does: it retries playlists the transcoder hasn't finished writing.
- **HDR is only reported where `matchMedia("(dynamic-range: high)")` says so**, which Tizen and older webOS don't implement, so those TVs get tone-mapped SDR for now. Tizen's `webapis.avinfo` is the likely fix.

### Apple

`hosts/nativescript/app/avplayer.ts`, an `AVPlayerLayer` in a sibling `UIView` below the canvas. None of this has run on hardware, in this order of risk:

1. **Whether the canvas composites transparently.** `app.ts` sets `opaque` false on the canvas's view tree during playback, and the renderer already requests alpha. `@nativescript/canvas` exposes no switch for this, and its backing layer is inside a binary framework. If video stays hidden, try `AVPlayerViewController` with a custom overlay. Don't move video into WebGL.
2. **The Siri Remote during playback.** `bindRemote` is verified by SolidTV on hardware, but not with a layer playing.
3. **HDR and Dolby Vision through a blended overlay**, and frame pacing while the UI draws.
4. Capabilities (`capabilities.ts`) ask `isPlayableExtendedMIMEType` and `VTIsHardwareDecodeSupported`. The C functions aren't in `@nativescript/types`, so they're declared by hand; the runtime's metadata should have them.

Other notes:

- Status is polled at 250 ms, not observed: KVO from NativeScript means subclassing `NSObject` with exposed selectors.
- `awaitPlaylist` holds until the encoder has written a playlist, as Flutter's `_awaitPlaylist` does: AVPlayer gives up on the first failed fetch.
- Sessions live in `ApplicationSettings` (`NSUserDefaults`), unencrypted, not the polyfill's `localStorage`, a debounced file in tvOS's purgeable Caches. The Keychain would be better.
- `webpack` pinned to 5.111 by `overrides`: newer ones moved files `@nativescript/webpack` requires. Packages are npm rather than the demo's pnpm; `npx ns` is unverified with that, so reach for its `nodeLinker: hoisted` setup first if the CLI can't find plugins.
- iOS has no remote, so `bindTouch` maps swipes to arrows, tap to Enter, long press to Back. It's the TV interface on a phone, in a 16:9 area at the left of a wider screen; Flutter's is the real iOS app.
- The tvOS asset catalog (layered icon, top shelf) isn't made yet; App Store submission needs it. Team IDs go in `App_Resources/*/build.xcconfig`.

### Android

`hosts/android`, one `MainActivity` hosting a `WebView`, in Java so there's no Kotlin plugin to keep in step.

- `WebViewAssetLoader` gives it a real origin, so module scripts load. It's `http`, not the usual `https`: an https page's `<video>` is auto-upgraded to https whatever the mixed content mode says, so a plain-HTTP home server fails with "Format error" while `fetch` and images work. Measured on the emulator. It isn't a secure context as a result; nothing here needs one.
- `getDefaultVideoPoster` returns a blank bitmap, or a WebView draws a grey play button over every `<video>`.
- A WebView keeps Back and media keys, so `dispatchKeyEvent` re-dispatches them into the page. From `targetSdk` 36 Back doesn't reach it at all, and would leave the app: that's the `OnBackInvokedCallback`.
- `window.NoFlexHost` is `exit` and `keepAwake`. A `<video>` in a WebView doesn't hold off the screensaver.
- Performance is the system WebView's. Very old boxes may ship one that can't cope; GeckoView is the documented fallback.
- The debug APK enables WebView debugging: `adb forward tcp:9222 localabstract:webview_devtools_remote_<pid>`, then `chrome://inspect`. On a phone AVD, set `immersive_mode_confirmations` to `confirmed` or a dialog eats the first keys.
- Uses the flake's `gradle` in the `flutter-android` shell, with AGP 8.13 to match it (Flutter's wrapper is on Gradle 9).

### Tizen and webOS

- Packaged apps load from `file://`, where Chromium refuses module scripts for want of a MIME type, so they get one classic bundle (`renderModernChunks: false`).
- Origin is `null`, which the server's CORS layer answers, except before the first account exists. So setup happens in a browser, and the app says so.
- Tizen: `tvinputdevice.registerKeyBatch` is what makes media keys reach the page. Signing needs Samsung's `tizen` CLI and a certificate; neither is in nixpkgs or npm, so `just tv package tizen` stops at the ready folder without it. `id` in `config.xml` is a placeholder.
- webOS: `disableBackHistoryAPI` hands us Back, and `window.close()` leaves. The Magic Remote points, so `useMouse` is on here and in `browser`.

## What's trimmed

Administration stays in the web client and the apps; the status dot (`ServerStatus`) is what admins get here. Settings (`pages/Account.tsx`) has what a viewer changes from the couch, not their profile, sessions or API keys.

- **Watch together:** needs a WebSocket, which NativeScript's runtime lacks.
- **Linked servers:** rails and search stay on the home server.
- **External login:** needs a browser to redirect through. A device-PIN (`pages/Login.tsx`) is the way in without typing a password, and covers it: whoever approves it can have signed in however they like.
- **Live TV's programme grid and recording rules:** channels with what's on now, and tuning in.
- **Photos:** a title page with the image; no slideshow.
- **Offline copies and downloading on the TV itself, library management, filters, per-title admin.**

## Conventions

- Files start with the SPDX header. JSON can't hold one, so `package.json` carries `license`.
- Import through `$lib`, `$components`, `$pages`, never `../../`. Only `theme.js` is relative, being beside neither.
- Type sizes are the web's scaled for ten feet. Nothing under `type.caption` except secondary lines at 22.
- Icons are Phosphor, through `Icon`. A new one goes in `ICONS` in `scripts/build-icons.mjs`, then `just tv assets`.
- A GraphQL field added to a card costs every card on Home. Detail-only fields go on `WORK_DETAIL`.
