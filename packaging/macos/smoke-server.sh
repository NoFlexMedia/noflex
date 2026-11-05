#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Starts a bundle with no paths configured, from an empty data directory, and checks
# it found its own ffmpeg, plugins and model.

set -euo pipefail

bundle=$(cd "$1" && pwd)
home=$(mktemp -d)
mkdir -p "$home/config/noflex"
cat >"$home/config/noflex/config.toml" <<TOML
bind = "127.0.0.1:18096"
data_dir = "$home/data"
cache_dir = "$home/cache"
TOML

HOME=$home XDG_CONFIG_HOME=$home/config "$bundle/noflex-server" >"$home/server.log" 2>&1 &
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
