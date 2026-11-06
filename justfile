# SPDX-License-Identifier: AGPL-3.0-only
#
# NoFlex's development commands. Run `just` for every documented recipe, or
# `just GROUP` for one group. Recipes expect the dev shell (`nix develop`).

# List every recipe, organized by command group.
default:
    @just --list --list-submodules

# Rust workspace: run, build, test, and regenerate what's generated from it.
mod server "tasks/server.just"

# First-party WASM plugins.
mod plugin "tasks/plugin.just"

# SvelteKit web client.
mod web "tasks/web.just"

# Flutter app and its test devices.
mod app "tasks/app.just"

# The sample library of Blender open movies, on one server or several.
mod demo "tasks/demo.just"

# The gates CI runs, whole or scoped.
mod check "tasks/check.just"

# Release bundles, client builds, and the ffmpeg behind them.
mod release "tasks/release.just"
