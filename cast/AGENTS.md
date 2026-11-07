# Cast

`cast/` is what NoFlex needs for Google Cast beyond the clients themselves: today, the receiver. A sender (`web/src/lib/cast.svelte.ts`) starts a session on a Cast device and loads a stream; the receiver plays it with the NoFlex look.

## Receiver

`cast/receiver/v<N>/` is a static app with no build step: `index.html`, `receiver.js`, `receiver.css`, `splash.svg`. It loads Google's receiver framework from Google, which a receiver has to.

- **It makes no requests to a NoFlex server.** The sender resolves the title, opens the playback session, resumes from the viewer's place, and reports progress. That is what lets it run from an HTTPS page whatever the server's address, since a browser blocks a page on HTTPS from calling a server on plain HTTP, and keeps credentials off the Cast device.
- **Consequence:** progress and session cleanup need the sender connected. If the phone leaves, the film keeps playing but nothing reports it; the server reaps the idle session.
- **Versioned by directory.** The Cast Developer Console points at one `v<N>/` URL. A change a published sender can't tolerate gets a new directory, and `customData.protocol` (in the sender's load request) says which dialect it speaks; an older receiver plays what it understands.
- **Host it over HTTPS** on the NoFlex domain (a published receiver must be HTTPS), serving `cast/receiver/` as is. For development, the Console accepts an `http://` LAN address for an unpublished app with registered test devices; `node --check cast/receiver/v1/receiver.js` is the only check that runs without a device.
- Styling is the framework's `cast-media-player` variables in `receiver.css`; the idle screen is `splash.svg`. Colors are `web/src/app.css`'s, ported by hand.
- Cast devices decode H.264, plus HEVC and VP9 on Ultra and Google TV models, and AV1 only on the Google TV Streamer. The sender picks a profile from the device's model, so most titles will transcode: Cast is never where AV1 wins.
