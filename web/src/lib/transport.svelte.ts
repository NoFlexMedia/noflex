// SPDX-License-Identifier: AGPL-3.0-only
//
// What the open player lets another device do to it. The player registers this
// while it's open, so a command from the viewer's other devices steers it as the
// keyboard does.

export interface DeviceState { workId: string; state: "PLAYING" | "PAUSED" | "BUFFERING"; positionMs: number; durationMs: number | null }

export interface Transport {
  play(): void;
  pause(): void;
  seekTo(seconds: number): void;
  /** Close the player, as Escape does. */
  stop(): void;
  /** End the current session so another title can replace it, as moving to the next episode does. */
  finish(): Promise<void>;
  state(): DeviceState | null;
}

export const transport = $state<{ current: Transport | null }>({ current: null });
