#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds the Flutter app for one target and packages it into dist/. The release
# workflow runs this, so a machine that can build a target runs exactly the same
# thing. See docs/PACKAGING.md.
#
#   packaging/flutter/build.sh <android|linux|windows|macos|ios>

set -euo pipefail

target=${1:?usage: build.sh <android|linux|windows|macos|ios>}
root=$(cd "$(dirname "$0")/../.." && pwd)
dist=$root/dist
cd "$root/flutter"

if [ "${GITHUB_REF_TYPE:-}" = tag ]; then
  version=${GITHUB_REF_NAME#v}
else
  version=$(sed -n 's/^version: \([^+]*\).*/\1/p' pubspec.yaml)
fi
name=noflex-$version
args=(--release "--build-name=$version" "--build-number=${GITHUB_RUN_NUMBER:-1}")

mkdir -p "$dist"

# Git on Windows can check the schema's symlink out as a file holding its target.
if [ "$target" = windows ]; then
  rm -f lib/graphql/schema.graphql
  cp ../backend/crates/noflex-graphql/tests/snapshots/schema.graphql lib/graphql/schema.graphql
fi

flutter pub get
dart run build_runner build

case $target in
  android)
    flutter build apk --split-per-abi "${args[@]}"
    for apk in build/app/outputs/flutter-apk/app-*-release.apk; do
      abi=${apk##*/app-}
      cp "$apk" "$dist/$name-android-${abi%-release.apk}.apk"
    done
    # Play only takes a bundle signed with the upload key.
    if [ -f android/key.properties ]; then
      flutter build appbundle "${args[@]}"
      cp build/app/outputs/bundle/release/app-release.aab "$dist/$name-android.aab"
    fi
    ;;
  linux)
    flutter build linux "${args[@]}"
    tar -czf "$dist/$name-linux-x64.tar.gz" -C build/linux/x64/release \
      --transform "s,^bundle,$name-linux-x64," bundle
    ;;
  windows)
    flutter build windows "${args[@]}"
    (cd build/windows/x64/runner/Release &&
      powershell -NoProfile -Command \
        "Compress-Archive -Force -Path * -DestinationPath '$(cygpath -w "$dist/$name-windows-x64.zip")'")
    ;;
  macos)
    flutter build macos "${args[@]}"
    # Plain zip would break the framework symlinks inside the app.
    ditto -c -k --sequesterRsrc --keepParent build/macos/Build/Products/Release/*.app \
      "$dist/$name-macos.zip"
    ;;
  ios)
    flutter build ipa --no-codesign "${args[@]}"
    payload=build/ios/unsigned/Payload
    rm -rf "$payload"
    mkdir -p "$payload"
    cp -R build/ios/archive/Runner.xcarchive/Products/Applications/*.app "$payload/"
    ditto -c -k --sequesterRsrc --keepParent "$payload" "$dist/$name-ios-unsigned.ipa"
    ;;
  *)
    echo "unknown target: $target" >&2
    exit 2
    ;;
esac

ls -lh "$dist"
