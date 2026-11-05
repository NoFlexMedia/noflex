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
cargo xtask bundle x86_64-unknown-linux-gnu
```

NixOS doesn't use bundles: the flake's package pins all of this through nixpkgs.

## Status

| Target                      | Server | ffmpeg | ONNX Runtime | State |
| --------------------------- | ------ | ------ | ------------ | ----- |
| `x86_64-unknown-linux-gnu`  | yes    | yes    | yes          | Built and run |
| `aarch64-unknown-linux-gnu` | yes    | scripted | to pin     | Pipeline target not yet built |
| `x86_64-pc-windows-msvc`    | ?      | scripted | to pin     | Pipeline target not yet built, nor the server |
| `aarch64-apple-darwin`      | CI     | CI     | yes          | Native recipe, not yet run on a Mac |

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
- **libplacebo tone-mapping:** a Vulkan loader and driver.

### License

The `gpl` variant is required for libx264 and libx265, so a bundle ships GPL-3.0
software alongside AGPL-3.0 NoFlex, as separate programs. Distributing our own build
means offering its source: `ffmpeg.toml` and `pipeline/` identify it, and a release
should publish the corresponding source with the binaries.

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

## Still open

- **macOS signing.** Bundles are ad-hoc signed by the linker. A downloaded one is
  quarantined by Gatekeeper until it's notarized, which needs an Apple Developer
  account.
- **Linux and Windows CI.** `ffmpeg-build` compiles from pinned inputs, so a release
  workflow can cache on the hash of `pipeline/`. It needs writing, along with a place to
  publish the corresponding source.
