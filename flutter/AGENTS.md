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

- `_AppFrame` in `app.dart` switches on `SessionService.phase`; flipping the phase is all sign-in, sign-out and switching profile do.
- `AppShell` keeps tab roots alive in an `IndexedStack`.
- Pages push onto the content navigator with `NavService.push`, beside the navigation rather than over it.
- The player (`pushFullWindow`), sheets and dialogs (`openSheet`) go on the root navigator.
- Destinations beyond the tabs are places (`NavPlace`). The sidebar goes to one with `NavService.open`, which replaces the current place and fades; a Browse tile pushes one with `push(place:)`.
- An observer on the content navigator keeps `NavService.place` current through every way of leaving. Everything opened from a place stays in it, so Movies stays lit under a film.

## Signing in and profiles

A device several people use keeps a session for each of them. `features/auth/profiles_screen.dart` is "Who's watching?".

- `TokenStore` holds everyone signed in here (`accounts`) and who's active (`activeId`), written as one secure-storage value so the headless sync never reads one without the other. "The session" everywhere else means the active account's.
- Only `SessionService` changes who's active: `signIn`, `chooseProfile`, `switchProfile`, `logout`. Each loads that account's downloads (`DownloadsService.useAccount`) before the shell appears.
- **At launch, one account that's still active goes straight in.** Anything else is `AppPhase.profiles`: several accounts, or one that stepped away. Showing it deactivates whoever was active and, if they have a PIN, locks their session on the server.
- **A profile with a PIN is entered only through `TokenStore.unlock`** (`unlockSession`), locked or not, so the gate holds when `lockSession` never got through. No hash of the PIN is on the device, so such a profile can't be opened while the server is away; one without a PIN can, for its downloads.
- **Only signing out or Remove takes someone off the device.** A session the server ended (expired, reused, out of PIN tries, or unknown to `rememberedProfiles`) keeps its tile as `needsSignIn` (`TokenStore.end`), and its downloads keep their owner. Its message names whose it was.
- A refresh refused with `SESSION_LOCKED` isn't an ending: it's `SessionExpired(locked: true)` and `onLocked`, back to "Who's watching?" with the session as it was.
- Choosing a `needsSignIn` tile opens the sign-in screen for them (`signingInFor`): "Sign in again", with Remove beside it. One person who has to sign in again skips the tiles and lands there directly (`_soleEnded`), as the TV does.
- Tiles paint from what's stored, then `rememberedProfiles` corrects them and marks a session the server no longer knows (`needsSignIn`). A tile's picture comes from there too: `viewer.avatarUrl` is signed for the session and dies with a lock.
- **Each account has its own device id** (`DeviceIdentity.idFor`), because the server refuses an id another user registered. It's in preferences, not with the session, so it outlives signing out.
- You → Devices → Link a device approves a code as the viewer. An administrator can choose anyone instead (`approveCode(user:)`), for an account with nothing signed in to approve from; the server records it.
- The PIN is set under You → Profile, with the password (`setPin`, `removePin`). `_pinChanged` tells this device's own tile.
- **Deleting an account is You → Profile → Delete account** (`features/account/delete_account.dart`), as app stores require of an app that makes accounts. `accountDeletion` says whether it takes the password or the username typed out, and whether it erases the server; a `LAST_ADMINISTRATOR` refusal mid-way turns the sheet into the erasing one.
- **The last administrator is asked twice**: what's erased and what's kept (media files, the server's configuration), then `delete everything` typed with their password, sent as `eraseServer`.
- A deleted account leaves the device as signing out does, and its device id goes too (`SessionService.accountDeleted`, `DeviceIdentity.forget`): the server has no such device any more. Everyone else stays.
- **A server being reset has nobody on it**, so `serverErased` forgets every account here and their downloads and ids, keeps the server, and shows `AppPhase.resetting` until `serverInfo` stops saying `resetting`. It restarts on the way, so an unreachable or refusing server is polled through. `start` and a `SERVER_RESETTING` refusal of the bootstrap land there too, for every other device.
- Signing in as someone already here replaces their session where it was. Whoever was signed in stays remembered, so the sign-in screen offers Back to profiles in place of Change server, which signs everyone out.

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
- **A video transcode is on demand** (see `backend/AGENTS.md`): its playlist is the whole title, so `startPositionMs` is `0`, the player opens at the resume point, and every seek is native (`_wholeTitle`, from the `#EXT-X-ENDLIST` `_awaitPlaylist` reads). A copied-video one (remux, or audio alone) is still linear, from its offset, and a seek past the buffer opens a new session.
- Server-encoded streams set `network-timeout` to 50 s, as the server holds a segment request for up to 45 s while it encodes it; media_kit's 5 s would drop the segment. Everything else keeps 5 s.

### Quality

- **Quality is Original until the viewer picks**: the player's Quality sheet or You → On this device → Streaming quality, one value per device (`noflex.quality`, `encodeChoice`). A pinned step is its `bitrateBps` as `maxBitrateBps`; the sheet offers the first session's `qualities`, as later ones are asked with a limit.
- `features/player/quality.dart` ports `lib/ts/quality.ts`, and `test/quality_test.dart` mirrors the TV's tests of it: a threshold changes in both.
- Auto asks for `adaptive` sessions. On the original, `AutoQuality`'s stalls reopen the session at the same place on the ladder below the original's bitrate; comfortably over the original for a minute, three minutes on, it goes back. Both toast.
- **libmpv doesn't adapt along a ladder.** FFmpeg's HLS demuxer plays whichever variant it's given, and given a master it fetches the first segment of every variant to probe them. So the app reads an adaptive master itself (`parseMaster`) and opens one variant's playlist, and `LadderSteps` (`ladder.dart`) moves: down on the stalls that would leave the original, up once a better variant has fit the estimate for 20 s after 45 s on one. Each move reopens that variant at the same place, a short wait where hls.js or ExoPlayer would switch seamlessly.
- **The bandwidth estimate is `demuxer-cache-state`'s `raw-input-rate`, only while the buffer fills**: not `idle`, and `fw-bytes` above nothing and under 90% of the 64 MB buffer. A full buffer reads as fast as it plays, and an empty one is the server starting. `PlaybackService.measuredBps` carries it to the next play's first variant.
- Seen on Linux behind a throttling proxy, Big Buck Bunny's 1080p H.264 original: at 1.2 Mb/s it stalled, Auto left for the AV1 ladder at 480p (estimate 1.1 Mb/s); at 0.65 Mb/s it stepped to 360p; unthrottled it went up to 720p and back to the original within two minutes.

### Tracks and display

- In-file audio and subtitle tracks switch in libmpv. Only a transcode needs the server: `audioStream`, or `burnSubtitle` for a track it can't deliver as text.
- libass draws subtitles into the picture.
- Picture-in-picture is Android only (`PipService`, `MainActivity.kt`): libmpv draws into a Flutter texture, and iOS's API wants an AVPlayer layer. Leaving the app while something plays enters it.
- HDR on Android forces `mediacodec-copy`, because SurfaceTexture's conversion doesn't know PQ or HLG. This hasn't been run on a real HDR device.

## NoFlex Connect

`features/connect/` controls the viewer's other devices: this app is a remote, not a receiver.

- `ConnectService` follows `connectDevices` (signals, from the shell's session phase) and holds the choice of where Play goes. `PlaybackService.playWork` hands the title to `ConnectService.cast` while a device is chosen; a watch room's still plays here.
- A command names the title, not a file: the device picks its own version. Resume comes from the viewer's place on the server unless `fromStart` or a position says otherwise.
- `RemoteBar` shows whichever device is playing, over the bottom bar on a phone and over the content beside the navigation on wider windows. Position advances between reports from `ConnectService.now`; controls update at once and the device's next report corrects them.
- `ConnectButton` is on a title's actions only while a device is listening, as a cast button is.

## Downloads

- `background_downloader` owns transfers natively: URLSession background tasks on iOS and macOS, WorkManager or UIDT on Android, isolates on Windows and Linux.
- `DownloadsService` keeps a JSON manifest (`OfflineStore`) and mirrors updates into signals.
- `smart_planner.dart` decides what to fetch and evict. The server ranks and enforces the budget (`smartDownloads`); the client enforces Wi-Fi, as the schema says.
- Only what the server put here (`OfflineOrigin.smart`) can be evicted. What the viewer chose stays until they remove it.
- A manual download asks `deviceDownload`. When that's null, the server has nothing ready for this device and an export job can stand in.
- The target folder setting is hidden on iOS. Android lists its app-specific volumes, including SD cards, which need no permission. Desktop gets a folder picker.

### Accounts

- **Downloads belong to an account.** Each has `offline/<user id>/` for its manifest and files, and a folder of its own (`Name [id]`) inside one the viewer chose. A transfer's id starts with the user's, since the downloader is one for the device.
- `DownloadsService` holds the active account's manifest only. An update for anyone else's transfer is dropped, and their manifest catches up from the downloader's records when they're back (`_syncWithDownloader`), as it does after the app was suspended.
- **Locking a profile stops its transfers** (`hold`), and opening it restarts them (`_release`): download URLs carry tokens for the session, which the lock refuses. They keep their URLs, good again once the PIN is in. Unverified against a real downloader.
- A reconcile that finishes after someone else became active stops rather than writing into their manifest.
- The headless sync fetches for the active account, and for nobody while the app sits on "Who's watching?".
- Signing out removes that account's downloads and nobody else's, and so does removing someone whose session ended (`removeAccount`). An ended session keeps them for when they sign in again. Changing server removes everyone's (`removeEveryone`).

### Smart transcode marks

- A mark (`smart_marks_section.dart`, You → Smart transcoding) is a hard signal to the server to prepare a title next. It isn't a download: what comes to the phone is still the Downloads section's business.
- "Mark for smart transcoding" is for every device with smart transcoding on; "Smart transcode for this device" for this one, whatever its setting. The two are independent.

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
- A stale debug-only file (`Unhandled Exception: FormatException` from `NativeReferenceHolder`) after killing the emulator mid-write goes away with `adb shell pm clear dev.noflex.app`.

## Licenses

You → About shows the app's version, its source, and every license in the build: Flutter's registry for Dart packages and the engine, plus what `services/licenses.dart` adds from `assets/licenses/manifest.json`.

- **`assets/licenses/` is generated** by `cargo xtask third-party sync` from `packaging/third-party/apps.toml`. Change the record, never the assets; [`packaging/AGENTS.md`](../packaging/AGENTS.md) says how, including what to do when media_kit moves.
- The record covers what Flutter can't see: the native libraries media_kit downloads prebuilt for each platform, and Inter. A new Dart package needs nothing.
- Linux plays through the system's libmpv, which the app doesn't ship; only mimalloc is linked in.

## Motion and loading

The root `AGENTS.md`'s design language, in this app's pieces.

- Timing comes from `NfMotion`, the web's easings and durations. Check `MediaQuery.disableAnimationsOf` wherever you start motion yourself.
- Controls sink under a press with `NfPressable`, fed by `InkWell.onHighlightChanged` so a scroll taking over the gesture lifts them. Use `Press` and `NfSink` when only part of a control sinks, as `WorkCard` does.
- Material's ripple (`InkSparkle`) stays alongside the sink, and is enough on its own for a plain `InkWell` row. Route transitions stay native too (`pageTransitionsTheme`).
- A card's picture flies into the page it opens with `PosterHero`. Tags are per card, since a title can be in several rails on one page, and only fly when both ends draw the same shape.
- `AmbientGlow` goes inside a page, behind its content. Routes are opaque, so a tint under the shell would never show. Home has none: its hero covers the part of the screen the tint reaches.
- A title still indexing (`WorkSummary.indexing`) gets `IndexingOverlay` from `Artwork`. `IndexingService.latest` is how cards read a title that may have moved on since its list was fetched, and one `worksById` every 4 s covers everything on screen. Not persisted to the offline manifest, and not followed on a linked server.
- Loads draw in the content's shape with `NfSkeleton` and `NfSkeletonRows`. `CircularProgressIndicator` is for something underway rather than arriving: a busy button, a channel starting, buffering, the next page of a list.

## Conventions

- Build UI from `widgets/` and the tokens in `theme.dart`; don't hard-code colors or radii.
- Icons are Phosphor (`package:phosphor_icons`, a maintained fork of `phosphor_flutter`, which doesn't compile on current Flutter), never Material `Icons.*`. `actionIconTheme` covers the back and close arrows the framework draws itself.
- Device brands Phosphor lacks come from `package:simple_icons`, through `utils/device_icon.dart`, a port of `lib/ts/device.ts`. Nowhere else: see the design language in the root `AGENTS.md`.
- Draw SVGs with `jovial_svg`, not `flutter_svg`: flutter_svg ignores `<style>`, which draws most exported logos solid black.
- Fonts are bundled, as there's no fetching them from a CDN in a self-hosted app.
- Inter is a variable font: set weight through `nfText`, since `fontWeight` alone leaves the glyphs at the file's default.
- Wrap user-triggered mutations in `ToastService.attempt`.
- Every source file starts with an SPDX header, generated platform boilerplate included.
- Put Unicode (`’`, `·`, `–`) directly in strings.
