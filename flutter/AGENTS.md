# Flutter Client

`flutter/` is the NoFlex app for Android, iOS, macOS, Windows and Linux: one codebase, adapting to phones, tablets, foldables and desktop windows. Like every client it talks to the server only through GraphQL.

## Commands

Run inside `nix develop` (or `nix develop .#flutter-only`) from `flutter/`:

```bash
flutter pub get
dart run build_runner build        # regenerate GraphQL types after any *.graphql or schema change
flutter run -d linux               # or an Android device; see "Test devices"
flutter analyze --no-pub           # must be clean before committing; infos count
flutter test                       # needs generated types
dart format .                      # 2-space, unlike the rest of the repo
```

`__generated__/` is gitignored, so a fresh checkout must run `build_runner` before anything resolves.

## Layout

```
lib/
├── main.dart, app.dart, di.dart
├── theme.dart        # tokens ported from web/src/app.css; dark only, as on the web
├── layout/window.dart # size classes, folds, AdaptivePanes
├── graphql/          # schema.graphql (symlink to the server's SDL snapshot),
│   │                 # scalars.dart, query.dart (the Ferry-to-signals seam)
│   └── queries/      # one operation per file, one fragment per file
├── models/           # WorkSummary and MediaProbe: what the UI draws, mapped at the seam
├── services/         # singletons: tokens, graphql, session, nav, toast, network, watch state
├── features/<area>/  # screens, their controllers and services
├── widgets/          # nf_* kit, Artwork, WorkCard, Rail. Build UI from these
└── utils/            # format, tracks, images, categories, merge
tool/make_decoder_probes.sh  # regenerates assets/decoder-probes/
```

## Reactivity: signals only

Signals are the only way this app does reactivity. `test/conventions_test.dart` fails on anything else: no `setState`, `ChangeNotifier`, `ValueNotifier`, `StreamBuilder`, `FutureBuilder`, Provider, Riverpod or bloc.

- `signal()` holds state, `computed()` derives it, `batch()` groups writes, `effect()` reacts outside the tree.
- `SignalBuilder(builder: (context) => ...)` is the rebuild boundary; `SignalWidget` makes a whole small widget one. Keep them tight. signals 7 deprecated `Watch`, `SignalsMixin` and `.watch(context)`, so they're not used here even though older sibling projects do.
- **Bridge external state once, at the seam.** media_kit's streams are mirrored in `PlayerController`, Ferry's in `graphql/query.dart`, background_downloader's in `DownloadsService`, connectivity in `NetworkService`. Nothing downstream listens to a stream.
- `StatefulWidget` is for lifecycle and imperative objects only: `TextEditingController`, `FocusNode`, `PageController`, and creating or disposing a controller. Screen state is signals, on a controller class or in the `State`.

## Dependency injection

`getIt` from `di.dart`. Read services with `getIt<T>()`; no Provider or `InheritedWidget` for them. `BuildContext` is for framework state (theme, MediaQuery). The exception is `features/downloads/background_sync.dart`, which runs in its own isolate and has no `getIt`.

## Navigation

No router, no named routes.

- `_AppFrame` in `app.dart` switches on `SessionService.phase`; flipping the phase is all sign-in and sign-out do.
- `AppShell` keeps tab roots alive in an `IndexedStack`.
- Pages push onto the content navigator with `NavService.push`, beside the navigation rather than over it.
- The player (`pushFullWindow`), sheets and dialogs (`openSheet`) go on the root navigator.
- Destinations beyond the tabs are places (`NavPlace`). The sidebar goes to one with `NavService.open`, which replaces the current place and fades; a Browse tile pushes one with `push(place:)`.
- An observer on the content navigator keeps `NavService.place` current through every way of leaving. Everything opened from a place stays in it, so Movies stays lit under a film.

## Adaptive layout

Decide from available width through `layout/window.dart`, never from device checks.

