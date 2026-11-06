// SPDX-License-Identifier: AGPL-3.0-only
//
// Playback on Apple's platforms: an AVPlayerLayer in a native view behind the
// transparent canvas, so video stays hardware-composited and HDR can reach the
// panel, which drawing it into a GL texture would give up.
//
// Status is polled rather than observed: KVO from NativeScript means subclassing
// NSObject with exposed selectors, for what a few reads a second tells just as well.

import type { View } from "@nativescript/core";
import type { AudioTrack, PlayerEvent, VideoPlayer, VideoSource } from "$lib/player/types.js";

// In the runtime's metadata, though @nativescript/types leaves CoreMedia's C functions out.
declare function CMTimeGetSeconds(time: any): number;
declare function CMTimeMakeWithSeconds(seconds: number, timescale: number): any;

const POLL_MS = 250;
const TIMESCALE = 600;
// How long a slow encoder or a live channel gets to write its first playlist.
const PLAYLIST_PATIENCE_MS = 60_000;

// AVPlayerItemStatus and AVPlayerTimeControlStatus.
const ITEM_READY = 1;
const ITEM_FAILED = 2;
const CONTROL_PAUSED = 0;
const CONTROL_WAITING = 1;
const CONTROL_PLAYING = 2;

let videoHost: View | null = null;
let setCanvasOpaque: (opaque: boolean) => void = () => {};

/** Where video goes, and how to let it show through. See app.ts. */
export function bindVideoHost(host: View, canvasOpaque: (opaque: boolean) => void): void {
  videoHost = host;
  setCanvasOpaque = canvasOpaque;
}

const seconds = (time: any): number => {
  const value = CMTimeGetSeconds(time);
  return Number.isFinite(value) ? value : 0;
};

/**
 * Hold until the encoder has written a playlist. AVPlayer gives up on the first
 * failed fetch, and the server holds a playlist request for up to 10 s before
 * answering 404, so a live channel or slow encoder would otherwise never start.
 * hls.js retries by itself; Flutter's libmpv needs the same `_awaitPlaylist`.
 */
