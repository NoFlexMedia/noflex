#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Starts a bundle from an empty data directory, with nothing but its own paths
# configured, and checks it found its own ffmpeg, plugins and model. Runs in Git
# Bash on Windows too. Usage: server.sh BUNDLE

set -euo pipefail

bundle=$(cd "$1" && pwd)
home=$(mktemp -d)
# A Windows server reads paths as Windows writes them.
native() { if command -v cygpath >/dev/null; then cygpath -m "$1"; else echo "$1"; fi; }
cat >"$home/config.toml" <<TOML
bind = "127.0.0.1:18096"
data_dir = "$(native "$home/data")"
cache_dir = "$(native "$home/cache")"
TOML

# The user tier is empty here, so only the bundle's plugins load.
NOFLEX_CONFIG_FILE=$(native "$home/config.toml") XDG_CONFIG_HOME=$(native "$home/user") \
  "$bundle/noflex-server" >"$home/server.log" 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true; tail -40 "$home/server.log"' EXIT

for _ in $(seq 1 120); do
  curl -sf -o /dev/null http://127.0.0.1:18096/healthz/deep && break
  sleep 1
done
health=$(curl -s http://127.0.0.1:18096/healthz/deep)
echo "$health"
echo "$health" | grep -q '"ok":true'
echo "$health" | grep -Eq '"loaded_plugins":[1-9]'
