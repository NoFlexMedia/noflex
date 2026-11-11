// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { AUTO, AutoQuality, decodeChoice, encodeChoice, frameLabel, sessionQuality } from "$quality";

function clocked() {
  let now = 0;
  const auto = new AutoQuality(() => now);
  return { auto, at: (ms: number) => { now = ms; } };
}

describe("AutoQuality", () => {
  it("leaves the original after repeated stalls, but not for filling the buffer", () => {
    const { auto, at } = clocked();
    auto.settle();
    at(1_000); auto.stalled(); at(1_500); auto.resumed();
    expect(auto.struggling()).toBe(false);
    at(20_000); auto.stalled(); at(21_000); auto.resumed();
    expect(auto.struggling()).toBe(false);
    at(40_000); auto.stalled();
    expect(auto.struggling()).toBe(true);
  });

  it("forgets stalls that fell out of the window", () => {
    const { auto, at } = clocked();
    at(10_000); auto.stalled(); auto.resumed();
    at(10_000 + AUTO.windowMs + 1); auto.stalled(); auto.resumed();
    expect(auto.struggling()).toBe(false);
  });

  it("takes one long stall as enough", () => {
    const { auto, at } = clocked();
    at(10_000); auto.stalled();
    at(10_000 + AUTO.longStallMs - 1);
    expect(auto.struggling()).toBe(false);
    at(10_000 + AUTO.longStallMs);
    expect(auto.struggling()).toBe(true);
  });

  it("goes back to the original only after holding, with headroom the whole time", () => {
    const { auto, at } = clocked();
    const original = 10_000_000;
    expect(auto.recovered(50e6, original)).toBe(false);
    auto.leftOriginal();
    // Comfortable for long enough, but too soon after leaving.
    at(1_000); expect(auto.recovered(20e6, original)).toBe(false);
    at(1_000 + AUTO.recoverMs); expect(auto.recovered(20e6, original)).toBe(false);
    // A dip below the headroom starts the clock again.
    at(AUTO.holdMs - 10_000); expect(auto.recovered(12e6, original)).toBe(false);
    at(AUTO.holdMs); expect(auto.recovered(20e6, original)).toBe(false);
    at(AUTO.holdMs + AUTO.recoverMs - 1); expect(auto.recovered(20e6, original)).toBe(false);
    at(AUTO.holdMs + AUTO.recoverMs); expect(auto.recovered(20e6, original)).toBe(true);
  });
});

describe("sessionQuality", () => {
  it("asks for the ladder under the original once Auto has left it", () => {
    expect(sessionQuality({ kind: "auto" }, { bitrateBps: 20e6 })).toEqual({ adaptive: true, maxBitrateBps: null });
    expect(sessionQuality({ kind: "auto" }, { bitrateBps: 20e6 }, true)).toEqual({ adaptive: true, maxBitrateBps: 18e6 });
    expect(sessionQuality({ kind: "pinned", bitrateBps: 4e6 }, null)).toEqual({ adaptive: false, maxBitrateBps: 4e6 });
  });

  it("keeps a choice as text", () => {
    for (const choice of [{ kind: "original" }, { kind: "auto" }, { kind: "pinned", bitrateBps: 8e6 }] as const) {
      expect(decodeChoice(encodeChoice(choice))).toEqual(choice);
    }
    expect(decodeChoice("nonsense")).toEqual({ kind: "original" });
  });
});

describe("frameLabel", () => {
  it.each([[1280, 544, "720p"], [1920, 1080, "1080p"], [1920, 1088, "1080p"], [3840, 1600, "4K"], [1440, 1080, "1080p"], [720, 576, "720p"]])("names %ix%i %s", (width, height, label) => {
    expect(frameLabel(width, height)).toBe(label);
  });
});
