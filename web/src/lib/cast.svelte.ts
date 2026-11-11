// SPDX-License-Identifier: AGPL-3.0-only
//
// Google Cast from the sending side, for a browser that can load Google's sender
// script (Chrome, when the server's `cast_sdk` admits it). A session on a Cast
// device shows up beside the viewer's own devices as one more place to play,
// and is steered from the same bar. The receiver (cast/receiver) only plays: this
// resolves the title, opens the playback session, resumes from the viewer's
// place and reports progress, for as long as the session lasts.

import { MarkPlayedStore, ReportProgressStore } from "$houdini";
import type { ClientCapabilities } from "$codecs";
import { AutoQuality, frameLabel, sameChoice, sessionQuality, type QualityChoice, type StreamQuality } from "$quality";
import { castCapabilities, probeCast, type CastDevice } from "$lib/castProfile.js";
import { connect, type Control, type RemoteDevice } from "$lib/connect.svelte.js";
import { request, writeUserData } from "$lib/graphql.js";
import { pickImage } from "$lib/images.js";
import { qualityChoice, rememberQuality, resolvePlayable, startSession, stopSession, subtitleFor, type PlaybackDecision } from "$lib/playback.svelte.js";
import { app } from "$lib/session.svelte.js";
import { mediaUrl } from "$lib/tokens.js";
import { toast } from "$lib/ui.svelte.js";

const SDK = "https://www.gstatic.com/cast/sdk/libs/sender/1.0/cast_framework.js?loadCastFramework=1";
const DEFAULT_RECEIVER = "CC1AD845";
/** Google's Default Media Receiver until a published NoFlex receiver's id is built in. */
const APP_ID = (import.meta.env.VITE_CAST_APP_ID as string | undefined) || DEFAULT_RECEIVER;
/** Where NoFlex's receiver answers what its device decodes (cast/receiver). */
const NAMESPACE = "urn:x-cast:dev.noflex.cast";
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
  quality: QualityChoice;
  /** Auto moved this title off the original because it struggled. */
  leftOriginal: boolean;
  /** The stream is a ladder the receiver adapts along. */
  adaptive: boolean;
  /** The original file, for Auto to weigh the connection against. */
  source: PlaybackDecision["source"] | null;
}

