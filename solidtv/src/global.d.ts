// SPDX-License-Identifier: AGPL-3.0-only

declare global {
  const __TARGET__: "browser" | "tizen" | "webos" | "android" | "firebolt" | "nativescript";
  // Read by @solidtv/solid's source, which is what the `@solidtv/source` condition resolves to.
  const SOLIDTV_DOM_RENDERING: boolean;
  const SOLIDTV_DISABLE_SHADERS: boolean;
}

declare module "@solidtv/solid" {
  interface KeyMap {
    Play: (string | number)[];
    Pause: (string | number)[];
    PlayPause: (string | number)[];
    Stop: (string | number)[];
    FastForward: (string | number)[];
    Rewind: (string | number)[];
  }
}

export {};
