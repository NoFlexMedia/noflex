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
}

export interface AudioTrack { id: number; label: string }

export type PlayerEvent =
  | { type: "ready" }
  | { type: "playing" }
  | { type: "paused" }
  | { type: "waiting" }
  | { type: "time" }
  | { type: "ended" }
  | { type: "tracks"; tracks: AudioTrack[]; current: number }
  | { type: "error"; message: string };

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
  paused(): boolean;
  setAudioTrack(id: number): void;
  on(listener: (event: PlayerEvent) => void): void;
  destroy(): void;
}
