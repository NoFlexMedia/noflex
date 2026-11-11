#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Packages a Linux bundle as a .deb and an .rpm into OUT, as the release workflow
# does. Usage: package.sh BUNDLE VERSION OUT, where BUNDLE is a directory that
# `cargo xtask bundle` made, such as dist/noflex-x86_64-unknown-linux-gnu.

set -euo pipefail

bundle=$(cd "$1" && pwd)
version=$2
mkdir -p "$3"
out=$(cd "$3" && pwd)
cd "$(dirname "$0")/../.."

case $(basename "$bundle") in
noflex-x86_64-unknown-linux-gnu) arch=amd64 ;;
noflex-aarch64-unknown-linux-gnu) arch=arm64 ;;
*)
  echo "not a Linux bundle: $bundle" >&2
  exit 2
  ;;
esac

# nfpm expands the environment in its fields but not in file paths.
config=$(mktemp)
trap 'rm -f -- "$config"' EXIT
sed "s|\${BUNDLE}|$bundle|g" packaging/linux/nfpm.yaml >"$config"
for packager in deb rpm; do
  NFPM_ARCH=$arch NFPM_VERSION=$version \
    nfpm package --config "$config" --packager "$packager" --target "$out"
done
