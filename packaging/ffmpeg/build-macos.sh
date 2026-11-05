#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds ffmpeg natively for macOS, where neither BtbN's images nor ours can reach
# VideoToolbox. Each dependency is compiled from the commit pinned in pipeline/
# scripts.d, so every platform ships the same versions, and linked statically so
# nothing is needed from Homebrew at runtime. Needs nasm, cmake, meson, ninja,
# pkg-config, autotools, m4 and svn to build. Runs on Linux too, without VideoToolbox,
# which tests everything here but the one flag. Usage: build-macos.sh COMMIT OUT

set -euo pipefail

commit=$1
out=$(mkdir -p "$2" && cd "$2" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
scripts=$here/pipeline/scripts.d
work=${NOFLEX_FFMPEG_WORK:-$(mktemp -d)}
prefix=$work/prefix
jobs=$(getconf _NPROCESSORS_ONLN)
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
export MACOSX_DEPLOYMENT_TARGET=12.0
mkdir -p "$work" "$prefix"
# Homebrew avoids the name of Apple's unrelated libtool.
if [ "$(uname)" = Darwin ]; then
    export LIBTOOLIZE=glibtoolize
fi

# The value of VAR in a pipeline script, without running the script.
pinned() {
    sed -n "s/^$2=\"\(.*\)\"$/\1/p" "$scripts/$1.sh" | head -1
}

# Fetches a pipeline script's pinned commit into $work/$name and enters it.
enter() {
    local name=$1 script=$2 repo commit_id
    repo=$(pinned "$script" SCRIPT_REPO)
    commit_id=$(pinned "$script" SCRIPT_COMMIT)
    cd "$work"
    if [ ! -d "$name/.git" ]; then
        git init --quiet "$name"
        git -C "$name" fetch --quiet --depth=1 "$repo" "$commit_id"
        git -C "$name" checkout --quiet FETCH_HEAD
    fi
    cd "$name"
    echo "==> $name ${commit_id:0:12}"
}

meson_static() {
    meson setup build --prefix="$prefix" --libdir=lib --buildtype=release \
        --default-library=static "$@"
    ninja -C build -j"$jobs" install
}

cmake_static() {
    cmake -G Ninja -B build -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib \
        -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF "$@"
    ninja -C build -j"$jobs" install
}

enter x264 50-x264
bash ./configure --prefix="$prefix" --enable-static --enable-pic --disable-cli
make -j"$jobs" install

# Two builds folded into one library, because libx265 picks its bit depth at build
# time and HDR needs 10-bit next to the usual 8.
enter x265 50-x265
cmake -G Ninja -S source -B build10 -DCMAKE_BUILD_TYPE=Release -DENABLE_SHARED=OFF -DENABLE_CLI=OFF \
    -DHIGH_BIT_DEPTH=ON -DEXPORT_C_API=OFF -DCMAKE_ASM_NASM_FLAGS=-w-macro-params-legacy
ninja -C build10 -j"$jobs" x265-static
cp build10/libx265.a libx265_main10.a
cmake -G Ninja -S source -B build8 -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib \
    -DCMAKE_BUILD_TYPE=Release -DENABLE_SHARED=OFF -DENABLE_CLI=OFF -DEXTRA_LIB=x265_main10.a \
    -DEXTRA_LINK_FLAGS=-L. -DLINKED_10BIT=ON -DCMAKE_ASM_NASM_FLAGS=-w-macro-params-legacy
cp libx265_main10.a build8/
ninja -C build8 -j"$jobs" install
if [ "$(uname)" = Darwin ]; then
    libtool -static -o "$prefix/lib/libx265.a" build8/libx265.a libx265_main10.a
else
    printf 'create %s\naddlib build8/libx265.a\naddlib libx265_main10.a\nsave\nend\n' \
        "$prefix/lib/libx265.a" | ar -M
fi
# x265 only writes this when it can see git tags, which a shallow fetch doesn't have.
cxx_runtime=-lstdc++
if [ "$(uname)" = Darwin ]; then
    cxx_runtime=-lc++
fi
mkdir -p "$prefix/lib/pkgconfig"
cat >"$prefix/lib/pkgconfig/x265.pc" <<PC
Name: x265
Description: H.265/HEVC video encoder
Version: 4.1
Libs: -L$prefix/lib -lx265 $cxx_runtime -lm -lpthread
Cflags: -I$prefix/include
PC

enter svtav1 50-svtav1
cmake_static -DBUILD_APPS=OFF -DBUILD_TESTING=OFF

enter opus 50-libopus
cmake_static -DOPUS_BUILD_PROGRAMS=OFF -DOPUS_BUILD_TESTING=OFF

echo "==> lame"
cd "$work"
if [ ! -d lame ]; then
    svn checkout --quiet "$(pinned 50-libmp3lame SCRIPT_REPO)@$(pinned 50-libmp3lame SCRIPT_REV)" lame
fi
cd lame
# Its committed configure, because current autoconf rejects its configure.ac. That
# configure's config.guess is too old to know arm64, so name the platform.
build=()
if [ "$(uname)" = Darwin ]; then
    build=(--build=aarch64-apple-darwin)
fi
CFLAGS="${CFLAGS:-} -DNDEBUG" ./configure --prefix="$prefix" --enable-static --disable-shared \
    --disable-cpml --disable-frontend --disable-decoder "${build[@]}"
make -j"$jobs" install

enter dav1d 50-dav1d
meson_static -Denable_tools=false -Denable_tests=false

enter zimg 50-zimg
git submodule update --init --quiet --depth=1
autoreconf -i
./configure --prefix="$prefix" --enable-static --disable-shared
make -j"$jobs" install

enter fribidi 25-fribidi
meson_static -Dbin=false -Ddocs=false -Dtests=false

enter freetype 45-fonts/25-freetype
meson_static -Dbrotli=disabled -Dbzip2=disabled -Dharfbuzz=disabled -Dpng=disabled -Dzlib=disabled

enter harfbuzz 45-fonts/45-harfbuzz
meson_static -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dcairo=disabled \
    -Dicu=disabled -Dtests=disabled -Ddocs=disabled -Dutilities=disabled

# fontconfig is off on purpose on macOS, where libass finds fonts through CoreText.
enter libass 50-libass
fonts=()
if [ "$(uname)" != Darwin ]; then
    fonts=(-Drequire-system-font-provider=false)
fi
meson_static -Dfontconfig=disabled -Dtest=disabled -Dprofile=disabled "${fonts[@]}"

echo "==> ffmpeg ${commit:0:12}"
cd "$work"
if [ ! -d ffmpeg/.git ]; then
    git init --quiet ffmpeg
    git -C ffmpeg fetch --quiet --depth=1 https://github.com/FFmpeg/FFmpeg.git "$commit"
    git -C ffmpeg checkout --quiet FETCH_HEAD
fi
cd ffmpeg

if [ "$(uname)" = Darwin ]; then
    platform=(--enable-videotoolbox --enable-audiotoolbox --install-name-dir=@rpath
        --extra-ldexeflags=-Wl,-rpath,@executable_path/../lib)
else
    platform=(--extra-libs=-lm '--extra-ldexeflags=-Wl,-rpath,\\\$\$ORIGIN/../lib')
fi
./configure --prefix="$work/ffmpeg-out" --pkg-config-flags=--static \
    --enable-gpl --enable-version3 --disable-debug --disable-doc --disable-ffplay \
    --enable-shared --disable-static --extra-version=noflex \
    --extra-cflags="-I$prefix/include" --extra-ldflags="-L$prefix/lib" \
    --enable-libx264 --enable-libx265 --enable-libsvtav1 --enable-libopus --enable-libmp3lame \
    --enable-libdav1d --enable-libzimg --enable-libass --enable-libfreetype \
    --enable-libharfbuzz --enable-libfribidi "${platform[@]}"
make -j"$jobs"
make install

mkdir -p "$out/bin" "$out/lib"
cp -RP "$work/ffmpeg-out/bin/." "$out/bin/"
find "$work/ffmpeg-out/lib" -maxdepth 1 \( -name '*.dylib' -o -name '*.so*' \) -exec cp -RP {} "$out/lib/" \;
cp COPYING.GPLv3 "$out/LICENSE.txt"
