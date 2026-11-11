// SPDX-License-Identifier: AGPL-3.0-only
//
// The seam between the transport UI and what actually decodes video: a `<video>`
// behind the canvas on the web hosts, AVPlayerLayer behind it on Apple's.

export interface VideoSource {
  url: string;
  /** Adaptive (server-encoded or remuxed), so it needs an HLS engine; else a plain file. */
  hls: boolean;
  live: boolean;
  /** Index among the audio streams of a version that lists them all. */
  audioStream?: number | null;
  /**
   * Stream time to start at. An encode's playlist covers the whole title, so starting
   * anywhere else first would have the server encode from the top for nothing.
   */
  startSeconds?: number;
  /** What the last stream measured, for an engine choosing a ladder's first step. */
  bandwidthBps?: number | null;
}

export interface AudioTrack { id: number; label: string }

export type PlayerEvent =
  | { type: "ready" }
  | { type: "playing" }
  | { type: "paused" }
  | { type: "waiting" }
  /** Filling the buffer after this isn't a stall. */
  | { type: "seeking" }
  | { type: "time" }
  | { type: "ended" }
  | { type: "tracks"; tracks: AudioTrack[]; current: number }
  /** The step of a ladder now playing, where the engine says. */
  | { type: "level"; height: number; bitrateBps: number }
  | { type: "error"; message: string };

/** Whatever this engine can say about what it's doing. Blank where it can't. */
export interface PlayerStats {
  width: number;
  height: number;
  droppedFrames?: number;
  totalFrames?: number;
  bandwidthBps?: number;
  streamBitrateBps?: number;
}

export interface VideoPlayer {
  load(source: VideoSource): Promise<void>;
  play(): void;
  pause(): void;
  /** Stream time, in seconds: the session's `startPositionMs` is the caller's to add. */
  seek(seconds: number): void;
  position(): number;
  duration(): number;
  /** How far the stream reaches, for telling a seek the transcoder can serve from one it can't. */
  seekableEnd(): number;
  /** Stream time the buffer reaches from the playhead. */
  buffered(): number;
  paused(): boolean;
  setAudioTrack(id: number): void;
  setRate(rate: number): void;
  stats(): PlayerStats;
  on(listener: (event: PlayerEvent) => void): void;
  destroy(): void;
}