- Navigation goes bottom bar, then rail (`>= 600`), then sidebar (`>= 1100`).
- The sidebar is the web's: the same destinations and foot, collapsing to icons with its button or `[`, remembered per device.
- Use `AdaptivePanes` for two-pane content. It splits on a vertical fold through the middle, so neither pane straddles the hinge.
- A list beside what it opens passes `splitOnFlatFold: false`: half a screen for a few words cramps the content, and a flat fold is no obstacle to it. A bent fold still divides them.
- The shell moves display features into its content's coordinates (`FoldsFromContent`), because a pane has no position until after its first layout, and a route's transition moves it after that.
- In tabletop posture the player gives the picture the top half and the controls the bottom.
- Rails and grids size from width, so more room shows more cards rather than bigger ones.

## Administration

`features/admin/` is the web's Administration page: one section list beside the open section, or leading to it on a phone.

- The open section is a signal rather than a route, so unfolding keeps the place.
- Each section asks for what it needs and stops when it's closed. Only the libraries, users and plugins (`AdminController.admin`) and the live host sample (`LiveStatus`) are shared.
- The entry shows only for `viewer.isAdmin`: in the sidebar, and on the You screen for widths without one.
- Polled queries use `FetchPolicy.NoCache`, or the normalized cache would keep every log line and stream forever.
- A plugin's homepage is shown to copy. Only provider credits open a browser (`url_launcher`), since TheTVDB's terms ask for a direct link.

## GraphQL

Ferry, against `lib/graphql/schema.graphql`, a symlink to `backend/crates/noflex-graphql/tests/snapshots/schema.graphql`.

- **One operation per file, one fragment per file.** Ferry sends the whole generated document with every request, so a file holding several operations gets refused as too complex, and imported fragments the operation doesn't use fail validation. Import with `#import 'fragments/work_card.graphql'`.
- Reads that drive a screen use `Query` (signals, cache then network). One-shot reads and mutations use `GraphqlService.run`, which throws `ApiError`, `ServerUnreachable` or `SessionExpired`. Requests that must not answer from cache set `FetchPolicy.NetworkOnly`.
- Ferry has nothing like Houdini's `@Type_delete`, so a removal ends in `Query.refetch`. A sheet draws from the `Query` rather than a copy of its rows, so it follows along.
- Ferry calls a response carrying only GraphQL errors "loading". `run` and `Query` account for that; anything new reading a response directly must too.
- A server refusing a non-null field leaves it out of the data, and Ferry would turn that into a deserialization failure. `refusalOf`, in a link outside auth, raises the real error first, so "invalid username or password" doesn't read as an unreachable server.
- Map generated types to `WorkSummary` where they arrive. Generated classes differ per operation and can't hold a linked server or sit in the offline manifest.
- `WatchStateService` holds progress and favorites as they change here, so cards update without refetching or writing to the cache. Read play state through it inside a reactive scope.
- Subscriptions go over `/graphql/ws`, which refuses an upgrade that names no subprotocol. `gql_websocket_link` speaks the legacy `graphql-ws`, so `_Transport` asks for it.
- Linked servers: pass `server:` to `GraphqlService.run` or `clientFor`. API responses use paths relative to the server that sent them; `TokenStore.absolute` resolves the home server's, `WorkSummary.fromCard(server:)` a linked one's.

## Playback

- **media_kit** (libmpv), not fvp: fvp's MDK is a closed-source binary, which can't meet AGPL's corresponding-source requirement.
- **media_kit has declared limited maintenance**, and its last release predates a year of fixes. `pubspec.yaml` overrides every media_kit package to a commit on `main`; move back to a release when one catches up.

### Decoders and version choice

- libmpv tries the platform's hardware decoder first and falls back to software by itself (verified on the emulator, `av1_mediacodec` failing over).
- **Measure, don't assume.** `DecoderProbeService` plays black-frame clips (`assets/decoder-probes/`) through a hidden libmpv with `vo=null` and `hwdec=auto-copy`, and reads `hwdec-current`: one code path on every platform, no native code. Results are cached per app version and OS version.
- If real playback lands in software where the measurement said hardware, `PlayerController` corrects the profile.
- `rankVersions` and `pickEdition` (`features/player/decoders.dart`) prefer hardware, then the most picture the display can show, then SDR over HDR that has to be tone-mapped, then the cheaper file.
- An edition is a different cut, so ties between editions keep the server's order. Once the viewer picks an edition or version, their pick stands.
- `buildCapabilities` reports the better of hardware and the software ceiling for each codec, and only audio this libmpv build can decode (`decoder-list`; Windows has no TrueHD). Dolby Vision without a compatible base layer is left out.

