# SPDX-License-Identifier: AGPL-3.0-only

# List web client recipes and their descriptions.
default:
    @just --list web

# Install the client's dependencies.
[working-directory('../web')]
install:
    npm install

# Serve the client with hot reload on http://localhost:5173, proxying the API to :8096.
[working-directory('../web')]
dev:
    npm run dev

# Build the client; the next server build embeds it.
[working-directory('../web')]
build:
    npm run build

# Type-check the client against the committed GraphQL schema.
[working-directory('../web')]
check:
    npm run check
