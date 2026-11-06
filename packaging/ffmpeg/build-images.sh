#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds the image ffmpeg compiles in: the base tools, a cross-toolchain for the
# target's oldest supported libc, then each dependency. Takes an hour or more, so the
# image is tagged with a hash of what made it and reused until pipeline/ or
# backend/rust-toolchain.toml changes. The last line of output names it.
# Usage: build-images.sh TARGET, where TARGET is linux64, linuxarm64 or win64.

set -euo pipefail

target=$1
cd "$(dirname "$0")/pipeline"
rust_toolchain=../../../backend/rust-toolchain.toml
rust_version=$(awk -F '"' '/^channel = / { print $2 }' "$rust_toolchain")
: "${rust_version:?Missing Rust toolchain pin}"
engine=$(command -v podman || command -v docker)

source util/vars.sh "$target" gpl-shared 9.0

hash=$(find addins images patches scripts.d util variants download.sh generate.sh "$rust_toolchain" -type f -print0 |
  LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16)
tagged="${IMAGE%:latest}:$hash"
if "$engine" image inspect "$tagged" >/dev/null 2>&1; then
  echo "$tagged"
  exit 0
fi

"$engine" build --build-arg "RUST_VERSION=$rust_version" -t "$BASE_IMAGE" images/base
"$engine" build --build-arg "GH_REPO=$REGISTRY/$REPO" -t "$TARGET_IMAGE" "images/base-$target"
./download.sh
./generate.sh "$target" gpl-shared 9.0
"$engine" build --jobs="$(nproc)" -t "$tagged" .
echo "$tagged"
