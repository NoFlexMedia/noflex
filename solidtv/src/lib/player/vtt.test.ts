// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { cueAt, parseVtt } from "$lib/player/vtt.js";

const SAMPLE = `WEBVTT

NOTE a comment with no timing

1
00:00:01.000 --> 00:00:03.500 align:start
<i>Hello</i> &amp; welcome

00:03.000 --> 00:05.000
Second line
across two

01:00:00.000 --> 01:00:01.000
An hour in
`;

describe("parseVtt", () => {
  it("reads both timestamp forms, strips markup, and skips blocks without timing", () => {
    expect(parseVtt(SAMPLE)).toEqual([
      { start: 1, end: 3.5, text: "Hello & welcome" },
      { start: 3, end: 5, text: "Second line\nacross two" },
      { start: 3600, end: 3601, text: "An hour in" },
    ]);
  });

  it("takes CRLF files", () => {
    expect(parseVtt("WEBVTT\r\n\r\n00:01.000 --> 00:02.000\r\nHi\r\n")).toEqual([{ start: 1, end: 2, text: "Hi" }]);
  });
});

describe("cueAt", () => {
  const cues = parseVtt(SAMPLE);

  it("stacks cues that overlap", () => {
    expect(cueAt(cues, 3.2).text).toBe("Hello & welcome\nSecond line\nacross two");
  });

  it("is blank between cues, however it got there", () => {
    expect(cueAt(cues, 100).text).toBe("");
    expect(cueAt(cues, 0.5, 2).text).toBe("");
  });

  it("finds its way back after a seek backwards", () => {
    const later = cueAt(cues, 3600.5);
    expect(later.text).toBe("An hour in");
    expect(cueAt(cues, 1.5, later.index).text).toBe("Hello & welcome");
  });
});