const PLAN: Record<string, string> = { DIRECT_PLAY: "Direct play", REMUX: "Direct stream (remux)", TRANSCODE_AUDIO: "Transcoding audio", TRANSCODE_VIDEO: "Transcoding video", TRANSCODE: "Transcoding" };

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
  /** The ladder the title on the device is offered at, for the quality menu. */
  qualities = $state<StreamQuality[]>([]);
  choice = $state<QualityChoice>({ kind: "original" });
  /** The receiver is showing its stream info panel. */
  statsOnTv = $state(false);
  #auto = new AutoQuality();
  /** The receiver's latest report on playback. */
  #health: { estimatedBandwidthBps?: number | null } | null = null;
  #switching = false;
  #loaded: Loaded | null = null;
  #remote: any = null;
  #controller: any = null;
  #beat: ReturnType<typeof setInterval> | undefined;
  #last: "PLAYING" | "PAUSED" | null = null;
  #started = false;
  /** A title being started, which the Cast device hasn't answered for yet. */
  #preparing: { work: any; positionMs: number } | null = null;
  /** What each device decodes, probed once per page. */
  #profiles = new Map<string, Promise<ClientCapabilities>>();
  #replies = new Map<number, (message: any) => void>();
  #nextMessage = 0;

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
        const session = this.#session();
        const device = session?.getCastDevice();
        if (device && event.sessionState === SessionState.SESSION_STARTED) connect.setTarget(this.#id(device));
        if (session && APP_ID !== DEFAULT_RECEIVER) {
          session.addMessageListener(NAMESPACE, (_namespace: string, data: string) => {
            try {
              const message = JSON.parse(data);
              if (message.type === "health") this.#onHealth(message);
              else this.#replies.get(message.id)?.(message);
            } catch { /* not ours to read */ }
          });
          // Probe while the viewer picks a title, so loading doesn't wait on it.
          void this.#profile(session);
        }
      }
      if (event.sessionState === SessionState.SESSION_ENDED) void this.#finish(false);
      this.#publish();
    });
    setInterval(() => this.#steer(), 1000);
    connect.routes.set("cast", { cast: (work, options) => this.load(work, options), control: (device, action, positionMs) => this.control(device, action, positionMs) });
    this.nearby = context.getCastState() !== cast.framework.CastState.NO_DEVICES_AVAILABLE;
    this.#publish();
  }

  #session(): any { return sdk().cast.framework.CastContext.getInstance().getCurrentSession(); }

  #onHealth(health: any): void {
    this.#health = health;
    if (health.kind === "loaded") this.#auto.settle();
    else if (health.kind === "stalled" && !health.seeking) this.#auto.stalled();
    else if (health.kind === "resumed") this.#auto.resumed();
  }

  /** Auto moves between the original and the ladder; along the ladder, the receiver adapts. */
  #steer(): void {
    const loaded = this.#loaded;
    if (!loaded || this.#switching || loaded.quality.kind !== "auto" || this.#remote?.isPaused) return;
    if (!loaded.adaptive && !loaded.leftOriginal && this.#auto.struggling()) {
      this.#auto.leftOriginal();
      void this.#reload({ leftOriginal: true }, "Lowered the quality to keep up with the connection");
    } else if (loaded.adaptive && loaded.leftOriginal && this.#auto.recovered(this.#health?.estimatedBandwidthBps, loaded.source?.bitrateBps)) {
      void this.#reload({ leftOriginal: false }, "Back to the original quality");
    }
  }

  /** Load what's playing again from where it is, at another quality. */
  async #reload(change: { quality?: QualityChoice; leftOriginal?: boolean }, notice?: string): Promise<void> {
    const loaded = this.#loaded;
    if (!loaded) return;
    this.#switching = true;
    try {
      const done = await this.load(loaded.work, { editionId: loaded.editionId, mediaFileId: loaded.mediaFileId, positionMs: this.#positionMs(), quality: change.quality ?? loaded.quality, leftOriginal: change.leftOriginal ?? false });
      if (done && notice) {
        toast(notice);
        this.#session()?.sendMessage(NAMESPACE, { type: "notice", text: notice }).catch(() => {});
      }
    } finally {
      this.#switching = false;
    }
  }

  /** Play what's on the device at `choice` from now on, as this browser does. */
  setQuality(choice: QualityChoice): void {
    rememberQuality(choice);
    if (this.#loaded && !sameChoice(choice, this.#loaded.quality)) void this.#reload({ quality: choice });
  }

  /** Show or hide the receiver's stream info panel. */
  showStats(show: boolean): void {
    this.statsOnTv = show;
    this.#session()?.sendMessage(NAMESPACE, { type: "stats", show }).catch(() => {});
  }

  /** Send the receiver `message` and wait for its reply. */
  #ask(session: any, message: object, timeoutMs: number): Promise<any> {
    const id = ++this.#nextMessage;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.#replies.delete(id); reject(new Error("The Cast device didn’t answer.")); }, timeoutMs);
      this.#replies.set(id, (reply) => { clearTimeout(timer); this.#replies.delete(id); resolve(reply); });
      session.sendMessage(NAMESPACE, { ...message, id }).catch((cause: unknown) => { clearTimeout(timer); this.#replies.delete(id); reject(cause); });
    });
  }

  /** What the session's device decodes: asked of NoFlex's receiver, or looked up by model for Google's. */
  #profile(session: any): Promise<ClientCapabilities> {
    const device = session.getCastDevice();
    const id = this.#id(device);
    const fallback = () => castCapabilities(device.modelName ?? "");
    if (APP_ID === DEFAULT_RECEIVER) return Promise.resolve(fallback());
    let profile = this.#profiles.get(id);
    if (!profile) {
      profile = this.#ask(session, { type: "device" }, 3000)
        .then((info: CastDevice) => probeCast(info, (questions) => this.#ask(session, { type: "probe", questions }, 5000).then((reply) => reply.answers ?? [])))
        .catch((cause) => {
          console.warn("Cast device probe failed; going by its model", cause);
          this.#profiles.delete(id);
          return fallback();
        });
      this.#profiles.set(id, profile);
    }
    return profile;
  }
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
    const preparing = !live ? this.#preparing : null;
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
      // The model says what it is, as a TV with Cast built in reports its own.
      device: { id: this.#id(device), name: device.friendlyName, client: [device.modelName, "Chromecast"].filter(Boolean).join(" · ") },
      playback: live ? {
        state: state === "PLAYING" ? "PLAYING" : state === "PAUSED" ? "PAUSED" : "BUFFERING",
        positionMs: this.#positionMs(),
        durationMs: loaded.durationMs ?? (remote.duration ? Math.round(remote.duration * 1000) : null),
        work: loaded.work,
      } : preparing ? { state: "BUFFERING", positionMs: preparing.positionMs, durationMs: null, work: preparing.work } : null,
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
    this.qualities = [];
    clearInterval(this.#beat);
    this.#report("STOPPED");
    if (ended) request(new MarkPlayedStore(), { id: loaded.work.id }).then((data) => writeUserData(loaded.work.id, data.markPlayed), () => {});
    await stopSession(loaded.sessionId);
  }

  /** Start `work` on the device in this session, from the viewer's place unless told otherwise. */
  async load(work: { id: string; title?: string }, options: { editionId?: string; mediaFileId?: string; positionMs?: number; quality?: QualityChoice; leftOriginal?: boolean } = {}): Promise<boolean> {
    const session = this.#session();
    if (!session) return false;
    if (loopback()) {
      toast("Open NoFlex at this server’s network address, not localhost, to cast: a Cast device couldn’t reach this one.", "error");
      return false;
    }
    const { chrome } = sdk();
    try {
      const playable = await resolvePlayable(work, options);
      const quality = options.quality ?? qualityChoice();
      const leftOriginal = options.leftOriginal ?? false;
      const same = this.#loaded?.mediaFileId === playable.mediaFileId;
      // Leaving the original asks for less than it; only the session that played it knows its rate.
      const original = same ? this.#loaded?.source ?? null : null;
      if (!same) this.#auto = new AutoQuality();
      // The receiver buffers while it loads, which is no sign of struggling.
      this.#auto.settle();
      const qualities = same ? this.qualities : [];
      await this.#finish(false);
      // From here the bar shows it loading, through opening the session and the encoder's first segments.
      this.#preparing = { work: playable.detail.work ?? work, positionMs: playable.resume };
      this.#publish();
      const stream = await startSession(playable.mediaFileId, playable.burnedSubtitle, playable.resume, playable.audioStream, await this.#profile(session), sessionQuality(quality, original, leftOriginal));
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
      const backdrop = pickImage(detail.images, "backdrop");
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
      const runtime = (playable.edition?.runtimeMs ?? detail.runtimeMs ?? 0) || null;
      const offsetMs = transcode ? (stream.startPositionMs ?? 0) : 0;
      const decision = stream.decision;
      // The ladder is the whole of it on the first session; a capped one offers less.
      const ladder = qualities.length ? qualities : stream.qualities ?? [];
      const pinned = quality.kind === "pinned" ? ladder.find((step) => step.bitrateBps === quality.bitrateBps)?.label : null;
      info.customData = {
        protocol: PROTOCOL,
        workId: detail.id,
        offsetMs,
        durationMs: runtime,
        backdrop: backdrop ? absolute(backdrop) : null,
        stream: decision ? {
          route: stream.route,
          adaptive: !!stream.adaptive,
          container: decision.target.container,
          encodes: decision.plan === "TRANSCODE_VIDEO" || decision.plan === "TRANSCODE",
          mode: PLAN[decision.plan] ?? decision.plan,
          reasons: decision.reasons.map((reason) => reason.description),
          source: decision.source,
          target: decision.target,
          encoder: decision.encoder ?? null,
          hardware: decision.hardware,
          quality: pinned ?? frameLabel(decision.target.width, decision.target.height),
        } : null,
      };
      const request_ = new chrome.cast.media.LoadRequest(info);
      request_.autoplay = true;
      // A linear transcode starts at the resume point; a file or an on-demand playlist seeks to it.
      request_.currentTime = Math.max(0, playable.resume - offsetMs) / 1000;
      const chosen = subtitles.findIndex((item) => item.index === playable.subtitleIndex);
      if (chosen >= 0) request_.activeTrackIds = [chosen + 1];

      this.#loaded = {
        work: detail, mediaFileId: playable.mediaFileId, editionId: playable.edition.id, sessionId: stream.sessionId,
        transcode, offsetMs, durationMs: runtime,
        quality, leftOriginal, adaptive: !!stream.adaptive, source: decision?.source ?? null,
      };
      this.qualities = ladder;
      this.choice = quality;
      await session.loadMedia(request_);
      this.#preparing = null;
      clearInterval(this.#beat);
      this.#beat = setInterval(() => { if (this.#last === "PLAYING") this.#report("PLAYING"); }, REPORT_MS);
      this.#publish();
      return true;
    } catch (cause) {
      this.#preparing = null;
      void this.#finish(false);
      this.#publish();
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
        await this.load(loaded.work, { editionId: loaded.editionId, mediaFileId: loaded.mediaFileId, positionMs, quality: loaded.quality, leftOriginal: loaded.leftOriginal });
        return;
      }
      this.#auto.settle();
      this.#remote.currentTime = local;
      controller.seek();
    } else if ((action === "PAUSE") !== this.#remote.isPaused) {
      controller.playOrPause();
    }
    this.#publish();
  }
}

export const cast = new Cast();
