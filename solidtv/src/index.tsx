// SPDX-License-Identifier: AGPL-3.0-only

import { Config, createRenderer, loadFonts } from "@solidtv/solid";
import type { RendererMain } from "@solidtv/renderer";
import { FocusStackProvider } from "@solidtv/solid/primitives";
import { HashRouter, KeepAliveRoute } from "@solidtv/solid/primitives/router";
import { Route } from "@solidjs/router";
import { Rounded, RoundedWithBorder, RoundedWithBorderAndShadow, RoundedWithShadow, LinearGradient, RadialGradient, HolePunch, RadialProgress } from "@solidtv/renderer/shaders";
import { fonts } from "./fonts.js";
import { color, ease, SCREEN, type as typeScale } from "./theme.js";
import { host } from "$lib/host.js";
import { player } from "$lib/playback.js";
import { start } from "$lib/session.js";
import { appHeight, appWidth, scale } from "$lib/viewport.js";
import App from "$components/App.js";
import Shell from "$components/Shell.js";
import Account from "$pages/Account.js";
import Browse from "$pages/Browse.js";
import BrowseAll from "$pages/BrowseAll.js";
import Collections from "$pages/Collections.js";
import Home from "$pages/Home.js";
import Library from "$pages/Library.js";
import LiveTv from "$pages/LiveTv.js";
import Search from "$pages/Search.js";
import Title from "$pages/Title.js";

// Solid's non-Proxy mergeProps asks for every own property's descriptor, which
// throws on a strict function's `caller` and `arguments`. Only engines without
// Proxy (webOS 3) take that path.
if (typeof Proxy === "undefined") {
  const own = Object.getOwnPropertyNames;
  Object.getOwnPropertyNames = (object) => {
    const names = own(object);
    return typeof object === "function" ? names.filter((name) => name !== "caller" && name !== "callee" && name !== "arguments") : names;
  };
}

const reducedMotion = (() => { try { return matchMedia("(prefers-reduced-motion: reduce)").matches; } catch { return false; } })();

Config.animationsEnabled = !reducedMotion;
Config.animationSettings = { duration: 240, easing: ease.out };
Config.fontSettings.fontFamily = "Inter";
Config.fontSettings.fontSize = typeScale.body;
Config.fontSettings.color = color.text;
Config.fontWeightAlias = { 400: "", regular: "", semibold: 600, bold: 700 };
// A press a handler took stays with the app; tvOS gives the rest to the system.
Config.preventDefaultOnHandledKeys = __TARGET__ === "nativescript";

// Never draws more pixels than the panel has.
Config.rendererOptions = {
  appWidth,
  appHeight,
  deviceLogicalPixelRatio: scale,
  devicePhysicalPixelRatio: scale >= 1 ? 1 : (typeof window !== "undefined" && window.devicePixelRatio) || 1,
  // 0: the canvas clears to transparent, which is how the video shows through it.
  clearColor: 0,
  textureMemory: { criticalThreshold: 200e6, targetThresholdLevel: 0.8 },
  numImageWorkers: typeof createImageBitmap === "function" ? 1 : 0,
  imageDecodeConcurrency: typeof createImageBitmap === "function" ? 4 : 2,
  boundsMargin: 400,
};

// The renderer fixes the canvas's size at startup, so a resized window starts over on
// the same page. Only here: TVs don't resize, and one that reports it mustn't reload.
if (__TARGET__ === "browser") {
  let resizing: ReturnType<typeof setTimeout> | undefined;
  window.addEventListener("resize", () => {
    clearTimeout(resizing);
    resizing = setTimeout(() => { if (!player.current) location.reload(); }, 300);
  });
}

// A frame first, so a slow engine paints the page's background before the renderer starts up.
requestAnimationFrame(() => {
  setTimeout(() => {
    const { renderer: created, render } = createRenderer();
    // The DOM renderer's type is in the union; it's compiled out here.
    const renderer = created as RendererMain;
    loadFonts(fonts);
    renderer.registerShaderType("rounded", Rounded);
    renderer.registerShaderType("roundedWithBorder", RoundedWithBorder);
    renderer.registerShaderType("roundedWithShadow", RoundedWithShadow);
    renderer.registerShaderType("roundedWithBorderWithShadow", RoundedWithBorderAndShadow);
    renderer.registerShaderType("radialGradient", RadialGradient);
    renderer.registerShaderType("linearGradient", LinearGradient);
    renderer.registerShaderType("holePunch", HolePunch);
    renderer.registerShaderType("radialProgress", RadialProgress);
    void start();
    render(() => (
      <FocusStackProvider>
        <HashRouter root={App}>
          <Route path="" component={Shell}>
            <KeepAliveRoute id="home" path="/" component={Home} />
            <Route path="/search" component={Search} />
            <KeepAliveRoute id="browse" path="/browse/:category" component={Browse} />
            <Route path="/browse/:category/all" component={BrowseAll} />
            <Route path="/libraries/:id" component={Library} />
            <Route path="/collections" component={Collections} />
            <Route path="/live-tv" component={LiveTv} />
            <Route path="/account" component={Account} />
            <Route path="/works/:id" component={Title} />
          </Route>
        </HashRouter>
      </FocusStackProvider>
    ));
    host.ready?.();
  }, 0);
});