### Streams and seeking

- Server-encoded streams wait for their playlist before `open` (`_awaitPlaylist`). libmpv gives up on the first failed fetch, media_kit's network timeout is 5 s, and the server holds a playlist request for up to 10 s before answering 404, so a live channel or slow encoder would otherwise never start. Web clients get this from hls.js retrying.
- A ready version has two forms: HLS (`url`) and the file (`fileUrl`).
- Android starts on HLS and moves to the file for a resume or any seek but one ahead into the buffer, because libmpv's HLS seek there reads to the end without landing.
- Every other platform stays on HLS. nixpkgs' mpv on Linux seeks it fine; iOS, macOS and Windows are unknown.
- The file has no index, so opening it walks every fragment. Measured at 50 ms round trip on a 30 minute film: 21 s to start and 35 s to reach 25:00, against 1 and 2 s for HLS. FFmpeg's global `sidx` didn't help, and `use_mfra_for` ends playback a couple of seconds after a seek.
- libmpv reports a lost stream just as it does a finished film, and may jump to the last frame on the way. `playback_position.dart` tells them apart, so a server going away mid-film shows an error with Try again rather than marking the title watched.
- Live sets `force-seekable=no`. libmpv seeks to attach its video output once playing, and a live stream that claims it can seek never recovers from that.

### Tracks and display

- In-file audio and subtitle tracks switch in libmpv. Only a transcode needs the server: `audioStream`, or `burnSubtitle` for a track it can't deliver as text.
- libass draws subtitles into the picture.
- Picture-in-picture is Android only (`PipService`, `MainActivity.kt`): libmpv draws into a Flutter texture, and iOS's API wants an AVPlayer layer. Leaving the app while something plays enters it.
- HDR on Android forces `mediacodec-copy`, because SurfaceTexture's conversion doesn't know PQ or HLG. This hasn't been run on a real HDR device.

## Downloads

- `background_downloader` owns transfers natively: URLSession background tasks on iOS and macOS, WorkManager or UIDT on Android, isolates on Windows and Linux.
- `DownloadsService` keeps a JSON manifest (`OfflineStore`) and mirrors updates into signals.
- `smart_planner.dart` decides what to fetch and evict. The server ranks and enforces the budget (`smartDownloads`); the client enforces Wi-Fi, as the schema says.
- Only what the server put here (`OfflineOrigin.smart`) can be evicted. What the viewer chose stays until they remove it.
- A manual download asks `deviceDownload`. When that's null, the server has nothing ready for this device and an export job can stand in.
- The target folder setting is hidden on iOS. Android lists its app-specific volumes, including SD cards, which need no permission. Desktop gets a folder picker.

### Offline copies

- Exports (`ExportJob` in the API) are stream copies, so they keep the original's codecs. They're saved from You → Offline copies.
- A copy lives on the server that made it, so You → Offline copies asks every connected server.
- Finished copies of one edition and format share a file across users; removing one only deletes the file once nobody else holds it.

### Linked servers

- A linked server's title downloads from that server, whose administrator can refuse (`serverInfo.downloadsAllowed`; ask the server the title came from).
- The manifest keeps only its `serverId`, never its key. `workOf` gets the `ServerRef` back from the session, and progress queued offline waits until that server is known again.
- Smart downloads stay on the home server.

### iOS

iOS gives no guarantees about when an app runs, so nothing depends on Dart running.

