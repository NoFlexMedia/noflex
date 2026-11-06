// SPDX-License-Identifier: AGPL-3.0-only
//
// What belongs to this TV rather than the viewer: how captions read across this room,
// and how its remote behaves. What follows the viewer between devices is the `playback`
// preference on the server (`savePlaybackPrefs`), which every client shares.

import { host } from "$lib/host.js";
import { reactive } from "$lib/state.js";

const KEY = "noflex.settings";

const DEFAULTS = {
  captionSize: "medium" as "small" | "medium" | "large",
  captionBackground: "dim" as "none" | "dim" | "solid",
  /** Seconds the skip buttons and a press of Left or Right move. */
  skipStep: 10,
  autoSkipIntro: false,
  /** Seconds the next episode waits, from when the credits start. */
  upNextSeconds: 10,
  allowHdr: true,
};

export type DeviceSettings = typeof DEFAULTS;

function stored(): Partial<DeviceSettings> {
  try {
    return JSON.parse(host.storage.get(KEY) ?? "{}") as Partial<DeviceSettings>;
  } catch {
    return {};
  }
}

export const settings = reactive<DeviceSettings>({ ...DEFAULTS, ...stored() });

export function changeSetting<K extends keyof DeviceSettings>(key: K, value: DeviceSettings[K]): void {
  settings[key] = value;
  host.storage.set(KEY, JSON.stringify(settings));
}

export const CAPTION_SIZES: Record<DeviceSettings["captionSize"], number> = { small: 36, medium: 44, large: 60 };
export const CAPTION_BACKGROUNDS: Record<DeviceSettings["captionBackground"], number> = { none: 0x00000000, dim: 0x000000b3, solid: 0x000000ff };

/** A `quality` preference's picture ceiling. Other values mean no ceiling. */
export const QUALITY_HEIGHTS: Record<string, number> = { "2160": 2160, "1080": 1080, "720": 720, "480": 480 };
