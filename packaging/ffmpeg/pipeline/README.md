# ffmpeg build pipeline

Based on [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) at
`c63bb308a1de656b425c132a8102f8fd5389afc5`, under its MIT license (`LICENSE`). The files here keep that
license rather than NoFlex's. Run it through `cargo xtask ffmpeg-build`; see
[`docs/PACKAGING.md`](../../../docs/PACKAGING.md).

## Differences from upstream

- `scripts.d/50-ffnvcodec.sh` uses NVENC SDK 12.0 for every branch, which is what sets
  the oldest Nvidia driver we support.
- `images/` pins crosstool-NG, which upstream takes from `HEAD`.
- `../build-images.sh` replaces `makeimage.sh`: plain `build` rather than `buildx bake`,
  so it works under Podman as well as Docker, and it tags images by a hash of this
  directory. Buildah doesn't reuse layers across `RUN --mount` steps the way BuildKit
  does, so that hash is what makes rebuilds cheap.
- `../build.sh` replaces `build.sh`, and fails if the image's NVENC SDK isn't the pinned one.
- `download.sh` and `generate.sh` find bash through `env`, and `download.sh` handles
  rootless Podman.
- Only `linux64`, `linuxarm64` and `win64` are kept, in the `gpl-shared` variant.

## Still floating

Each dependency and the toolchain are pinned, but rebuilding from scratch also takes
the current `ubuntu:26.04` image and its packages, `Implib.so`, and rustup's stable Rust.