- Everything wanted is enqueued at once as URLSession background tasks, which the system finishes with the app suspended or terminated. It relaunches the app to report back. Force-quitting cancels them; that's iOS, not us.
- `Config.resourceTimeout` is 7 days (default 4 hours), set before the first task runs, because a task's timer includes waiting for Wi-Fi. Download URLs carry 7-day tokens for the same reason.
- Wi-Fi only is `allowsCellularAccess = false`; the system holds the task until Wi-Fi returns.
- Discovering new titles uses `BGAppRefreshTask` (`background_sync.dart`, identifier in `Info.plist` and `AppDelegate.swift`), which runs when iOS sees fit, possibly rarely. The app also reconciles on launch, on regaining a network, after playback, and every 30 minutes while open.
- Files live in Application Support, excluded from iCloud backup. Paths are stored relative because the container moves between launches.
- The headless sync stands down while the app is on screen (a stamp it clears on pause), and the app reloads the manifest and tokens on resume. Two copies of the manifest would overwrite each other, and two refreshes of one token look like replay to the server, which revokes the session.

## Test devices

`nix develop` provides the emulator. AVDs live in `.cache/android/avd`, per checkout.

```bash
avdmanager create avd -n noflex_fold -k "system-images;android-36;google_apis_playstore;x86_64" --device pixel_fold
emulator -avd noflex_fold -no-window -gpu host -no-boot-anim   # drop -no-window to watch it
adb exec-out screencap -d <display id> -p > shot.png           # foldables have two displays; ids from `adb shell dumpsys SurfaceFlinger --display-id`
```

- **Build for the emulator with `--dart-define=NOFLEX_ANDROID_VO=mediacodec_embed`.** libmpv's `gpu` video output can't create an EGL context under the emulator's GL translation (`EGL_BAD_ATTRIBUTE`, in every `-gpu` mode), so nothing displays otherwise.
- `mediacodec_embed` skips libmpv's GL but can only show hardware-decoded frames, and there's no libass overlay. H.264, HEVC and VP9 display on the emulator; AV1 and subtitles need a real device or the Linux desktop.
- Live channels play without a picture on the emulator, though they show one on Linux.
- A ready version started from the top jumps about 10 s in on its own, on an internal seek just after it starts. Whether a real device's `gpu` output does the same isn't known.
- The server is at `10.0.2.2` from inside the emulator.
- A stale debug-only file (`Unhandled Exception: FormatException` from `NativeReferenceHolder`) after killing the emulator mid-write goes away with `adb shell pm clear live.noflex.noflex`.

## Motion and loading

The root `AGENTS.md`'s design language, in this app's pieces.

- Timing comes from `NfMotion`, the web's easings and durations. Check `MediaQuery.disableAnimationsOf` wherever you start motion yourself.
- Controls sink under a press with `NfPressable`, fed by `InkWell.onHighlightChanged` so a scroll taking over the gesture lifts them. Use `Press` and `NfSink` when only part of a control sinks, as `WorkCard` does.
- Material's ripple (`InkSparkle`) stays alongside the sink, and is enough on its own for a plain `InkWell` row. Route transitions stay native too (`pageTransitionsTheme`).
- A card's picture flies into the page it opens with `PosterHero`. Tags are per card, since a title can be in several rails on one page, and only fly when both ends draw the same shape.
- `AmbientGlow` goes inside a page, behind its content. Routes are opaque, so a tint under the shell would never show. Home has none: its hero covers the part of the screen the tint reaches.
- Loads draw in the content's shape with `NfSkeleton` and `NfSkeletonRows`. `CircularProgressIndicator` is for something underway rather than arriving: a busy button, a channel starting, buffering, the next page of a list.

## Conventions

- Build UI from `widgets/` and the tokens in `theme.dart`; don't hard-code colors or radii.
- Icons are Phosphor (`package:phosphor_icons`, a maintained fork of `phosphor_flutter`, which doesn't compile on current Flutter), never Material `Icons.*`. `actionIconTheme` covers the back and close arrows the framework draws itself.
- Draw SVGs with `jovial_svg`, not `flutter_svg`: flutter_svg ignores `<style>`, which draws most exported logos solid black.
- Fonts are bundled, as there's no fetching them from a CDN in a self-hosted app.
- Inter is a variable font: set weight through `nfText`, since `fontWeight` alone leaves the glyphs at the file's default.
- Wrap user-triggered mutations in `ToastService.attempt`.
- Every source file starts with an SPDX header, generated platform boilerplate included.
- Put Unicode (`’`, `·`, `–`) directly in strings.
