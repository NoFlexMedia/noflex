# Packaging

`packaging/` is what turns the source into what people install, and the pins behind it. [`docs/PACKAGING.md`](../docs/PACKAGING.md) is the operator's view; `cargo xtask` (in `backend/xtask/`) runs everything here.

## Layout

```
ffmpeg/          # ffmpeg.toml and the pipeline behind `cargo xtask ffmpeg-build` and `ffmpeg-source`
flutter/build.sh # one Flutter target into dist/, as the release workflow runs it
linux/           # the systemd unit, account and config every Linux bundle carries; nfpm.yaml for .deb and .rpm
container/       # the image around the Linux bundles, and compose.yaml for the docs
windows/         # the Inno Setup installer and installer.sh, which compiles it
macos/           # the server bundle on Apple Silicon
smoke/           # what every bundle has to pass on its own platform
third-party/     # apps.toml and notices/: third-party code the apps ship
model.toml, onnxruntime.toml  # what a server bundle downloads, by hash
```

## The server's installs

- **Configure the server in its config file, not its environment,** wherever an operator can edit it: a `NOFLEX_*` variable outranks the file, so a unit that sets one silently overrides what's written there. The container is the exception, as its paths are fixed.
- **Every `NOFLEX_*` variable in `nix/`, `linux/`, `container/` and `windows/` must be one the server reads.** `packaging_sets_only_variables_the_server_reads` in noflex-server's `config.rs` fails otherwise, which is the bug that left the NixOS module's settings ignored. That includes comments, so don't name a variable for anything else there.
- Linux bundles install to `/opt/noflex`; the unit, the packages and the docs all assume it.
- Port 28793 is in the installer's firewall rule and Start menu link, the container's `EXPOSE` and compose file, and each config: change them together with the server's default.
- `.sh` files stay LF (`.gitattributes`), since Windows runners run them in Git Bash.
- Windows: Inno Setup **6** (`is-6_7_4`, by hash in `release.yml`). 7's compiler calls itself non-commercial. The installer's `AppId` never changes; Windows matches upgrades by it.
- Windows has no Nix: its release build uses rustup and MSVC. `docs/PACKAGING.md` says how to check a change with zig and Wine instead.

## Third-party code in the apps

Flutter's license registry covers Dart packages and the engine by itself. `third-party/apps.toml` records what it can't see: the native libraries media_kit downloads prebuilt for each platform, and the Inter font.

| Command | Network | What it does |
| --- | --- | --- |
| `cargo xtask third-party check` | no | Fails if `flutter/pubspec.lock` pins another media_kit commit than `media_kit` names, the record names something missing, an iOS or macOS build lists a GPL library, or `flutter/assets/licenses/` differs from what the record makes. A test (`the_record_matches_the_app`) runs it, so CI does, and the release workflow runs it before building. |
| `cargo xtask third-party sync` | yes | Writes `flutter/assets/licenses/`: every text, and `manifest.json`, which `services/licenses.dart` reads. Never edit those files by hand. |
| `cargo xtask third-party source <out>` | yes | Gathers each build's scripts and every pinned library source, with a README. The release workflow attaches it as the LGPL's corresponding source. Prints the sha256 of any `url` that lacks one. |

### The record

- `[library.<id>]`: `name`, `license` as SPDX, and its text from exactly one of `notice` (a file in `third-party/notices/`, plus the shared `texts` it names), `files` (copied from its first pinned source) or `license_urls` (branch heads, for a library no build pins).
- LGPL and Apache libraries get a `notice`: their holders, and which license this build uses them under. mpv's own `Copyright` file describes its GPL default, not our `-Dgpl=false` build.
- `[build.<id>]`: the `platforms` it ships on, its `source` (the build's scripts and patches, also corresponding source), and a `[[build.<id>.library]]` per library, with `version`, `url` and `sha256` when the build pins it.
- `url` is the archive the build actually fetched, or the same version's release tarball when it cloned a tag. Never a GitLab generated archive: its hash isn't stable.
- One text per library across platforms, from the first build that pins its source, in build name order.

### When media_kit moves

`check` fails naming the new commit. This is reading other projects' build scripts, so it's judgment rather than code:

1. Set `media_kit` to the commit `flutter/pubspec.lock` resolves.
2. Find each platform package's pinned build in media_kit at that commit: `MPV_XCFRAMEWORKS_VERSION` in `libs/ios/…/Makefile` and `libs/macos/…/Makefile`, the jar URLs in `libs/android/…/build.gradle`, `LIBMPV_URL` and `MIMALLOC_URL` in the Windows and Linux `CMakeLists.txt`. Unchanged pins need nothing.
3. Read each changed build at its new tag and record what ships:
   - **iOS, macOS:** the frameworks in media_kit's `Package.swift`, matched to libmpv-darwin-build's `packages.lock.nix`, whose `url` and `sha256` copy across as they are.
   - **Android:** mpv's dependency tree for the flavor (`dep_*` in `include/depinfo.sh`, without `ENCODERS_GPL`), at the versions there; `include/download-deps.sh` names the repositories.
   - **Windows:** the DLL itself, since the build floats: mpv's `List of enabled features` (`strings libmpv-2.dll`), FFmpeg's flags in `packages/ffmpeg.cmake`, and what each links in `packages/*.cmake`. Most libraries can't be pinned.
4. Keep the flavors LGPL: mpv with `-Dgpl=false`, FFmpeg without `--enable-gpl`. `check` only sees the SPDX ids you write, so a GPL flavor recorded as LGPL passes it.
5. `source <dir>` for any new `sha256`, then `sync`, and commit the record and `flutter/assets/licenses/` together.

### Gaps

- **Windows has no exact source** for most of its libraries, so `source` can only list them. Building that DLL ourselves from pinned inputs is the fix.
- The server's ffmpeg has its own source archive, `cargo xtask ffmpeg-source`, separate from the apps'. At about 2 GiB it goes up in parts.
