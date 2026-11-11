# Packaging

A release bundle is a directory, and a `.tar.zst` of it, holding the server and
everything it would otherwise need from the host:

```
noflex-server
plugins/      first-party plugins, as .wasm
ffmpeg/       ffmpeg, ffprobe and their libraries
onnxruntime/  ONNX Runtime, CPU build
assets/       the embedding model
```

The server looks beside its own executable first, so unpacking and running it is
the whole install. A plugin, `FFMPEG_BIN`, or `search.onnxruntime_library` on the
host overrides what's in the bundle.

```sh
cargo xtask bundle x86_64-unknown-linux-gnu [--ffmpeg <dir>] [--web <dir>]
```

`--ffmpeg` and `--web` reuse a finished ffmpeg build and web client, as the
release does. A Linux bundle also carries `linux/`: the systemd unit, its
account for sysusers, and the service's config. Windows bundles are zips.
NixOS doesn't use bundles: the flake's package pins all of this through nixpkgs.

## Status

| Target                      | Server | ffmpeg | ONNX Runtime | Packaged as | State |
| --------------------------- | ------ | ------ | ------------ | ----------- | ----- |
| `x86_64-unknown-linux-gnu`  | yes    | yes    | yes          | tar.zst, .deb, .rpm, image | Built and run; packages installed on Debian 13 and Fedora 43 |
| `aarch64-unknown-linux-gnu` | yes    | release | yes         | tar.zst, .deb, .rpm, image | Release workflow not yet run |
| `x86_64-pc-windows-msvc`    | yes    | release | yes         | zip, installer | GNU build run under Wine, as a service and through the installer; MSVC build not yet run |
| `aarch64-apple-darwin`      | CI     | CI     | yes          | tar.zst     | Native recipe, not yet run on a Mac |

The release workflow, `.github/workflows/release.yml`, builds every target but
macOS's on a tag: ffmpeg once per target, cached on a hash of the pipeline, then a
bundle on each target's own runner, smoke-tested there outside any Nix shell
(`packaging/smoke/`), then packaged. It attaches everything to a draft release with
`SHA256SUMS` and a provenance attestation, and pushes the container image.

## Linux packages

