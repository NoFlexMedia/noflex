#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Compiles ffmpeg in the image build-images.sh made for a target, which holds every
# dependency cross-compiled for that target's oldest supported libc. Usage: build.sh
# IMAGE FFMPEG_COMMIT NVENC_SDK OUT

set -euo pipefail

image=$1 commit=$2 sdk=$3 out=$4
mkdir -p "$out"

engine=$(command -v podman || command -v docker)

"$engine" run --rm -i \
    -e FFMPEG_COMMIT="$commit" -e NVENC_SDK="$sdk" \
    -v "$(realpath "$out")":/out \
    "$image" bash -s <<'BUILD'
set -euo pipefail
cd /tmp

# These headers alone set the oldest Nvidia driver the result runs on, so don't let
# a stale image raise it quietly.
got=$(pkg-config --modversion ffnvcodec)
case "$got" in
    "$NVENC_SDK".*) ;;
    *)
        echo "expected NVENC SDK $NVENC_SDK, the image has $got" >&2
        exit 1
        ;;
esac

git init --quiet ffmpeg
cd ffmpeg
git fetch --quiet --depth=1 https://github.com/FFmpeg/FFmpeg.git "$FFMPEG_COMMIT"
git checkout --quiet FETCH_HEAD

# Escaped once for make, once for the shell make runs, so the linker sees $ORIGIN.
rpath=''
case "$FFBUILD_TOOLCHAIN" in
    *linux*) rpath='-Wl,-rpath,\\\$\$ORIGIN/../lib' ;;
esac

./configure --prefix=/tmp/prefix --pkg-config-flags=--static $FFBUILD_TARGET_FLAGS $FF_CONFIGURE \
    --extra-cflags="$FF_CFLAGS" --extra-cxxflags="$FF_CXXFLAGS" --extra-libs="$FF_LIBS" \
    --extra-ldflags="$FF_LDFLAGS" --extra-ldexeflags="$FF_LDEXEFLAGS $rpath" \
    --cc="$CC" --cxx="$CXX" --ar="$AR" --ranlib="$RANLIB" --nm="$NM" \
    --disable-ffplay --disable-doc --extra-version=noflex >/dev/null
make -j"$(nproc)" >/dev/null
make install >/dev/null

mkdir -p /out/bin /out/lib
cp -a /tmp/prefix/bin/. /out/bin/
find /tmp/prefix/lib -maxdepth 1 -name '*.so*' -exec cp -a {} /out/lib/ \;
cp COPYING.GPLv3 /out/LICENSE.txt

# Output belongs to whoever owns the mount, however the engine maps users.
chown -R --reference=/out /out
BUILD
