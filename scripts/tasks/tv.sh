#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds and packages the TV app for each thing that can run it. Run it through
# `just tv`. See solidtv/AGENTS.md for what each target is and what it needs.

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
tv=$root/solidtv
out=$root/dist/tv
web_targets=(browser tizen webos android firebolt)

die() {
  echo "tv: $*" >&2
  exit 1
}

install() {
  (cd "$tv" && npm install)
  if [ "${1:-}" = apple ]; then
    (cd "$tv/hosts/nativescript" && npm install)
  fi
}

build_one() {
  (cd "$tv" && npx vite build --mode "$1")
  # What the store's tooling reads sits beside the bundle.
  case $1 in
  tizen) cp "$tv/hosts/tizen/config.xml" "$tv/hosts/tizen/icon.png" "$tv/dist/tizen/" ;;
  webos) cp "$tv/hosts/webos/appinfo.json" "$tv/hosts/webos/icon.png" "$tv/hosts/webos/largeIcon.png" "$tv/dist/webos/" ;;
  esac
}

build() {
  local target=${1:-all}
  if [ "$target" = all ]; then
    for each in "${web_targets[@]}"; do
      build_one "$each"
    done
  else
    build_one "$target"
  fi
}

# A signed .wgt takes Samsung's own CLI and a certificate made with it, neither of
# which can be had from nixpkgs or npm.
package_tizen() {
  build_one tizen
  if ! command -v tizen >/dev/null; then
    echo "tv: solidtv/dist/tizen is ready, but signing needs Samsung's tizen CLI on PATH; see solidtv/AGENTS.md" >&2
    return
  fi
  (cd "$tv/dist/tizen" && tizen package -t wgt ${TIZEN_PROFILE:+-s "$TIZEN_PROFILE"} -o "$out")
}

package_webos() {
  build_one webos
  (cd "$tv" && npx ares-package dist/webos --outdir "$out" --no-minify)
}

package_android() {
  command -v gradle >/dev/null || die "gradle isn't on PATH; enter the flutter-android shell (nix develop .#flutter-android)"
  build_one android
  (cd "$tv/hosts/android" && gradle --no-daemon assembleRelease)
  cp "$tv/hosts/android/app/build/outputs/apk/release/"*.apk "$out/noflex-tv-android.apk"
}

# Hosted, so what ships is a folder to put behind a URL.
package_static() {
  build_one "$1"
  tar -C "$tv/dist/$1" -czf "$out/noflex-tv-$1.tar.gz" .
}

package_apple() {
  [ "$(uname -s)" = Darwin ] || die "$1 builds need a Mac with Xcode"
  (cd "$tv/hosts/nativescript" && LC_ALL=en_US.UTF-8 npx ns build "$1")
}

package() {
  mkdir -p "$out"
  case ${1:-} in
  tizen) package_tizen ;;
  webos) package_webos ;;
  android) package_android ;;
  browser | firebolt) package_static "$1" ;;
  tvos | ios) package_apple "$1" ;;
  *) die "unknown target '${1:-}'; expected tizen, webos, android, browser, firebolt, tvos or ios" ;;
  esac
}

command=${1:-}
[ $# -gt 0 ] && shift
case $command in
install) install "$@" ;;
build) build "$@" ;;
package) package "$@" ;;
*) die "usage: tv.sh install|build|package [target]" ;;
esac
