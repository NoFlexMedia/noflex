// SPDX-License-Identifier: AGPL-3.0-only
//
// Everything that differs between what runs this app. `src/` is written once; a
// host answers these. The Vite targets are told apart by `__TARGET__`, which folds
// away the others' code; NativeScript has no DOM, so its boot file calls
// `configureHost` before importing the app.

import type { ClientCapabilities } from "$codecs";
import type { VideoPlayer } from "$lib/player/types.js";

export interface HostStorage {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
}

export interface Host {
  /** Shown in the viewer's device list, with `deviceName`. */
  clientName: string;
  deviceName(): string;
  storage: HostStorage;
  capabilities(): Promise<ClientCapabilities>;
  createPlayer(): Promise<VideoPlayer>;
  /** Leave the app. Hosts where the system does that on an unhandled Back have none. */
  exit?(): void;
  /** Called once the first page has drawn. */
  ready?(): void;
  /** Hold off the screensaver during playback, where the system won't do it for a <video> or layer. */
  keepAwake?(on: boolean): void;
}

// Private windows and blocked site data throw on access.
const webStorage: HostStorage = {
  get: (key) => { try { return localStorage.getItem(key); } catch { return null; } },
  set: (key, value) => { try { localStorage.setItem(key, value); } catch { /* unavailable */ } },
  remove: (key) => { try { localStorage.removeItem(key); } catch { /* unavailable */ } },
};

const TARGET_NAMES: Record<string, string> = { tizen: "Samsung TV", webos: "LG TV", android: "Android TV", firebolt: "RDK TV", browser: "Browser" };

const declaredGlobal = (name: string): any => (globalThis as any)[name];

function exitTarget(): (() => void) | undefined {
  if (__TARGET__ === "tizen") return () => declaredGlobal("tizen")?.application.getCurrentApplication().exit();
  // disableBackHistoryAPI leaves leaving to us; webOS's own launcher takes it from here.
  if (__TARGET__ === "webos") return () => window.close();
  if (__TARGET__ === "android") return () => declaredGlobal("NoFlexHost")?.exit();
  if (__TARGET__ === "firebolt") return () => { void import("@firebolt-js/sdk").then(({ Lifecycle }) => Lifecycle.close("userExit" as any)); };
  return undefined;
}

function readyTarget(): void {
  // Without this the remote's media keys never reach the page.
  if (__TARGET__ === "tizen") {
    try { declaredGlobal("tizen")?.tvinputdevice.registerKeyBatch(["MediaPlay", "MediaPause", "MediaPlayPause", "MediaStop", "MediaFastForward", "MediaRewind"]); } catch { /* an older model; the arrows still seek */ }
  }
  // A Firebolt platform keeps an app on its splash until told it can draw.
  if (__TARGET__ === "firebolt") void import("@firebolt-js/sdk").then(({ Lifecycle }) => Lifecycle.ready());
}

export const host: Host = {
  clientName: "NoFlex TV",
  deviceName: () => TARGET_NAMES[__TARGET__] ?? "TV",
  storage: webStorage,
  capabilities: () => import("$lib/player/capabilities.js").then(({ clientCapabilities }) => clientCapabilities()),
  createPlayer: () => import("$lib/player/html.js").then(({ HtmlVideoPlayer }) => new HtmlVideoPlayer()),
  exit: exitTarget(),
  ready: readyTarget,
  keepAwake: __TARGET__ === "android" ? (on) => declaredGlobal("NoFlexHost")?.keepAwake(on) : undefined,
};

export function configureHost(overrides: Partial<Host>): void { Object.assign(host, overrides); }
