// SPDX-License-Identifier: AGPL-3.0-only
//
// One source, one build per way a TV loads it. See `TARGETS`.

import { fileURLToPath } from "node:url";
import { transformSync } from "@babel/core";
import legacy from "@vitejs/plugin-legacy";
import { defineConfig, type Plugin } from "vite";
import solid from "vite-plugin-solid";

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

type Target = "browser" | "tizen" | "webos" | "android" | "firebolt";

// `packaged`: runs from file://, where Chromium refuses module scripts for want
// of a MIME type (found on Tizen 5.5), so it gets one classic bundle. That bundle
// also has to parse on the oldest engine the target has shipped: Chrome 38 on
// webOS 3, Chromium 47 on Tizen 3.
const TARGETS: Record<Target, { outDir: string; packaged: boolean }> = {
  browser: { outDir: "dist/browser", packaged: false },
  firebolt: { outDir: "dist/firebolt", packaged: false },
  android: { outDir: "dist/android", packaged: false },
  tizen: { outDir: "dist/tizen", packaged: true },
  webos: { outDir: "dist/webos", packaged: true },
};

/**
 * plugin-legacy builds its polyfill chunk at ES2015 and nothing later touches
 * it, so Chrome 38 can't parse it. Bring it down the rest of the way.
 */
function es5Polyfills(): Plugin {
  return {
    name: "noflex-es5-polyfills",
    enforce: "post",
    apply: "build",
    generateBundle(_, bundle) {
      for (const chunk of Object.values(bundle)) {
        if (chunk.type !== "chunk" || !chunk.fileName.includes("polyfills-legacy")) continue;
        chunk.code = transformSync(chunk.code, {
          configFile: false,
          babelrc: false,
          browserslistConfigFile: false,
          compact: true,
          targets: { chrome: "38" },
          presets: [["@babel/preset-env", { modules: false, useBuiltIns: false }]],
        })?.code ?? chunk.code;
      }
    },
  };
}

/** A file:// page can't load anything `crossorigin`: it has no origin to send. */
function noCrossorigin(): Plugin {
  return {
    name: "noflex-no-crossorigin",
    enforce: "post",
    transformIndexHtml: (html) => html.replace(/ crossorigin(="[^"]*")?/g, ""),
  };
}

export default defineConfig(({ mode }) => {
  const target: Target = mode in TARGETS ? (mode as Target) : "browser";
  const { outDir, packaged } = TARGETS[target];
  return {
    base: "./",
    define: {
      __DEV__: mode === "development",
      __TARGET__: JSON.stringify(target),
      __enableCompressedTextures__: false,
      SOLIDTV_DOM_RENDERING: false,
      LIGHTNING_DOM_RENDERING: false,
    },
    plugins: [
      solid({ solid: { moduleName: "@solidtv/solid", generate: "universal", builtIns: [] } }),
      es5Polyfills(),
      legacy({
        targets: ["chrome>=38", "not IE 11"],
        // plugin-legacy 8 defaults to chrome>=105, which leaves `?.` and `??` in
        // the modern bundle; webOS 5 and 6 pass its check but can't parse them.
        modernTargets: ["chrome>=64"],
        renderModernChunks: !packaged,
        additionalLegacyPolyfills: ["whatwg-fetch"],
        // What the shared capability probe uses that the floor above lacks.
        modernPolyfills: ["es.global-this", "es.array.find-last", "es.array.at", "es.string.replace-all", "es.promise.finally", "es.object.from-entries", "es.array.flat-map"],
        // The renderer is heavy on class fields; plain assignment keeps node construction cheap.
        assumptions: { setPublicClassFields: true, privateFieldsAsProperties: true, noClassCalls: true, noDocumentAll: true },
      }),
      ...(packaged ? [noCrossorigin()] : []),
    ],
    build: {
      outDir,
      emptyOutDir: true,
      // oxc can't emit the ES5 the legacy chunks need.
      minify: "terser",
      // Compress folds the renderer's build-time branches; names stay for on-device traces.
      terserOptions: { mangle: false, format: { comments: false } },
      sourcemap: false,
      rolldownOptions: { transform: { assumptions: { setPublicClassFields: true, noDocumentAll: true } } },
    },
    resolve: {
      alias: [
        { find: /^\$lib\//, replacement: `${here("./src/lib")}/` },
        { find: /^\$components\//, replacement: `${here("./src/components")}/` },
        { find: /^\$pages\//, replacement: `${here("./src/pages")}/` },
        { find: "$codecs", replacement: here("../web/src/lib/codecSupport.ts") },
      ],
      conditions: ["@solidtv/source", "browser", ...(mode === "development" ? ["development"] : [])],
      dedupe: ["solid-js", "solid-js/universal", "@solidjs/router", "@solidtv/renderer", "@solidtv/solid", "@solidtv/solid/primitives"],
    },
    optimizeDeps: { exclude: ["@solidtv/solid", "@solidtv/renderer"] },
    server: { port: 5174, fs: { allow: [here(".."), here(".")] } },
    test: { globals: false, environment: "node" },
  };
});
