# SPDX-License-Identifier: AGPL-3.0-only

# List site recipes and their descriptions.
default:
    @just --list site

# Install the site's dependencies.
[working-directory('../site')]
install:
    npm install

# Serve the site with hot reload on http://localhost:5175.
[working-directory('../site')]
dev:
    npm run dev

# Build the static site, with the Cast receiver, into site/build.
[working-directory('../site')]
build:
    npm run build

# Serve the last build on http://localhost:5175, as a static host would.
[working-directory('../site')]
preview:
    npx vite preview --port 5175
