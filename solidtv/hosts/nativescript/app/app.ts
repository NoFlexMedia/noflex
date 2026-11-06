// SPDX-License-Identifier: AGPL-3.0-only
//
// The boot file for Apple TV and iOS. Everything above the renderer is the shared
// app in ../../../src, unchanged; this builds the two views it needs, one behind
// the other, and answers `host` for a runtime with no DOM.

// The polyfill first: the renderer and router read browser globals at import. Then
// the shims, which give those globals the shape a browser app expects.
import "@nativescript/canvas-polyfill";
import "@solidtv/nativescript/shims";
import { Application, ApplicationSettings, Color, Device, GridLayout, SwipeDirection, type SwipeGestureEventData } from "@nativescript/core";
import { isTvOS } from "@nativescript/core/platform";
import { Canvas } from "@nativescript/canvas";
import { renderer } from "@solidtv/solid";
import { KeyBridge, bindCanvas, bindLifecycle, bindRemote, type PausableRenderer } from "@solidtv/nativescript";
import { configureHost } from "$lib/host.js";
import { AvVideoPlayer, bindVideoHost } from "./avplayer.js";
import { appleCapabilities } from "./capabilities.js";

configureHost({
  deviceName: () => (isTvOS ? "Apple TV" : Device.deviceType === "Tablet" ? "iPad" : "iPhone"),
  // NSUserDefaults. The polyfill's localStorage is a file in Caches on tvOS, which the
  // system may purge, and a debounced write there could lose a rotated refresh token.
  storage: {
    get: (key) => ApplicationSettings.getString(key) ?? null,
    set: (key, value) => ApplicationSettings.setString(key, value),
    remove: (key) => ApplicationSettings.remove(key),
  },
  capabilities: appleCapabilities,
  createPlayer: () => Promise.resolve(new AvVideoPlayer()),
  // The screensaver doesn't know about a layer we play into ourselves.
  keepAwake: (on) => { UIApplication.sharedApplication.idleTimerDisabled = on; },
});

/**
 * The canvas's backing view and layers, which `@nativescript/canvas` doesn't expose
 * separately: all of them have to agree for the video underneath to show through.
 * The renderer already asks for an alpha context and clears to 0. Opaque again when
 * nothing's playing, so the compositor isn't blending a full-screen layer all evening.
 */
function setOpaque(view: UIView, opaque: boolean): void {
  view.opaque = opaque;
  view.backgroundColor = opaque ? UIColor.blackColor : UIColor.clearColor;
  view.layer.opaque = opaque;
  const sublayers = view.layer.sublayers;
  for (let index = 0; index < (sublayers?.count ?? 0); index += 1) sublayers!.objectAtIndex(index).opaque = opaque;
  const subviews = view.subviews;
  for (let index = 0; index < subviews.count; index += 1) setOpaque(subviews.objectAtIndex(index), opaque);
}

/** A phone has no remote: the screen stands in for one. Tap is select, a long press is Back. */
function bindTouch(canvas: Canvas, bridge: KeyBridge): void {
  const arrows: Partial<Record<SwipeDirection, string>> = {
    [SwipeDirection.left]: "ArrowLeft",
    [SwipeDirection.right]: "ArrowRight",
    [SwipeDirection.up]: "ArrowUp",
    [SwipeDirection.down]: "ArrowDown",
  };
  canvas.on("tap", () => bridge.tap("Enter"));
  canvas.on("longPress", () => bridge.tap("Backspace"));
  canvas.on("swipe", (args) => {
    const key = arrows[(args as SwipeGestureEventData).direction];
    if (key) bridge.tap(key);
  });
}

function boot(canvas: Canvas): void {
  bindCanvas(canvas);
  const bridge = new KeyBridge({ target: document as never });
  bindLifecycle(() => renderer as unknown as PausableRenderer | undefined, bridge, Application);
  if (isTvOS) bindRemote(bridge, Application.ios.window);
  else bindTouch(canvas, bridge);
  import("../../../src/index").catch((error: unknown) => console.error("The app's entry failed to load", error));
}

function createRootView(): GridLayout {
  const root = new GridLayout();
  root.backgroundColor = new Color("#07080b");

  // Siblings in a shared parent: a CALayer's zPosition alone doesn't order across
  // the GL surface. The video view goes first, so it's behind.
  const video = new GridLayout();
  video.width = { unit: "%", value: 100 };
  video.height = { unit: "%", value: 100 };
  video.iosOverflowSafeArea = true;
  video.isUserInteractionEnabled = false;
  root.addChild(video);

  const canvas = new Canvas();
  canvas.width = "100%";
  canvas.height = "100%";
  // Percent sizes size the layout and the surface; this keeps out of tvOS's title-safe insets.
  canvas.iosOverflowSafeArea = true;
  canvas.on("ready", () => {
    // The plugin's ready callback swallows exceptions: a throw would be a silent blank screen.
    try {
      bindVideoHost(video, (opaque) => setOpaque(canvas.ios as UIView, opaque));
      boot(canvas);
    } catch (error: unknown) {
      console.error("Boot failed", error);
    }
  });
  root.addChild(canvas);
  return root;
}

Application.run({ create: createRootView });
