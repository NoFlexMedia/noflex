# SPDX-License-Identifier: AGPL-3.0-only

# List check recipes and their descriptions.
default:
    @just --list check

# Format-check, lint and test one crate; the quick gate while working.
[working-directory('../backend')]
crate name *args:
    cargo fmt --all -- --check
    cargo clippy -p {{ name }} --all-targets -- -D warnings
    cargo nextest run -p {{ name }} {{ args }}

# Run every backend gate over the whole workspace (slow).
[working-directory('../backend')]
backend:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace

# Check the web client.
[working-directory('../web')]
web:
    npm run check

# Check the TV app and build every web-based target.
tv:
    just tv check
    just tv build

# Analyze and test the Flutter app.
[working-directory('../flutter')]
app:
    flutter analyze --no-pub
    flutter test --no-pub

# Format-check and lint the justfiles and task scripts.
[working-directory('..')]
tasks:
    just --fmt --check
    for file in tasks/*.just; do just --fmt --check --justfile "$file" || exit 1; done
    shfmt --diff --indent 2 scripts/tasks
    shellcheck scripts/tasks/*.sh

# Build the release package and check formatting through the flake, as CI does (slowest).
[working-directory('..')]
nix:
    nix flake check

# Run every client and backend gate; `just check nix` is the one left out.
all: tasks backend web tv app
