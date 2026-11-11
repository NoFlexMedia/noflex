# Cast

`cast/` is what NoFlex needs for Google Cast beyond the clients themselves: today, the receiver. A sender (`web/src/lib/cast.svelte.ts`) starts a session on a Cast device and loads a stream; the receiver plays it with the NoFlex look.

## Receiver

`cast/receiver/v<N>/` is a static app: `index.html`, `receiver.js`, `receiver.css`, and `codecs.js`, which the site's build bundles from `lib/ts/codecSupport.ts` (`site/scripts/cast-receiver.js`). It loads Google's receiver framework from Google, which a receiver has to.

- **It makes no requests to a NoFlex server.** The sender resolves the title, opens the playback session, resumes from the viewer's place, and reports progress. That is what lets it run from an HTTPS page whatever the server's address, since a browser blocks a page on HTTPS from calling a server on plain HTTP, and keeps credentials off the Cast device.
- **Consequence:** progress and session cleanup need the sender connected. If the phone leaves, the film keeps playing but nothing reports it; the server reaps the idle session.
- **Versioned by directory.** The Cast Developer Console points at one `v<N>/` URL. A change a published sender can't tolerate gets a new directory, and `customData.protocol` (in the sender's load request) says which dialect it speaks; an older receiver plays what it understands.
- **Hosted over HTTPS by the website** (a published receiver must be HTTPS): `site/` copies `cast/receiver/` as is to `https://noflex.dev/cast/receiver/`, so `v1` is `https://noflex.dev/cast/receiver/v1/`. For development, the Console accepts an `http://` LAN address for an unpublished app with registered test devices: serve `site/build/cast/receiver/` after `just site build`, as the source directory lacks `codecs.js`. `node --check cast/receiver/v1/receiver.js` is the only check that runs without a device.
- **Our own player, not `cast-media-player`.** The framework plays into a plain `<video class="castMediaElement">`, and `receiver.js` draws over it: the idle screen, the artwork while loading, the title, quality chips and timeline when paused or seeking, a spinner only after 400 ms, and stream info. `uiConfig.touchScreenOptimizedApp` is what Google asks of a receiver without `cast-media-player`, so smart displays get our buttons (`body.touchscreen`); a TV that hands the page its D-pad gets Enter, ←/→ and ↑ for stream info.
- Tokens are `web/src/app.css`'s, ported by hand into `receiver.css` in `vh`, so a 1080p TV, a 720p Chromecast and a Nest Hub share one layout. Icons are Phosphor's paths, inline.
- **What the sender loads says how it's playing**: `customData` carries `offsetMs` (a linear transcode's zero), `durationMs`, `backdrop` and `stream` (route, plan, reasons, source and target, encoder, quality), which the chips and the stream info panel draw. The panel opens from the sender (`{type: "stats"}`) or the D-pad.
- **It reports health back** on the same namespace: `{type: "health", kind}` on load, on every buffering change, on a ladder switch and every 5 s while playing, with Shaka's bandwidth estimate and the stream's size, rate and dropped frames. The sender's Auto decides from it; the receiver decides nothing.
- **HLS goes through Shaka** (`useShakaForHls`), for its adaptive bitrate across a ladder and its bandwidth estimate; `shakaConfig` waits 50 s for a segment the server is still encoding.
- Checked in headless Chrome against a stand-in for the framework (idle, loading, paused, stream info, health messages), never on a device: the framework's own playback, Shaka and the touch and D-pad paths are unverified.
- **The receiver probes its own device.** Asked `{type: "capabilities"}` on `urn:x-cast:dev.noflex.cast`, it runs the web client's own walk (`probeElsewhere`) with each `MediaQuestion` put to `canDisplayType`, and answers with the whole `ClientCapabilities`. A TV with Cast built in or a Google TV with an AV1 decoder is measured like a browser, about 500 questions, in one message to any sender.
- Google's Default Media Receiver can't be asked, so `castCapabilities` goes by model name (Google's device table) and falls back to H.264 at level 4.1.

## Sender

`web/src/lib/cast.svelte.ts` is the browser's side, for Chrome with the server's `cast_sdk` on (`docs/CONFIGURATION.md`): Google's sender script has to be admitted by the page's policy.

- **A Cast session is one more device in the Play-on list and the now-playing bar** (`connect.external`, with `Route` for how to steer it), so playing and steering feel as they do for a NoFlex device. Google's own button (`CastButton`, the SDK's `google-cast-launcher`) is what starts and ends a session, as Cast's guidelines require; it never goes inside our menu.
- The receiver app id is `VITE_CAST_APP_ID` at build time, and Google's Default Media Receiver until a NoFlex receiver is published. The default plays the stream but shows none of our look.
- **The sender must stay connected** for progress and cleanup: it reports every 15 s and ends the server session when the title stops or the Cast session ends.
- It asks the server with the receiver's probed profile, cached per device for the page (`castProfile.ts`), waits for a transcode's playlist before loading, and refuses `localhost`: a Cast device can't reach the browser's own loopback.
- **Quality is this browser's choice** (`qualityChoice`, the player's menu) and can be changed for the device from the now-playing bar's settings, which also toggles stream info on the TV. Auto runs `AutoQuality` (`$quality`) on the receiver's health reports: stalls on the original reload at the same place on the ladder, and the receiver's Shaka adapts along it from there.
- An on-demand transcode's playlist is the whole title, so the load request's `currentTime` is the resume point less the stream's offset, as for a file.
- Untested against a real Cast device. Loading the script, the session events, and the load request follow Google's documented API.
