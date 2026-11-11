# SPDX-License-Identifier: AGPL-3.0-only

set positional-arguments

script := justfile_directory() / "scripts/tasks/tv.sh"

# List TV app recipes and their descriptions.
default:
    @just --list tv

# Install the app's dependencies; `apple` adds the NativeScript project's.
install what='':
    @"{{ script }}" install "$@"

# Serve the app in a browser on http://localhost:5174, pointed at SERVER (default: the demo server's address).
[working-directory('../solidtv')]
dev server='http://127.0.0.1:28793':
    VITE_NOFLEX_SERVER={{ server }} npm run dev

# Type-check, validate GraphQL documents against the schema, and run the tests.
[working-directory('../solidtv')]
check:
    npm run check

# Build one target, or `all` of the web-based ones, into solidtv/dist/.
build target='all':
    @"{{ script }}" build "$1"

# Package a target for its store or device: tizen, webos, android, firebolt, browser, tvos, ios.
package target:
    @"{{ script }}" package "$1"

# Run on a booted simulator, or a paired device with --device ID: tvos or ios. Needs a Mac.
[working-directory('../solidtv/hosts/nativescript')]
run target='tvos' *args:
    LC_ALL=en_US.UTF-8 npx ns run {{ target }} --no-hmr {{ args }}

# Regenerate the committed sprite sheet, launcher icons and font atlases.
[working-directory('../solidtv')]
assets:
    npm run assets:icons
    npm run assets:fonts
