// SPDX-License-Identifier: AGPL-3.0-only
//
// Google Cast from the sending side, for a browser that can load Google's sender
// script (Chrome, when the server's `cast_sdk` admits it). A session on a Cast
// device shows up beside the viewer's own devices as one more place to play,
// and is steered from the same bar. The receiver (cast/receiver) only plays: this
// resolves the title, opens the playback session, resumes from the viewer's
// place and reports progress, for as long as the session lasts.

import { MarkPlayedStore, ReportProgressStore } from "$houdini";
import { castCapabilities } from "$lib/castProfile.js";
import { connect, type Control, type RemoteDevice } from "$lib/connect.svelte.js";
import { request, writeUserData } from "$lib/graphql.js";
import { pickImage } from "$lib/images.js";
import { resolvePlayable, startSession, stopSession, subtitleFor } from "$lib/playback.svelte.js";
import { app } from "$lib/session.svelte.js";
import { mediaUrl } from "$lib/tokens.js";
import { toast } from "$lib/ui.svelte.js";

const SDK = "https://www.gstatic.com/cast/sdk/libs/sender/1.0/cast_framework.js?loadCastFramework=1";
/** Google's Default Media Receiver until a published NoFlex receiver's id is built in. */
const APP_ID = (import.meta.env.VITE_CAST_APP_ID as string | undefined) || "CC1AD845";
/** The dialect `cast/receiver` is told this sender speaks. */
const PROTOCOL = 1;
const REPORT_MS = 15_000;
// A transcode's seek past what it has produced restarts it at the target, as the player does.
const TRANSCODE_REACH = 30;

const sdk = () => window as any;
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** What this sender loaded, which the Cast device doesn't know. */
interface Loaded {
  work: any;
  mediaFileId: string;
  editionId: string;
  sessionId: string;
  transcode: boolean;
  /** Source time of the stream's zero. */
  offsetMs: number;
  /** The real length, as a growing transcode doesn't know it. */
  durationMs: number | null;
}

/** A Chromecast can't reach this browser's own loopback address. */
const loopback = () => /^(localhost|127\.|\[?::1\]?$)/.test(location.hostname);
const absolute = (url: string) => new URL(mediaUrl(url), location.origin).href;

async function playlistReady(url: string): Promise<void> {
  for (let attempt = 0; attempt < 40; attempt++) {
    try { if ((await fetch(url)).ok) return; } catch { /* the encoder isn't there yet */ }
    await sleep(500);
  }
  throw new Error("The stream didn’t start in time.");
}

class Cast {
  /** Whether the sender script loaded and Cast devices are on the network. */
  nearby = $state(false);
  #loaded: Loaded | null = null;
  #remote: any = null;
  #controller: any = null;
  #beat: ReturnType<typeof setInterval> | undefined;
  #last: "PLAYING" | "PAUSED" | null = null;
  #started = false;

