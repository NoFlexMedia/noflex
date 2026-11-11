# SPDX-License-Identifier: AGPL-3.0-only

# List server recipes and their descriptions.
default:
    @just --list server

# Run the server on your real data directory (http://127.0.0.1:28793); use `just demo up` for throwaway data.
[working-directory('../backend')]
run *args:
    cargo run -p noflex-server -- {{ args }}

# Rebuild and restart the server whenever the backend changes.
[working-directory('../backend')]
watch:
    cargo watch -x 'run -p noflex-server'

# Build the server, or the given cargo arguments in its place.
[working-directory('../backend')]
build *args='-p noflex-server':
    cargo build {{ args }}

# Run one crate's tests with nextest; extra arguments filter them.
[working-directory('../backend')]
test crate *args:
    cargo nextest run -p {{ crate }} {{ args }}

# Run the storage conformance suite.
[working-directory('../backend')]
conformance:
    cargo nextest run -p noflex-storage-turso --test conformance

# Format the workspace with nightly rustfmt.
[working-directory('../backend')]
fmt:
    cargo fmt --all

# Regenerate the SDL snapshot the clients validate against, after a schema change.
[working-directory('../backend')]
snapshot:
    NOFLEX_UPDATE_SNAPSHOTS=1 cargo test -p noflex-graphql --test it schema_snapshot

# Rebuild the plugin-host fixture guests; tests only build them when missing, so do this after an ABI change.
[working-directory('../backend')]
fixtures:
    cargo build --release --target wasm32-wasip2 --workspace --manifest-path crates/noflex-plugin-host/tests/fixtures/Cargo.toml

# Download the pinned embedding model into DIR, for semantic search (`plugin.assets_dir`).
[working-directory('../backend')]
model-fetch dir:
    cargo xtask model-fetch {{ quote(absolute_path(join(invocation_directory(), dir))) }}
