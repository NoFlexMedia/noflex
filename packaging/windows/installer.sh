#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds the Windows installer for a server bundle into OUT, as the release
# workflow does. Runs in Git Bash on Windows, with Inno Setup's iscc on PATH or
# where its installer puts it. Usage: installer.sh BUNDLE VERSION OUT

set -euo pipefail

bundle=$(cygpath -w "$1")
version=$2
mkdir -p "$3"
out=$(cygpath -w "$3")
script=$(cygpath -w "$(dirname "$0")/noflex.iss")
iscc=$(command -v iscc || echo "/c/Program Files (x86)/Inno Setup 6/ISCC.exe")

# MSYS would read /DVersion=… as a path and rewrite it.
MSYS2_ARG_CONV_EXCL='*' "$iscc" "/DVersion=$version" "/DBundle=$bundle" "/O$out" "$script"