async function awaitPlaylist(master: string, cancelled: () => boolean): Promise<void> {
  let media: string;
  try {
    const text = await (await fetch(master)).text();
    const entry = text.split("\n").map((line) => line.trim()).find((line) => line && line.charAt(0) !== "#");
    if (!entry) return;
    media = new URL(entry, master).toString();
  } catch {
    return;
  }
  const deadline = Date.now() + PLAYLIST_PATIENCE_MS;
  while (!cancelled() && Date.now() < deadline) {
    try {
      if ((await fetch(media)).ok) return;
    } catch { /* not there yet */ }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
}

export class AvVideoPlayer implements VideoPlayer {
  private player: AVPlayer = AVPlayer.new();
  private layer: AVPlayerLayer;
  private listener: (event: PlayerEvent) => void = () => {};
  private timer: ReturnType<typeof setInterval> | null = null;
  private endObserver: NSObjectProtocol | null = null;
  private disposed = false;
  private readied = false;
  private lastControl = -1;
  private lastPosition = -1;
  private audioGroup: AVMediaSelectionGroup | null = null;

  constructor() {
    this.layer = AVPlayerLayer.playerLayerWithPlayer(this.player);
    this.layer.videoGravity = AVLayerVideoGravityResizeAspect;
    const view = videoHost?.ios as UIView | undefined;
    if (view) {
      this.layer.frame = view.bounds;
      view.layer.addSublayer(this.layer);
    }
    setCanvasOpaque(false);
    try {
      AVAudioSession.sharedInstance().setCategoryError(AVAudioSessionCategoryPlayback);
      AVAudioSession.sharedInstance().setActiveError(true);
    } catch { /* plays with the default category */ }
  }

  private emit(event: PlayerEvent): void { if (!this.disposed) this.listener(event); }
  on(listener: (event: PlayerEvent) => void): void { this.listener = listener; }

  async load(source: VideoSource): Promise<void> {
    if (source.hls) await awaitPlaylist(source.url, () => this.disposed);
    if (this.disposed) return;
    const url = NSURL.URLWithString(source.url);
    if (!url) {
      this.emit({ type: "error", message: "This stream couldn’t be played." });
      return;
    }
    const item = AVPlayerItem.playerItemWithURL(url);
    this.endObserver = NSNotificationCenter.defaultCenter.addObserverForNameObjectQueueUsingBlock(AVPlayerItemDidPlayToEndTimeNotification, item, NSOperationQueue.mainQueue, () => this.emit({ type: "ended" }));
    this.player.replaceCurrentItemWithPlayerItem(item);
    this.player.play();
    this.timer = setInterval(() => this.tick(source), POLL_MS);
  }

  private tick(source: VideoSource): void {
    const item = this.player.currentItem;
    if (!item) return;
    if (item.status === ITEM_FAILED) {
      this.emit({ type: "error", message: "This stream couldn’t be played." });
      this.stopPolling();
      return;
    }
    if (item.status === ITEM_READY && !this.readied) {
      this.readied = true;
      this.emit({ type: "ready" });
      this.readTracks(source);
    }
    const control = this.player.timeControlStatus;
    if (control !== this.lastControl) {
      this.lastControl = control;
      if (control === CONTROL_PLAYING) this.emit({ type: "playing" });
      else if (control === CONTROL_PAUSED) this.emit({ type: "paused" });
      else if (control === CONTROL_WAITING) this.emit({ type: "waiting" });
    }
    const position = this.position();
    if (position !== this.lastPosition) {
      this.lastPosition = position;
      this.emit({ type: "time" });
    }
  }

  private readTracks(source: VideoSource): void {
    const item = this.player.currentItem;
    if (!item) return;
    this.audioGroup = item.asset.mediaSelectionGroupForMediaCharacteristic(AVMediaCharacteristicAudible);
    const options = this.audioGroup?.options;
    if (!options || options.count < 2) return;
    const tracks: AudioTrack[] = [];
    for (let id = 0; id < options.count; id += 1) tracks.push({ id, label: options.objectAtIndex(id).displayName });
    // A prepared version lists every stream, in the source's order.
    const chosen = source.audioStream != null && source.audioStream < tracks.length ? source.audioStream : 0;
    this.setAudioTrack(chosen);
    this.emit({ type: "tracks", tracks, current: chosen });
  }

  private stopPolling(): void {
    if (this.timer != null) clearInterval(this.timer);
    this.timer = null;
  }

  play(): void { this.player.play(); }
  pause(): void { this.player.pause(); }
  seek(target: number): void { this.player.seekToTime(CMTimeMakeWithSeconds(target, TIMESCALE)); }
  position(): number { return seconds(this.player.currentTime()); }
  duration(): number {
    const item = this.player.currentItem;
    return item ? seconds(item.duration) : 0;
  }
  seekableEnd(): number {
    const ranges = this.player.currentItem?.seekableTimeRanges;
    if (!ranges || ranges.count === 0) return this.duration();
    const range = ranges.objectAtIndex(ranges.count - 1).CMTimeRangeValue;
    return seconds(range.start) + seconds(range.duration);
  }
  paused(): boolean { return this.player.timeControlStatus === CONTROL_PAUSED; }
  setAudioTrack(id: number): void {
    const option = this.audioGroup?.options.objectAtIndex(id);
    if (option && this.audioGroup) this.player.currentItem?.selectMediaOptionInMediaSelectionGroup(option, this.audioGroup);
  }

  destroy(): void {
    this.disposed = true;
    this.stopPolling();
    if (this.endObserver) NSNotificationCenter.defaultCenter.removeObserver(this.endObserver);
    this.player.pause();
    this.player.replaceCurrentItemWithPlayerItem(null);
    this.layer.removeFromSuperlayer();
    setCanvasOpaque(true);
  }
}
