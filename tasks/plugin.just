# SPDX-License-Identifier: AGPL-3.0-only

# List plugin recipes and their descriptions.
default:
    @just --list plugin

# Build the named plugins by id or crate name, or all of them.
[working-directory('../backend')]
build *names:
    cargo xtask plugin-build {{ names }}

# Build plugins and hot-reload them into your running server, through $XDG_CONFIG_HOME/noflex/plugins.
[working-directory('../backend')]
dev +names:
    cargo xtask plugin-dev {{ names }}

# Run one plugin's tests against its recorded fixtures, by part of its folder name: `just plugin test tmdb`.
[working-directory('../backend')]
test name *args:
    cargo test --manifest-path plugins/*{{ name }}*/Cargo.toml {{ args }}
