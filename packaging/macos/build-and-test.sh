#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds the Apple Silicon bundle on this Mac and exercises it: linkage, real encodes,
# the real model through the plugin host, the server starting from an empty directory,
# then a benchmark. Used by CI and by hand. Uses rustup rather than Nix, even if Nix is
# installed, because a binary linked by Nix's toolchain only runs where its store does.
# Usage: build-and-test.sh [cargo profile, default dev]

set -euo pipefail

profile=${1:-dev}
target=aarch64-apple-darwin
root=$(cd "$(dirname "$0")/../.." && pwd)
bundle=$root/dist/noflex-$target
export MACOSX_DEPLOYMENT_TARGET=12.0
export RUST_MIN_STACK=8388608

if [ "$(uname -sm)" != "Darwin arm64" ]; then
    echo "needs an Apple Silicon Mac; this is $(uname -sm)" >&2
    exit 1
fi
missing=()
for tool in brew rustup node npm python3 curl; do
    command -v "$tool" >/dev/null || missing+=("$tool")
done
if [ "${#missing[@]}" -gt 0 ]; then
    echo "install first: ${missing[*]} (brew install rustup node)" >&2
    exit 1
fi

echo "==> build tools"
brew install --quiet nasm cmake meson ninja pkg-config autoconf automake libtool subversion zstd

echo "==> Rust"
cd "$root/backend"
rustup toolchain install
PATH="$(dirname "$(rustup which cargo)"):$PATH"
export PATH
# Nothing from a Nix dev shell may reach the compiler or linker.
unset RUSTC_WRAPPER NIX_CFLAGS_COMPILE NIX_LDFLAGS SDKROOT CC CXX AR LD

if [ ! -x "$root/dist/ffmpeg-macos/bin/ffmpeg" ]; then
    cargo run -q -p xtask -- ffmpeg-build "$target" "$root/dist/ffmpeg-macos"
fi
cargo run -q -p xtask -- bundle "$target" --ffmpeg "$root/dist/ffmpeg-macos" --profile "$profile"

echo "==> linkage"
"$root/packaging/macos/check-linkage.sh" "$bundle"

echo "==> ffmpeg"
"$root/packaging/macos/smoke-ffmpeg.sh" "$bundle/ffmpeg/bin/ffmpeg"

echo "==> real model through the plugin host"
ORT_DYLIB_PATH="$bundle/onnxruntime/lib/libonnxruntime.dylib" \
    NOFLEX_EMBEDDINGS_TEST_ASSETS="$bundle/assets" \
    cargo test -p noflex-plugin-host --test it real_plugin::loads_real_embeddings_plugin -- --nocapture

echo "==> server from the bundle"
"$root/packaging/macos/smoke-server.sh" "$bundle"

echo "==> benchmark"
bench=$root/dist/bench
mkdir -p "$bench"
python3 -m venv "$bench/venv"
"$bench/venv/bin/pip" install -q onnxruntime tokenizers numpy
if [ ! -d "$bench/scifact" ]; then
    curl -fsSL -o "$bench/scifact.zip" https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/scifact.zip
    unzip -q "$bench/scifact.zip" -d "$bench"
fi
sysctl -n machdep.cpu.brand_string hw.physicalcpu hw.memsize
threads=$(sysctl -n hw.physicalcpu)
for provider in CPUExecutionProvider CoreMLExecutionProvider; do
    "$bench/venv/bin/python" -I "$root/packaging/bench/embeddings.py" \
        "$bundle/assets/embeddinggemma-2" "$bench/scifact" "$provider" "$threads" ||
        echo "$provider unavailable"
done

echo "==> done: $root/dist/noflex-$target.tar.zst"
