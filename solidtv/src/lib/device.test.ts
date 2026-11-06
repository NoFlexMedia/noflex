// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { browserDeviceName, deviceBrand } from "$devices";

const CHROME = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

describe("browserDeviceName", () => {
  it.each([
    [{ userAgent: CHROME }, "Chrome on Linux"],
    // Brave says Chrome in its user agent, on purpose.
    [{ userAgent: CHROME, brave: {} }, "Brave on Linux"],
    [{ userAgent: CHROME, userAgentData: { brands: [{ brand: "Chromium" }, { brand: "Brave" }] } }, "Brave on Linux"],
    [{ userAgent: `${CHROME} Edg/130.0.0.0` }, "Edge on Linux"],
    [{ userAgent: `${CHROME} OPR/115.0.0.0` }, "Opera on Linux"],
    [{ userAgent: `${CHROME} Vivaldi/7.0` }, "Vivaldi on Linux"],
    [{ userAgent: "Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36" }, "Samsung Internet on Android"],
    [{ userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0" }, "Firefox on Windows"],
    [{ userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0 Mobile/15E148 Safari/604.1" }, "Chrome on iPhone"],
    [{ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15" }, "Safari on macOS"],
    // An iPad says it's a Mac, and only its touch screen gives it away.
    [{ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15", maxTouchPoints: 5 }, "Safari on iPad"],
    [{ userAgent: "Mozilla/5.0 (Linux; Tizen 6.5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/85.0 TV Safari/537.36" }, "Chrome on Tizen"],
    [{ userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/130.0.0.0 Safari/537.36" }, "Chrome on Linux"],
    [{ userAgent: "something else entirely" }, "Browser"],
  ])("%j is %s", (hints, name) => {
    expect(browserDeviceName(hints)).toBe(name);
  });
});

// flutter/test/profile_test.dart holds the same table against its port.
describe("deviceBrand", () => {
  it.each([
    ["Google TV (Chromecast HD)", "NoFlex TV", "googletv"],
    ["Android TV (SHIELD Android TV)", "NoFlex TV", "nvidia"],
    ["Fire TV (AFTMM)", "NoFlex TV", "television"],
    ["Samsung TV", "NoFlex TV", "television"],
    ["Apple TV", "NoFlex TV", "apple"],
    ["Brave on Linux", "NoFlex Web", "brave"],
    ["Firefox on Android", "NoFlex Web", "firefox"],
    ["Chrome on iPhone", "NoFlex Web", "chrome"],
    ["Edge on Windows", "NoFlex Web", "browser"],
    ["Chris’s iPhone", "NoFlex for iOS", "phone"],
    ["iPad", "NoFlex for iOS", "tablet"],
    ["samsung SM-S918B", "NoFlex for Android", "android"],
    ["DESKTOP-4F2A", "NoFlex for Windows", "windows"],
    ["Chris’s MacBook Pro", "NoFlex for macOS", "apple"],
    ["Ubuntu 24.04.1 LTS", "NoFlex for Linux", "ubuntu"],
    ["NixOS 25.11", "NoFlex for Linux", "nixos"],
    ["Some Distro", "NoFlex for Linux", "linux"],
    ["PlayStation 5", null, "playstation"],
    ["Xbox Series X", null, "gamepad"],
    ["Mystery Box", null, "desktop"],
    [null, null, "desktop"],
  ])("%s, %s is %s", (device, client, brand) => {
    expect(deviceBrand(device, client)).toBe(brand);
  });
});
