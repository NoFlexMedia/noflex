#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# A bundle that links Homebrew or Nix runs on the machine that built it and nowhere
# else, so fail on any library that isn't the OS's or the bundle's own.

set -euo pipefail

bundle=$1
status=0
while IFS= read -r file; do
    file "$file" | grep -q 'Mach-O' || continue
    while IFS= read -r library; do
        case "$library" in
            /usr/lib/* | /System/Library/* | @rpath/* | @loader_path/* | @executable_path/*) ;;
            *)
                echo "$file links $library"
                status=1
                ;;
        esac
    done < <(otool -L "$file" | tail -n +2 | awk '{print $1}')
done < <(find "$bundle" -type f \( -perm -u+x -o -name '*.dylib' \))
exit "$status"
