# SPDX-License-Identifier: AGPL-3.0-only

# List release recipes and their descriptions.
default:
    @just --list release

# Assemble a server bundle under dist/, e.g. `just release bundle x86_64-unknown-linux-gnu`.
[working-directory('../backend')]
bundle target *args:
    cargo xtask bundle {{ target }} {{ args }}

# Package a Linux bundle as a .deb and an .rpm into OUT, e.g. `just release linux-packages dist/noflex-x86_64-unknown-linux-gnu 0.1.0 dist`.
[working-directory('..')]
linux-packages bundle version out:
    packaging/linux/package.sh {{ quote(absolute_path(join(invocation_directory(), bundle))) }} {{ version }} {{ quote(absolute_path(join(invocation_directory(), out))) }}

# Build the Flutter app for android, linux, windows, macos or ios, and package it into dist/.
[working-directory('..')]
app target:
    packaging/flutter/build.sh {{ target }}

# Compile the pinned ffmpeg for a target into OUT.
[working-directory('../backend')]
ffmpeg-build target out:
    cargo xtask ffmpeg-build {{ target }} {{ quote(absolute_path(join(invocation_directory(), out))) }}

# Gather the source of the pinned ffmpeg and everything it links into OUT, as a release attaches it.
[working-directory('../backend')]
ffmpeg-source out:
    cargo xtask ffmpeg-source {{ quote(absolute_path(join(invocation_directory(), out))) }}

# Check that the ffmpeg in DIR has everything NoFlex requires.
[working-directory('../backend')]
ffmpeg-verify target dir:
    cargo xtask ffmpeg-verify {{ target }} {{ quote(absolute_path(join(invocation_directory(), dir))) }}

# Check the record of third-party code the apps ship against the app and its license assets.
[working-directory('../backend')]
third-party-check:
    cargo xtask third-party check

# Rewrite the apps' license assets from packaging/third-party/apps.toml.
[working-directory('../backend')]
third-party-sync:
    cargo xtask third-party sync

# Gather the pinned sources of the third-party code the apps ship into OUT.
[working-directory('../backend')]
third-party-source out:
    cargo xtask third-party source {{ quote(absolute_path(join(invocation_directory(), out))) }}
