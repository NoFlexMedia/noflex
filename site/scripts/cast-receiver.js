// SPDX-License-Identifier: AGPL-3.0-only
//
// The Cast receiver must be served over HTTPS from the NoFlex domain
// (cast/AGENTS.md), so the site carries it at /cast/receiver/, copied as is,
// with the web client's codec probe bundled beside each version as `codecs.js`
// for it to run on the device.

import { cpSync, readdirSync } from "node:fs";
import { build } from "vite";

const source = new URL("../../cast/receiver/", import.meta.url);
const target = new URL("../build/cast/receiver/", import.meta.url);
cpSync(source, target, { recursive: true });

for (const version of readdirSync(source).filter((name) => /^v\d+$/.test(name))) {
  await build({
    configFile: false,
    logLevel: "warn",
    build: {
      outDir: new URL(version, target).pathname,
      emptyOutDir: false,
      lib: {
        entry: new URL("../../lib/ts/codecSupport.ts", import.meta.url).pathname,
        formats: ["iife"],
        name: "NoFlexCodecs",
        fileName: () => "codecs.js",
      },
    },
  });
}