`packaging/linux/package.sh` turns a bundle into a .deb and an .rpm with
[nfpm](https://nfpm.goreleaser.com/), from `packaging/linux/nfpm.yaml`: the bundle
in `/opt/noflex`, the unit, the account, and the config as a conffile. The
bundle's ffmpeg brings its own libraries, so the packages depend only on glibc
2.28 and libstdc++, and recommend what ffmpeg loads when it finds it: libva, Mesa's
VAAPI drivers and a Vulkan loader. They're attached to the release; there's no
apt or dnf repository yet.

## Container image

`packaging/container/Containerfile` wraps the Linux bundles for amd64 and arm64
in `debian:trixie-slim`, with what the bundle's ffmpeg looks for on a host: VAAPI
with Mesa's and Intel's drivers, the oneVPL runtime for QSV on x86, Vulkan for
tone-mapping, and the NVIDIA Container Toolkit's variables for Nvidia's. It's
about 430 MB before the bundle, mostly Mesa. tini is PID 1, to reap.

## Windows

The server runs as a service (`noflex-server service install|uninstall|run`,
`noflex-server/src/service.rs`), and keeps its system tier in `%ProgramData%\NoFlex`
(`noflex_core::system_dirs`). `packaging/windows/noflex.iss` is the installer,
compiled with Inno Setup 6 by `packaging/windows/installer.sh`. Inno Setup 7's
compiler calls itself non-commercial, so the release pins 6.7.4, checked against
its hash. Release builds use rustup and MSVC on a Windows runner, the other
exception to the flake being the only toolchain.

Checking a change without Windows: `cargo zigbuild --target x86_64-pc-windows-gnu
-p noflex-server` builds the server with a toolchain that has that target and
`windres` on `PATH` (Turso's build script compiles a version resource with it), and
Wine runs the result, the service and the installer included. Wine can't set a
service to restart after a non-crash failure, which `service install` warns about.

## ffmpeg

We build ffmpeg ourselves. `packaging/ffmpeg/pipeline/` is based on
[BtbN's FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), which is MIT licensed:
one script per dependency, each pinning a commit, compiled in a container against a
cross-toolchain for the target's oldest libc. `cargo xtask ffmpeg-build` builds that
image the first time, which takes an hour or more, tags it with a hash of `pipeline/`,
and reuses it until something there changes. Nothing is pulled from BtbN at build time.

The change that started this:

**Those headers alone set the oldest Nvidia driver that works.** FFmpeg compares
the SDK it compiled against with what the driver reports, and refuses if the driver
is older. FFmpeg's own version doesn't decide this: 9.0 builds against any SDK from
11.1 up. BtbN uses the newest:

| ffmpeg       | Headers    | Oldest driver (Linux) | AV1 NVENC | On driver 595 |
| ------------ | ---------- | --------------------- | --------- | ------------- |
| BtbN 9.0     | `master`   | 610                   | yes       | refused |
| BtbN 8.1     | `sdk/13.0` | 570                   | yes       | works |
| **Ours 9.0** | `sdk/12.0` | **520.56**            | **yes**   | **works** |
| not built    | `sdk/11.1` | 470.57                | no        | |

SDK 12.0 is the oldest with AV1, and every AV1-capable GPU needs a driver that new
anyway. Only Kepler-era cards, stuck on the 470 legacy driver, fall back to software
encoding. What 12.0 lacks (UHQ tuning, temporal filter, 4:2:2, MV-HEVC) NoFlex
doesn't ask for; raise the headers only for a feature we'd actually use.

`packaging/ffmpeg/ffmpeg.toml` pins the ffmpeg commit, and `pipeline/scripts.d` each
dependency. Dependency pins are taken alongside a commit of ffmpeg's release branch,
whose APIs they can outrun, so bump them together. `ffmpeg-verify` checks a build has
every encoder, filter and muxer the transcoder can ask for.

`pipeline/README.md` lists how it differs from upstream, and what still floats.

### What the host still provides

ffmpeg loads these at runtime, and does without them when they're absent:

- **NVENC:** the Nvidia driver, 520.56 or newer (Windows 522.25).
- **VAAPI:** `libva` and a driver for the GPU. BtbN's images load libva lazily, and
  ffmpeg aborts rather than errors if it's missing, but only when VAAPI is tried,
  which hardware detection survives.
- **QSV:** Intel's oneVPL runtime.
- **AMF:** AMD's AMF runtime, which ships with the Adrenalin driver on Windows
  and only the proprietary AMDGPU-PRO stack on Linux, where Mesa's VAAPI is the
  usual path.
- **libplacebo tone-mapping:** a Vulkan loader and driver.

### License

The `gpl` variant is required for libx264 and libx265, so a bundle ships GPL-3.0
software alongside AGPL-3.0 NoFlex, as separate programs. Distributing our own build
means offering its source: `cargo xtask ffmpeg-source <out>` gathers ffmpeg at the
pinned commit, every dependency as `pipeline/` fetches it, and the build scripts,
about 2 GiB, and the release attaches it in parts: `cat noflex-*-ffmpeg-source.tar.* | tar -x`.

## macOS

Apple Silicon only, because Microsoft publishes no Intel ONNX Runtime for macOS.
`packaging/macos/build-and-test.sh` builds and exercises the bundle on any Apple
Silicon Mac, and `.github/workflows/macos.yml` runs it on a GitHub runner, because
neither Apple's linker and SDK nor VideoToolbox exist anywhere else.

`packaging/ffmpeg/build-macos.sh` compiles the same dependency commits as the Linux
images, natively and statically, so the versions can't drift. It runs on Linux too,
without VideoToolbox, to test changes without using a Mac runner. Tone-mapping stays
on the CPU, since libplacebo would need MoltenVK.

Release builds use rustup and Xcode rather than the Nix flake, the one exception to
the flake being the only toolchain: a binary linked by Nix's points into `/nix/store`,
and `packaging/macos/check-linkage.sh` fails the build if anything does. `search.device`
reaches the GPU or Neural Engine through CoreML.

## Linking the server

A binary linked in the Nix shell only runs where that Nix store exists, so Linux
bundles link through [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild)
against glibc 2.28, the same floor as the ffmpeg images. musl isn't an option:
loading ONNX Runtime needs `dlopen`.

Microsoft's ONNX Runtime needs the host's `libstdc++`, which a minimal container
won't have.

## Embedding acceleration

Bundles ship ONNX Runtime's CPU build, because that's what runs everywhere and the
accelerated ones are large. To use an accelerator, install a build of ONNX Runtime
made for it, then set `search.device` and `search.onnxruntime_library`
(see [`CONFIGURATION.md`](CONFIGURATION.md)):

- **Nvidia GPU:** Microsoft's `gpu_cuda` release, plus CUDA and cuDNN libraries of
  the version it names. Measured on an RTX 4090: 434 documents a second against 13.5
  on 16 CPU threads. It shares the GPU with hardware transcodes.
- **Intel NPU:** the `onnxruntime-openvino` build. The model ships a fixed-shape
  graph for it, and the host embeds one sequence at a time. Not yet run on real
  hardware.
- **Qualcomm NPU:** a QNN build. Not yet run on real hardware.

The Indexing tab shows where the model actually loaded.

## Client apps

`.github/workflows/release.yml` builds the Flutter app for every client target, and
`packaging/flutter/build.sh <target>` does the work, so any machine that can build a
target runs the same thing. Pushing a `v*` tag attaches the builds to a draft release
to review and publish; running the workflow by hand leaves them as artifacts. The tag
sets the version, and the run number the build number.

| Target    | Output                         | Built on     | State |
| --------- | ------------------------------ | ------------ | ----- |
| `android` | one APK per ABI, plus an AAB when signed | Nix shell | Built, with the release key and without |
| `linux`   | `tar.gz` of the bundle         | Ubuntu 24.04 | Built on clean Ubuntu 24.04; starts there and on Fedora 43 |
| `windows` | `zip` of the bundle            | Windows      | Not yet run |
| `macos`   | `zip` of the app, ad-hoc signed | macOS       | Not yet run |
| `ios`     | unsigned `ipa`, for sideloading | macOS       | Not yet run |

Only Android builds in the Nix shell, the `flutter-android` one, since its output
doesn't link against the host. The others use the same Flutter version installed
directly, for the reason under [Linking the server](#linking-the-server), and the
workflow fails if that version drifts from the flake's.

**Linux** needs these from the host, which a desktop install usually has apart from
libmpv: GTK 3, `libmpv.so.2`, `libsecret`, `libepoxy`, EGL and GLES (Mesa or a GPU driver),
`xdg-user-dirs`, and a Secret Service such as GNOME Keyring to stay signed in. It's built on
Ubuntu 24.04 because the video plugin links whichever libmpv it finds, and older releases
only have `libmpv.so.1`, which current distributions don't ship. That sets a glibc floor of
2.38, so Debian 12 is too old.

**Android** signs with the debug key unless the repository has these secrets, and those
builds can't update an install from another build. Create the key once and keep a copy,
since Android refuses updates signed by any other:

```sh
keytool -genkeypair -keystore release.jks -alias noflex -keyalg RSA -keysize 4096 -validity 10000
base64 -w0 release.jks | gh secret set ANDROID_KEYSTORE_BASE64
gh secret set ANDROID_KEYSTORE_PASSWORD
gh secret set ANDROID_KEY_ALIAS
gh secret set ANDROID_KEY_PASSWORD
```

Locally, `flutter/android/key.properties` with `storeFile`, `storePassword`, `keyAlias`
and `keyPassword` does the same, and git ignores it.

**Third-party source.** The apps ship native libraries media_kit downloads prebuilt, mpv
and FFmpeg among them, under the LGPL, which asks for their source beside the builds.
`packaging/third-party/apps.toml` records what each platform ships, and the release
attaches `noflex-<version>-third-party-source.tar`: each build's scripts and every library
source it pins, gathered by `cargo xtask third-party source`. The same record writes the
apps' license screen. [`packaging/AGENTS.md`](../packaging/AGENTS.md) says how to keep it.

**Windows, macOS and iOS** builds aren't signed. Windows shows a SmartScreen warning,
macOS quarantines the app until it's notarized, and iOS only installs through a
sideloading tool. Signing needs a certificate for Windows and an Apple Developer account
for the others.

## Still open

- **macOS signing.** Bundles are ad-hoc signed by the linker. A downloaded one is
  quarantined by Gatekeeper until it's notarized, which needs an Apple Developer
  account.
- **The Windows app's exact third-party source.** media_kit's Windows libmpv builds most of
  its libraries from branch heads, so the archive can only name them. Building that DLL
  from pinned inputs, as the server's ffmpeg is, would close it.
- **The first release run.** No tag has run the server jobs yet: the cold ffmpeg
  builds, the arm64 runner, the MSVC build and the installer on a real Windows.
- **Windows signing.** The installer and server aren't signed, so SmartScreen warns.
- **An apt and dnf repository**, so packages update with the system.
- **Windows: one Wine-only failure to look into.** Started by the installer under
  Wine, the service logs `smart transcoding: planning failed error=background task
  failed`, a `tokio::fs` task that never ran, without a panic. Run in the
  foreground, it doesn't. Check on a real Windows.
- **Publishing a winget manifest** after the first release, with `wingetcreate new`
  on the installer's URL.

## TV app

`just tv package <target>` builds one target of `solidtv/` into `dist/tv/`. It isn't in the release workflow yet, since most targets haven't run on a real device. [`solidtv/AGENTS.md`](../solidtv/AGENTS.md) says what each has been verified on.

| Target     | Output                       | Needs | Install on a device |
| ---------- | ---------------------------- | ----- | ------------------- |
| `tizen`    | signed `.wgt`                | Samsung's `tizen` CLI and a certificate made with it; without them, stops at `solidtv/dist/tizen` | Developer mode on the TV (`12345` in Apps), then `tizen install` |
| `webos`    | `.ipk`                       | nothing beyond `just tv install` | Developer Mode app, then `npx ares-install` from `solidtv/` |
| `android`  | APK, debug-signed            | the `flutter-android` shell | `adb install`; covers Android TV, Google TV and Fire TV |
| `firebolt` | `tar.gz` to host at a URL    | nothing | Register the URL with the RDK platform |
| `browser`  | `tar.gz` to host at a URL    | nothing | Also how to reach TVs with no store of their own: Vizio, Hisense, Titan OS, Xbox's browser |
| `tvos`     | built by `ns`                | a Mac with Xcode, the tvOS platform, and the `xcodeproj` gem | `just tv run tvos --device <id>`, with a team ID in `App_Resources/tvOS/build.xcconfig` |
| `ios`      | built by `ns`                | the same | The same, under `App_Resources/iOS` |

The server has to allow cross-origin requests for every target except `tvos` and `ios`, which is `cross_origin`'s default. Every web host runs from another origin, or from `file://` with none.
