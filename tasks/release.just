# SPDX-License-Identifier: AGPL-3.0-only

# List release recipes and their descriptions.
default:
    @just --list release

# Assemble a server bundle under dist/, e.g. `just release bundle x86_64-unknown-linux-gnu`.
[working-directory('../backend')]
bundle target *args:
    cargo xtask bundle {{ target }} {{ args }}

# Build the Flutter app for android, linux, windows, macos or ios, and package it into dist/.
[working-directory('..')]
app target:
    packaging/flutter/build.sh {{ target }}

# Compile the pinned ffmpeg for a target into OUT.
[working-directory('../backend')]
ffmpeg-build target out:
    cargo xtask ffmpeg-build {{ target }} {{ quote(absolute_path(join(invocation_directory(), out))) }}

# Check that the ffmpeg in DIR has everything NoFlex requires.
[working-directory('../backend')]
ffmpeg-verify target dir:
    cargo xtask ffmpeg-verify {{ target }} {{ quote(absolute_path(join(invocation_directory(), dir))) }}
