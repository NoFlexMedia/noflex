// SPDX-License-Identifier: AGPL-3.0-only
//
// The app is ../../src, the web targets' source, untouched; this project only
// bundles it for the runtime. @nativescript/vite has no tvOS platform, so it's webpack.

const path = require("path");
const { DefinePlugin } = require("webpack");
const webpack = require("@nativescript/webpack");
const { chainSolidTV } = require("@solidtv/nativescript/webpack");

const ROOT = path.resolve(__dirname, "../..");
const SRC = path.join(ROOT, "src");
const PUBLIC = path.join(ROOT, "public");
const EMPTY = path.resolve(__dirname, "stubs/empty.cjs");

module.exports = (env) => {
  webpack.init(env);
  webpack.chainWebpack((config) => {
    chainSolidTV(config, {
      alias: {
        $lib: path.join(SRC, "lib"),
        $components: path.join(SRC, "components"),
        $pages: path.join(SRC, "pages"),
        $codecs: path.join(ROOT, "../lib/ts/codecSupport.ts"),
        // Web hosts' players, and a Firebolt transport: never opened here, but their chunks still bundle.
        "hls.js": EMPTY,
        "@firebolt-js/sdk": EMPTY,
      },
    });
    config.plugin("NoFlexTarget").use(DefinePlugin, [{ __TARGET__: JSON.stringify("nativescript") }]);
    // `npm run check` in solidtv/ is the type check; the bundle's would see the web hosts' DOM.
    config.plugins.delete("ForkTsCheckerWebpackPlugin");
    config.set("ignoreWarnings", [{ message: /not found in '(@firebolt-js\/sdk|hls\.js)'/ }]);
    // The fonts and sprites the web targets serve from public/, at the paths BASE_URL names.
    config.plugin("CopyWebpackPlugin").tap((args) => {
      args[0].patterns.push(
        { from: path.join(PUBLIC, "fonts"), to: "fonts" },
        { from: path.join(PUBLIC, "icons.png"), to: "icons.png" },
        { from: path.join(PUBLIC, "brand.png"), to: "brand.png" },
        { from: path.join(PUBLIC, "licenses"), to: "licenses" },
      );
      return args;
    });
  });
  return webpack.resolveConfig();
};