  /** Load Google's script once, if the server admits it. Returns the disposer. */
  run(): () => void {
    if (!app.server?.castSdk || this.#started) return () => {};
    this.#started = true;
    sdk().__onGCastApiAvailable = (available: boolean) => { if (available) this.#init(); };
    const script = document.createElement("script");
    script.src = SDK;
    script.async = true;
    document.head.append(script);
    return () => { void this.#finish(false); connect.external = []; };
  }

  #init(): void {
    const { cast, chrome } = sdk();
    const context = cast.framework.CastContext.getInstance();
    context.setOptions({
      receiverApplicationId: APP_ID,
      autoJoinPolicy: chrome.cast.AutoJoinPolicy.ORIGIN_SCOPED,
      resumeSavedSession: true,
    });
    this.#remote = new cast.framework.RemotePlayer();
    this.#controller = new cast.framework.RemotePlayerController(this.#remote);
    this.#controller.addEventListener(cast.framework.RemotePlayerEventType.ANY_CHANGE, () => this.#publish());
    context.addEventListener(cast.framework.CastContextEventType.CAST_STATE_CHANGED, (event: any) => {
      this.nearby = event.castState !== cast.framework.CastState.NO_DEVICES_AVAILABLE;
    });
    context.addEventListener(cast.framework.CastContextEventType.SESSION_STATE_CHANGED, (event: any) => {
      const { SessionState } = cast.framework;
      if (event.sessionState === SessionState.SESSION_STARTED || event.sessionState === SessionState.SESSION_RESUMED) {
        const device = this.#session()?.getCastDevice();
        if (device && event.sessionState === SessionState.SESSION_STARTED) connect.setTarget(this.#id(device));
      }
      if (event.sessionState === SessionState.SESSION_ENDED) void this.#finish(false);
      this.#publish();
    });
    connect.routes.set("cast", { cast: (work, options) => this.load(work, options), control: (device, action, positionMs) => this.control(device, action, positionMs) });
    this.nearby = context.getCastState() !== cast.framework.CastState.NO_DEVICES_AVAILABLE;
    this.#publish();
  }

  #session(): any { return sdk().cast.framework.CastContext.getInstance().getCurrentSession(); }
  #id(device: any): string { return `cast:${device.friendlyName}`; }

  /** Source time now, in milliseconds. */
  #positionMs(): number {
    const loaded = this.#loaded;
    return Math.max(0, Math.round((loaded?.offsetMs ?? 0) + (this.#remote?.currentTime ?? 0) * 1000));
  }

  /** Tell the shared list what the Cast device is doing. */
  #publish(): void {
    const device = this.#session()?.getCastDevice();
    if (!device) {
      connect.external = [];
      if (connect.targetId?.startsWith("cast:")) connect.setTarget(null);
      return;
    }
    const remote = this.#remote;
    const loaded = this.#loaded;
    const state = remote.playerState as string | null;
    const live = loaded && remote.isMediaLoaded && state && state !== "IDLE";
    // A finished or stopped title leaves the device idle; what it was is then ours to close out.
    if (loaded && !live && state === "IDLE" && !remote.isMediaLoaded) {
      void this.#finish(this.#session()?.getMediaSession?.()?.idleReason === "FINISHED");
    }
    if (live) {
      const now = state === "PAUSED" ? "PAUSED" : "PLAYING";
      if (now !== this.#last) { this.#last = now; this.#report(now); }
    }
    connect.external = [{
      online: true,
      device: { id: this.#id(device), name: device.friendlyName, client: "Chromecast" },
      playback: live ? {
        state: state === "PLAYING" ? "PLAYING" : state === "PAUSED" ? "PAUSED" : "BUFFERING",
        positionMs: this.#positionMs(),
        durationMs: loaded.durationMs ?? (remote.duration ? Math.round(remote.duration * 1000) : null),
        work: loaded.work,
      } : null,
      heardAt: Date.now(),
      via: "cast",
    } as unknown as RemoteDevice];
  }

  #report(state: "PLAYING" | "PAUSED" | "STOPPED"): void {
    const loaded = this.#loaded;
    if (!loaded?.work?.id) return;
    request(new ReportProgressStore(), { id: loaded.work.id, position: this.#positionMs(), state })
      .then((data) => writeUserData(loaded.work.id, data.reportProgress), () => {});
  }

  /** End what was playing: its last position, its watched mark if it ran out, and its server session. */
  async #finish(ended: boolean): Promise<void> {
    const loaded = this.#loaded;
    if (!loaded) return;
    this.#loaded = null;
    this.#last = null;
    clearInterval(this.#beat);
    this.#report("STOPPED");
    if (ended) request(new MarkPlayedStore(), { id: loaded.work.id }).then((data) => writeUserData(loaded.work.id, data.markPlayed), () => {});
    await stopSession(loaded.sessionId);
  }

  /** Start `work` on the device in this session, from the viewer's place unless told otherwise. */
  async load(work: { id: string; title?: string }, options: { editionId?: string; mediaFileId?: string; positionMs?: number } = {}): Promise<boolean> {
    const session = this.#session();
    if (!session) return false;
    if (loopback()) {
      toast("Open NoFlex at this server’s network address, not localhost, to cast: a Cast device couldn’t reach this one.", "error");
      return false;
    }
    const { chrome } = sdk();
    try {
      const model: string = session.getCastDevice().modelName ?? "";
      const playable = await resolvePlayable(work, options);
      await this.#finish(false);
      const stream = await startSession(playable.mediaFileId, playable.burnedSubtitle, playable.resume, playable.audioStream, castCapabilities(model));
      const url = absolute(stream.url);
      const transcode = stream.route === "TRANSCODE";
      if (stream.route !== "DIRECT") await playlistReady(url);

      const detail = playable.detail.work ?? work;
      const info = new chrome.cast.media.MediaInfo(url, stream.route === "DIRECT" ? (/webm/.test(stream.decision?.source?.container ?? "") ? "video/webm" : "video/mp4") : "application/x-mpegURL");
      info.streamType = chrome.cast.media.StreamType.BUFFERED;
      info.metadata = new chrome.cast.media.GenericMediaMetadata();
      info.metadata.title = detail.title;
      info.metadata.subtitle = subtitleFor(detail);
      const poster = pickImage(detail.images, "poster");
      if (poster) info.metadata.images = [new chrome.cast.Image(absolute(poster))];
      const subtitles = stream.subtitles ?? [];
      info.tracks = subtitles.map((item, position) => {
        const track = new chrome.cast.media.Track(position + 1, chrome.cast.media.TrackType.TEXT);
        track.trackContentId = absolute(item.url);
        track.trackContentType = "text/vtt";
        track.subtype = chrome.cast.media.TextTrackType.SUBTITLES;
        track.name = item.title || item.language || `Track ${position + 1}`;
        track.language = item.language ?? undefined;
        return track;
      });
      info.customData = { protocol: PROTOCOL, workId: detail.id };
      const request_ = new chrome.cast.media.LoadRequest(info);
      request_.autoplay = true;
      // A transcode already starts at the resume point; a file seeks to it.
      request_.currentTime = transcode ? 0 : playable.resume / 1000;
      const chosen = subtitles.findIndex((item) => item.index === playable.subtitleIndex);
      if (chosen >= 0) request_.activeTrackIds = [chosen + 1];

      const runtime = (playable.edition?.runtimeMs ?? detail.runtimeMs ?? 0) || null;
      this.#loaded = {
        work: detail, mediaFileId: playable.mediaFileId, editionId: playable.edition.id, sessionId: stream.sessionId,
        transcode, offsetMs: transcode ? (stream.startPositionMs ?? 0) : 0, durationMs: runtime,
      };
      await session.loadMedia(request_);
      clearInterval(this.#beat);
      this.#beat = setInterval(() => { if (this.#last === "PLAYING") this.#report("PLAYING"); }, REPORT_MS);
      this.#publish();
      toast(`Playing on ${session.getCastDevice().friendlyName}`, "success");
      return true;
    } catch (cause) {
      void this.#finish(false);
      toast(typeof cause === "string" ? `That Cast device couldn’t play this (${cause}).` : cause instanceof Error ? cause.message : "That Cast device couldn’t play this.", "error");
      return false;
    }
  }

  /** Steer the title on the Cast device. */
  async control(device: RemoteDevice, action: Control, positionMs?: number): Promise<void> {
    const loaded = this.#loaded;
    const controller = this.#controller;
    if (!loaded || !controller) return;
    if (action === "STOP") {
      controller.stop();
      await this.#finish(false);
      this.#publish();
      return;
    }
    if (action === "SEEK" && positionMs !== undefined) {
      const local = (positionMs - loaded.offsetMs) / 1000;
      const produced = this.#remote?.duration ?? 0;
      // Beyond what a transcode has produced, a new session starting there beats waiting for the encoder.
      if (loaded.transcode && (local < 0 || local > produced + TRANSCODE_REACH)) {
        await this.load(loaded.work, { editionId: loaded.editionId, mediaFileId: loaded.mediaFileId, positionMs });
        return;
      }
      this.#remote.currentTime = local;
      controller.seek();
    } else if ((action === "PAUSE") !== this.#remote.isPaused) {
      controller.playOrPause();
    }
    this.#publish();
  }
}

export const cast = new Cast();
