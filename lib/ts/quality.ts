// SPDX-License-Identifier: AGPL-3.0-only
//
// Stream quality: what the viewer chose, what to ask the server for, and when
// Auto leaves the original for the transcode ladder and comes back. The
// ladder itself adapts in the player (hls.js, Shaka, ExoPlayer, AVPlayer);
// what no player can do is move between the original and the ladder, as that
// is a new session. flutter/lib/features/player/quality.dart ports this.

/** One step of a session's `qualities`. */
export interface StreamQuality { label: string; width: number; height: number; bitrateBps: number }

/** The original file, Auto, or one step pinned by its bitrate. */
export type QualityChoice = { kind: "original" } | { kind: "auto" } | { kind: "pinned"; bitrateBps: number };

export const ORIGINAL: QualityChoice = { kind: "original" };

/** What to ask `startPlaybackSession` for. `leftOriginal` is Auto after the original struggled. */
export function sessionQuality(choice: QualityChoice, original: { bitrateBps?: number | null } | null, leftOriginal = false): { adaptive: boolean; maxBitrateBps: number | null } {
  if (choice.kind === "pinned") return { adaptive: false, maxBitrateBps: choice.bitrateBps };
  if (choice.kind === "original") return { adaptive: false, maxBitrateBps: null };
  // Under the original's own rate, so the server encodes rather than sending it again.
  const below = leftOriginal && original?.bitrateBps ? Math.floor(original.bitrateBps * 0.9) : null;
  return { adaptive: true, maxBitrateBps: below };
}

export const sameChoice = (a: QualityChoice, b: QualityChoice) =>
  a.kind === b.kind && (a.kind !== "pinned" || a.bitrateBps === (b as { bitrateBps: number }).bitrateBps);

/** The 16:9 frame a picture fits, as the server names its steps: 1280×544 is 720p. */
export function frameLabel(width: number | null | undefined, height: number | null | undefined): string | null {
  if (!width || !height) return null;
  const frames: [string, number, number][] = [["360p", 640, 360], ["480p", 854, 480], ["720p", 1280, 720], ["1080p", 1920, 1080], ["1440p", 2560, 1440], ["4K", 3840, 2160]];
  return frames.find(([, w, h]) => width * 20 <= w * 21 && height * 20 <= h * 21)?.[0] ?? "4K";
}

export function megabits(bps: number | null | undefined): string {
  if (!bps) return "";
  const value = bps / 1e6;
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} Mb/s`;
}

/** `1080p · 8.0 Mb/s` */
export const qualityText = (quality: StreamQuality) => `${quality.label} · ${megabits(quality.bitrateBps)}`;

/** A choice as it's kept between plays: `original`, `auto` or a bitrate. */
export function encodeChoice(choice: QualityChoice): string {
  return choice.kind === "pinned" ? String(choice.bitrateBps) : choice.kind;
}

export function decodeChoice(text: string | null | undefined): QualityChoice {
  if (text === "auto") return { kind: "auto" };
  const bitrateBps = Number(text);
  return Number.isFinite(bitrateBps) && bitrateBps > 0 ? { kind: "pinned", bitrateBps } : ORIGINAL;
}

/** Thresholds for `AutoQuality`, in milliseconds. */
export const AUTO = {
  /** After a start or a seek, filling the buffer isn't struggling. */
  settleMs: 8_000,
  windowMs: 60_000,
  /** Stalls within `windowMs` that mean the original is too much. */
  stalls: 2,
  /** One stall this long is enough by itself. */
  longStallMs: 4_000,
  /** How long the ladder must be comfortably over the original's rate before trying it again. */
  recoverMs: 60_000,
  /** How much over: room for the original's peaks. */
  headroom: 1.5,
  /** The least time on the ladder after leaving the original. */
  holdMs: 180_000,
};

/**
 * Auto's two decisions, fed the player's events. On the original: whether it's
 * stalling enough to leave it for the ladder. On the ladder: whether the
 * connection has been comfortably above the original's bitrate for long enough
 * to go back to it. `now` is a clock in milliseconds.
 */
export class AutoQuality {
  #settledAt = 0;
  #stalls: number[] = [];
  #stalledAt: number | null = null;
  #leftAt: number | null = null;
  #comfortableSince: number | null = null;

  constructor(private readonly now: () => number = () => Date.now()) {}

  /** The stream started or the viewer seeked: what follows is loading, not struggling. */
  settle(): void {
    this.#settledAt = this.now();
    this.#stalledAt = null;
  }

  stalled(): void {
    const now = this.now();
    if (this.#stalledAt != null || now - this.#settledAt < AUTO.settleMs) return;
    this.#stalledAt = now;
    this.#stalls = [...this.#stalls.filter((at) => now - at < AUTO.windowMs), now];
  }

  resumed(): void {
    this.#stalledAt = null;
  }

  /** On the original: whether to leave it for the ladder. */
  struggling(): boolean {
    const now = this.now();
    if (this.#stalledAt != null && now - this.#stalledAt >= AUTO.longStallMs) return true;
    return this.#stalls.filter((at) => now - at < AUTO.windowMs).length >= AUTO.stalls;
  }

  /** The session moved to the ladder because the original struggled. */
  leftOriginal(): void {
    this.#leftAt = this.now();
    this.#stalls = [];
    this.#comfortableSince = null;
  }

  /**
   * On the ladder, with the player's bandwidth estimate: whether to try the
   * original again. Only after leaving it for a while, and only once the
   * estimate has stayed over the original's rate with headroom.
   */
  recovered(estimateBps: number | null | undefined, originalBps: number | null | undefined): boolean {
    const now = this.now();
    if (this.#leftAt == null || !originalBps) return false;
    if (!estimateBps || estimateBps < originalBps * AUTO.headroom) {
      this.#comfortableSince = null;
      return false;
    }
    this.#comfortableSince ??= now;
    return now - this.#leftAt >= AUTO.holdMs && now - this.#comfortableSince >= AUTO.recoverMs;
  }
}
