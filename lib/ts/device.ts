// SPDX-License-Identifier: AGPL-3.0-only
//
// What a device is. Sessions and devices only record free text, so `deviceBrand` reads
// it for the likeliest match, and `browserDeviceName` is that text for anything running
// in a browser. Which icon a brand gets is each client's business;
// flutter/lib/utils/device_icon.dart ports the table, so a change here goes there too.

export type DeviceBrand =
  | "googletv" | "appletv" | "nvidia" | "television"
  | "playstation" | "steamdeck" | "gamepad"
  | "brave" | "vivaldi" | "opera" | "duckduckgo" | "firefox" | "safari" | "chrome" | "browser"
  | "tablet" | "phone" | "mac"
  | "windows" | "ubuntu" | "fedora" | "archlinux" | "debian" | "nixos" | "linuxmint" | "popos" | "manjaro" | "opensuse" | "linux" | "android"
  | "laptop" | "desktop";

// First match wins: a TV running Android reads as a TV, an iPad as a tablet, and a
// browser as itself rather than as whatever it runs on.
const BRANDS: readonly (readonly [RegExp, DeviceBrand])[] = [
  [/\bgoogle ?tv\b/i, "googletv"],
  [/\bapple ?tv\b|\btvos\b/i, "appletv"],
  [/\bshield\b|\bnvidia\b/i, "nvidia"],
  [/\b(tv|roku|fire ?stick|chromecast|bravia|vidaa)\b|\bwebos\b|\btizen\b/i, "television"],
  [/playstation/i, "playstation"],
  [/steam ?deck/i, "steamdeck"],
  [/xbox|nintendo/i, "gamepad"],
  [/\bbrave\b/i, "brave"],
  [/\bvivaldi\b/i, "vivaldi"],
  [/\bopera\b/i, "opera"],
  [/duckduckgo/i, "duckduckgo"],
  [/firefox/i, "firefox"],
  [/safari/i, "safari"],
  [/chrome|chromium/i, "chrome"],
  [/\bedge\b|samsung internet|browser/i, "browser"],
  [/ipad|tablet/i, "tablet"],
  [/iphone|\bios\b|phone|mobile/i, "phone"],
  [/\bmac|\bimac\b/i, "mac"],
  [/windows/i, "windows"],
  [/ubuntu/i, "ubuntu"],
  [/fedora/i, "fedora"],
  [/arch linux/i, "archlinux"],
  [/debian/i, "debian"],
  [/nixos/i, "nixos"],
  [/linux mint/i, "linuxmint"],
  [/pop!_os/i, "popos"],
  [/manjaro/i, "manjaro"],
  [/opensuse/i, "opensuse"],
  [/linux/i, "linux"],
  [/android/i, "android"],
  [/laptop/i, "laptop"],
];

export function deviceBrand(device?: string | null, client?: string | null): DeviceBrand {
  const text = `${device ?? ""} ${client ?? ""}`;
  for (const [pattern, brand] of BRANDS) if (pattern.test(text)) return brand;
  return "desktop";
}

/** The parts of `navigator` that say what this is, so a host without one can pass its own. */
export interface NavigatorHints {
  userAgent: string;
  maxTouchPoints?: number;
  /** Brave's only tell: its user agent says Chrome on purpose. */
  brave?: unknown;
  userAgentData?: { brands?: readonly { brand: string }[] };
}

// Forks first, since nearly all of them also say Chrome and Safari.
const BROWSER_TOKENS: readonly (readonly [RegExp, string])[] = [
  [/\bEdg(e|A|iOS)?\//, "Edge"],
  [/\bOPR\/|\bOPiOS\/|\bOPT\/|\bOpera\b/, "Opera"],
  [/\bVivaldi\//, "Vivaldi"],
  [/\bSamsungBrowser\//, "Samsung Internet"],
  [/\bDuckDuckGo\/|\bDdg\//, "DuckDuckGo"],
  [/\bFirefox\/|\bFxiOS\//, "Firefox"],
  // Not from a word boundary, or HeadlessChrome reads as Safari.
  [/Chrome\/|\bCriOS\//, "Chrome"],
  [/\bSafari\//, "Safari"],
];

// Client hints say more than the user agent does once a browser freezes it.
const BROWSER_BRANDS: readonly (readonly [string, string])[] = [
  ["Brave", "Brave"],
  ["Microsoft Edge", "Edge"],
  ["Opera", "Opera"],
  ["Vivaldi", "Vivaldi"],
  ["Samsung Internet", "Samsung Internet"],
  ["DuckDuckGo", "DuckDuckGo"],
];

function browserOf(hints: NavigatorHints): string {
  if (hints.brave) return "Brave";
  const brands = hints.userAgentData?.brands?.map((item) => item.brand) ?? [];
  for (const [brand, name] of BROWSER_BRANDS) if (brands.indexOf(brand) >= 0) return name;
  for (const [pattern, name] of BROWSER_TOKENS) if (pattern.test(hints.userAgent)) return name;
  return "Browser";
}

function systemOf({ userAgent: ua, maxTouchPoints = 0 }: NavigatorHints): string | null {
  if (/\bTizen\b/.test(ua)) return "Tizen";
  if (/\bWeb0S\b|\bwebOS\b/.test(ua)) return "webOS";
  if (/\bVIDAA\b/.test(ua)) return "VIDAA";
  if (/\bXbox\b/.test(ua)) return "Xbox";
  if (/\bPlayStation\b/.test(ua)) return "PlayStation";
  if (/\biPhone\b/.test(ua)) return "iPhone";
  // An iPad asks for desktop sites, and says it's a Mac.
  if (/\biPad\b/.test(ua) || (/\bMacintosh\b/.test(ua) && maxTouchPoints > 1)) return "iPad";
  if (/\bAndroid\b/.test(ua)) return "Android";
  if (/\bWindows\b/.test(ua)) return "Windows";
  if (/\bMac OS X\b/.test(ua)) return "macOS";
  if (/\bCrOS\b/.test(ua)) return "ChromeOS";
  if (/\bLinux\b/.test(ua)) return "Linux";
  return null;
}

/** "Brave on Linux", "Safari on iPad", for this browser's entry in a device list. */
export function browserDeviceName(hints: NavigatorHints): string {
  const browser = browserOf(hints);
  const system = systemOf(hints);
  return system ? `${browser} on ${system}` : browser;
}
