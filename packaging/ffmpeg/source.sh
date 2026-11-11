#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Gathers the corresponding source of the ffmpeg every bundle ships, which the GPL
# asks a release to offer: ffmpeg at the pinned commit, every dependency as its
# script in pipeline/scripts.d pins it, and the scripts that build them all.
# download.sh fetches the dependencies inside the pipeline's base image, so this
# needs Docker or Podman. Usage: source.sh FFMPEG_COMMIT OUT

set -euo pipefail

commit=$1
mkdir -p "$2"
out=$(cd "$2" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
engine=$(command -v podman || command -v docker)
rust_version=$(awk -F '"' '/^channel = / { print $2 }' "$here/../../backend/rust-toolchain.toml")

cd "$here/pipeline"
# shellcheck disable=SC1091 # vendored; sets BASE_IMAGE
source util/vars.sh dl only
"$engine" build --build-arg "RUST_VERSION=$rust_version" -t "$BASE_IMAGE" images/base
./download.sh

rm -rf "${out:?}"/*
mkdir -p "$out/dependencies"
# download.sh links each stage's name to the archive of what it pins now.
find .cache/downloads -maxdepth 1 -type l -name '*.tar.xz' -exec cp -L {} "$out/dependencies/" \;

work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
git init --quiet "$work/ffmpeg"
git -C "$work/ffmpeg" fetch --quiet --depth=1 https://github.com/FFmpeg/FFmpeg.git "$commit"
git -C "$work/ffmpeg" archive --prefix="ffmpeg-$commit/" FETCH_HEAD | xz -T0 >"$out/ffmpeg-$commit.tar.xz"

tar -C "$here/.." --exclude=pipeline/.cache -cf - ffmpeg | tar -C "$work" -xf -
mv "$work/ffmpeg" "$out/build"
cat >"$out/README.txt" <<README
The source of the ffmpeg in NoFlex's server bundles, under the GPL-3.0.

ffmpeg-$commit.tar.xz
    FFmpeg at the commit build/ffmpeg.toml pins.
dependencies/
    Every library it links, one archive per script in build/pipeline/scripts.d,
    as that script fetches it.
build/
    The scripts that build them: build/pipeline for Linux and Windows,
    build/build-macos.sh for macOS.

build/pipeline/README.md says how it builds, and what it takes from the network
beyond these archives.
README
du -sh "$out"/* >&2
