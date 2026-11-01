// SPDX-License-Identifier: AGPL-3.0-only

import { sveltekit } from "@sveltejs/kit/vite";
import houdini from "houdini/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [houdini(), sveltekit()],
  server: {
    proxy: Object.fromEntries(
      [
        "/graphql",
        "/download",
        "/stream",
        "/recordings",
        "/images",
        "/trickplay",
        "/auth",
        "/plugins",
        // Logo proxy only; `/live-tv` itself is an app route.
        "/live-tv/channels",
      ].map((path) => [path, { target: "http://127.0.0.1:8096", ws: path === "/graphql" }]),
    ),
  },
});
