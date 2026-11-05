// SPDX-License-Identifier: AGPL-3.0-only
//
// Sessions only record free text, so this reads it for the likeliest match.
// `device` is `deviceName()` for the web client and whatever other clients send.

import AndroidLogo from "phosphor-svelte/lib/AndroidLogo";
import AppleLogo from "phosphor-svelte/lib/AppleLogo";
import Browser from "phosphor-svelte/lib/Browser";
import Compass from "phosphor-svelte/lib/Compass";
import Desktop from "phosphor-svelte/lib/Desktop";
import DeviceMobile from "phosphor-svelte/lib/DeviceMobile";
import DeviceTablet from "phosphor-svelte/lib/DeviceTablet";
import Fire from "phosphor-svelte/lib/Fire";
import GameController from "phosphor-svelte/lib/GameController";
import GoogleChromeLogo from "phosphor-svelte/lib/GoogleChromeLogo";
import Laptop from "phosphor-svelte/lib/Laptop";
import LinuxLogo from "phosphor-svelte/lib/LinuxLogo";
import Television from "phosphor-svelte/lib/Television";
import WindowsLogo from "phosphor-svelte/lib/WindowsLogo";

// First match wins, so a TV running Android reads as a TV and an iPad as a tablet.
const ICONS = [
  [/\b(tv|roku|fire ?stick|chromecast|shield)\b|tvos|webos|tizen/i, Television],
  [/playstation|xbox|nintendo|steam deck/i, GameController],
  [/chrome/i, GoogleChromeLogo],
  // Phosphor has no Firefox logo; its flame is the closest thing.
  [/firefox/i, Fire],
  [/safari/i, Compass],
  [/\bedge\b|browser/i, Browser],
  [/ipad|tablet/i, DeviceTablet],
  [/iphone|\bios\b|phone|mobile/i, DeviceMobile],
  [/android/i, AndroidLogo],
  [/mac ?book|laptop/i, Laptop],
  [/\b(macos|mac)\b/i, AppleLogo],
  [/windows/i, WindowsLogo],
  [/linux/i, LinuxLogo],
] as const;

export function deviceIcon(session: { device?: string | null; client?: string | null }) {
  const text = `${session.device ?? ""} ${session.client ?? ""}`;
  return ICONS.find(([pattern]) => pattern.test(text))?.[1] ?? Desktop;
}
