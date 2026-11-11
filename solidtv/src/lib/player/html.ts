// SPDX-License-Identifier: AGPL-3.0-only
//
// Playback on every host with a browser engine: a <video> under the transparent
// canvas (see public/styles.css), on hls.js wherever there's MSE, for the same
// reasons as the web client: it retries playlists the transcoder hasn't finished
// writing, which native HLS won't, and waits as long as an encode takes.

import type { AudioTrack, PlayerEvent, PlayerStats, VideoPlayer, VideoSource } from "$lib/player/types.js";
import { languageName } from "$lib/format.js";

export class HtmlVideoPlayer implements VideoPlayer {
  private video: HTMLVideoElement;
  private hls: any = null;
  private listener: (event: PlayerEvent) => void = () => {};
  private disposed = false;
  /** Where to start once the engine knows the stream, for those that can't be told up front. */
  private start = 0;
  private readonly handlers: [string, () => void][];

  constructor() {
    this.video = document.createElement("video");
    this.video.id = "nf-video";
    this.video.setAttribute("playsinline", "");
    // Under the app root, whose canvas shows it through.
    document.body.insertBefore(this.video, document.body.firstChild);
    this.handlers = [
      ["loadedmetadata", () => {
        if (this.start > 1 && this.video.currentTime < 1) this.video.currentTime = this.start;
        this.start = 0;
        this.emit({ type: "ready" });
      }],
      ["playing", () => this.emit({ type: "playing" })],
      ["pause", () => this.emit({ type: "paused" })],
      ["waiting", () => this.emit({ type: "waiting" })],
      ["seeking", () => this.emit({ type: "seeking" })],
      ["timeupdate", () => this.emit({ type: "time" })],
      ["ended", () => this.emit({ type: "ended" })],
      ["error", () => { if (!this.hls) this.emit({ type: "error", message: "This stream couldn’t be played." }); }],
    ];
    for (const [name, handler] of this.handlers) this.video.addEventListener(name, handler);
  }

  private emit(event: PlayerEvent): void { if (!this.disposed) this.listener(event); }
  on(listener: (event: PlayerEvent) => void): void { this.listener = listener; }

  async load(source: VideoSource): Promise<void> {
    this.start = source.live ? 0 : source.startSeconds ?? 0;
    if (!source.hls) {
      this.video.src = source.url;
      this.play();
      return;
    }
    const { default: Hls } = await import("hls.js");
    if (this.disposed) return;
    if (!Hls.isSupported()) {
      if (!this.video.canPlayType("application/vnd.apple.mpegurl")) {
        this.emit({ type: "error", message: "Adaptive streaming isn’t supported on this device." });
        return;
      }
      this.video.src = source.url;
      this.play();
      return;
    }
    const retry = { maxNumRetry: 8, retryDelayMs: 500, maxRetryDelayMs: 3000, shouldRetry: (config: { maxNumRetry: number }, count: number) => count < config.maxNumRetry };
    const policy = { default: { maxTimeToFirstByteMs: 15_000, maxLoadTimeMs: 30_000, timeoutRetry: retry, errorRetry: retry } };
    // The server encodes a segment when it's asked for, and holds the request for up to 45 s.
    const fragments = { default: { maxTimeToFirstByteMs: 50_000, maxLoadTimeMs: 120_000, timeoutRetry: retry, errorRetry: retry } };
    // Told where to start, as a linear transcode's growing playlist would otherwise be
    // taken for a live stream and joined at the edge.
    const start = this.start;
    this.start = 0;
    const hls = new Hls({
      enableWorker: false, backBufferLength: 30, maxBufferLength: 30,
      manifestLoadPolicy: policy, playlistLoadPolicy: policy, fragLoadPolicy: fragments,
      // A ladder starts as high as the last stream held, or at the top: Auto tries the best first.
      abrEwmaDefaultEstimate: source.bandwidthBps ?? 50_000_000,
      ...(source.live ? {} : { startPosition: start }),
    } as any);
    this.hls = hls;
    hls.loadSource(source.url);
    hls.attachMedia(this.video);
    hls.on(Hls.Events.MANIFEST_PARSED, () => this.play());
    let chosen = source.audioStream ?? null;
    hls.on(Hls.Events.AUDIO_TRACKS_UPDATED, (_event: unknown, data: any) => {
      const tracks: AudioTrack[] = data.audioTracks.map((track: any) => ({ id: track.id, label: languageName(track.lang) || track.name }));
      // A prepared version lists every stream, in the source's order.
      if (chosen != null && chosen < data.audioTracks.length) hls.audioTrack = chosen;
      chosen = null;
      this.emit({ type: "tracks", tracks, current: hls.audioTrack });
    });
    hls.on(Hls.Events.AUDIO_TRACK_SWITCHED, (_event: unknown, data: any) => {
      this.emit({ type: "tracks", tracks: hls.audioTracks.map((track: any) => ({ id: track.id, label: languageName(track.lang) || track.name })), current: data.id });
    });
    hls.on(Hls.Events.LEVEL_SWITCHED, (_event: unknown, data: any) => {
      const level = hls.levels?.[data.level];
      if (level && hls.levels.length > 1) this.emit({ type: "level", height: level.height, bitrateBps: level.bitrate });
    });
    hls.on(Hls.Events.ERROR, (_event: unknown, data: any) => {
      if (!data.fatal) return;
      if (data.type === Hls.ErrorTypes.NETWORK_ERROR) hls.startLoad();
      else if (data.type === Hls.ErrorTypes.MEDIA_ERROR) hls.recoverMediaError();
      else this.emit({ type: "error", message: "This stream couldn’t be played." });
    });
  }

  play(): void {
    // Older engines return nothing from play().
    (this.video.play() as Promise<void> | undefined)?.catch?.(() => {});
  }
  pause(): void { this.video.pause(); }
  seek(seconds: number): void { this.video.currentTime = seconds; }
  position(): number { return Number.isFinite(this.video.currentTime) ? this.video.currentTime : 0; }
  duration(): number { return Number.isFinite(this.video.duration) ? this.video.duration : 0; }
  seekableEnd(): number {
    const ranges = this.video.seekable;
    return ranges.length ? ranges.end(ranges.length - 1) : this.duration();
  }
  buffered(): number {
    const ranges = this.video.buffered;
    const at = this.video.currentTime;
    for (let index = 0; index < ranges.length; index++) if (ranges.start(index) <= at && at <= ranges.end(index)) return ranges.end(index);
    return at;
  }
  paused(): boolean { return this.video.paused; }
  setAudioTrack(id: number): void { if (this.hls) this.hls.audioTrack = id; }
  setRate(rate: number): void { this.video.playbackRate = rate; }
  stats(): PlayerStats {
    const quality = this.video.getVideoPlaybackQuality?.();
    return {
      width: this.video.videoWidth,
      height: this.video.videoHeight,
      droppedFrames: quality?.droppedVideoFrames,
      totalFrames: quality?.totalVideoFrames,
      bandwidthBps: this.hls?.bandwidthEstimate,
      streamBitrateBps: this.hls?.levels?.[this.hls.currentLevel]?.bitrate,
    };
  }

  destroy(): void {
    this.disposed = true;
    for (const [name, handler] of this.handlers) this.video.removeEventListener(name, handler);
    this.hls?.destroy();
    this.hls = null;
    this.video.removeAttribute("src");
    this.video.load();
    this.video.parentNode?.removeChild(this.video);
  }
}
