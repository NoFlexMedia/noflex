// SPDX-License-Identifier: AGPL-3.0-only

import { CachePolicy, ChildPageStore, MediaFilesStore, StartPlaybackStore, StopPlaybackStore, WorkDetailStore } from "$houdini";
import { clientCapabilities } from "$codecs";
import { decodeChoice, encodeChoice, sessionQuality, type QualityChoice, type StreamQuality } from "$quality";
import { connect } from "$lib/connect.svelte.js";
import { app } from "$lib/session.svelte.js";
import { syncWatchState } from "$lib/sync.js";
import { isImageSubtitle, preferredAudio, preferredSubtitle } from "$lib/tracks.js";
import { mediaUrl } from "$lib/tokens.js";
import { request } from "$lib/graphql.js";
import { toast } from "$lib/ui.svelte.js";

export interface StreamSummary {
  container?: string | null; videoCodec?: string | null; videoProfile?: string | null; videoLevel?: number | null;
  bitDepth?: number | null; width?: number | null; height?: number | null; videoRange?: string | null;
  audioCodec?: string | null; audioChannels?: number | null; bitrateBps?: number | null;
}

/** Why the server picked this route; see `PlaybackDecisionInfo`. */
export interface PlaybackDecision {
  route: string;
  plan: string;
  reasons: { code: string; description: string }[];
  source: StreamSummary;
  target: StreamSummary;
  encoder?: string | null;
  audioEncoder?: string | null;
  hardware: boolean;
  tonemap: boolean;
}

export interface SubtitleDelivery { index: number; language?: string | null; title?: string | null; forced: boolean; default: boolean; url: string }

export interface Session {
  sessionId: string;
  progressToken?: string | null;
  route: string;
  url: string;
  jobId?: string | null;
  /** Source position of the stream's time zero: a transcode's start offset, 0 for direct play. */
  startPositionMs?: number;
  decision?: PlaybackDecision;
  subtitles?: SubtitleDelivery[];
  /** The ladder this file is offered at, for the quality menu. */
  qualities?: StreamQuality[];
  /** `url` is a ladder the player adapts along. */
  adaptive?: boolean;
}

export interface Playback extends Session {
  workId: string | null;
  title: string;
  subtitle?: string;
  work?: any;
  /** Where to begin, in source time. */
  resumeMs?: number;
  edition?: any;
  mediaFileId?: string;
  live?: boolean;
  room?: { id: string; isHost: boolean } | null;
  /** Index among the file's audio streams, for restarts and multi-track versions. */
  audioStream?: number | null;
  /** The subtitle track to start with, as a text track or burned in. */
  subtitleIndex?: number | null;
  burnedSubtitle?: number | null;
  quality?: QualityChoice;
  /** Auto moved this title off the original because it struggled. */
  leftOriginal?: boolean;
}

export const player = $state<{ current: Playback | null; preparing: string | null }>({ current: null, preparing: null });

const QUALITY_KEY = "noflex.quality";

/** The last stream's bandwidth estimate, so the next one starts at a quality it can hold. */
export const measured: { bandwidthBps: number | null } = { bandwidthBps: null };

/** The quality this browser plays at, Original until the viewer picks another. */
export function qualityChoice(): QualityChoice {
  try { return decodeChoice(localStorage.getItem(QUALITY_KEY)); } catch { return { kind: "original" }; }
}

export function rememberQuality(choice: QualityChoice): void {
  try { localStorage.setItem(QUALITY_KEY, encodeChoice(choice)); } catch { /* storage unavailable */ }
}

/** What a session asks the server for at `choice`; see `sessionQuality`. */
export interface QualityRequest { adaptive: boolean; maxBitrateBps: number | null }

// Probe while the user browses so the first play doesn't wait on it.
if (typeof window !== "undefined") (window.requestIdleCallback ?? setTimeout)(() => { clientCapabilities().catch(() => {}); });

/** Open a session; a transcode starts at `startPositionMs` so far seeks don't wait on ffmpeg. */
export async function startSession(mediaFileId: string, subtitle: number | null = null, startPositionMs = 0, audioStream: number | null = null, profile?: any, quality: QualityRequest = sessionQuality(qualityChoice(), null)): Promise<Session> {
  // A device other than this browser plays it when `profile` says what that device decodes.
  const measured = profile ?? await clientCapabilities();
  const capabilities = quality.maxBitrateBps ? { ...measured, maxBitrateBps: quality.maxBitrateBps } : measured;
  const data = await request(new StartPlaybackStore(), { media: mediaFileId, capabilities, subtitle, start: Math.max(0, Math.round(startPositionMs)), audio: audioStream, adaptive: quality.adaptive });
  const session = data.startPlaybackSession as Session;
  return { ...session, url: mediaUrl(session.url), subtitles: session.subtitles?.map((track: any) => ({ ...track, url: mediaUrl(track.url) })) } as Session;
}

/** End a session server-side; best effort, a dead session is already gone. */
export function stopSession(sessionId: string | null | undefined): Promise<void> {
  if (!sessionId) return Promise.resolve();
  return request(new StopPlaybackStore(), { session: sessionId }).then(() => {}, () => {});
}

/** `audioStream` and `subtitleIndex` default to the viewer's preferences; null subtitles means off. */
export interface PlayOptions { editionId?: string; mediaFileId?: string; audioStream?: number; subtitleIndex?: number | null; fromStart?: boolean; positionMs?: number; room?: Playback["room"] }

/** A work resolved down to the one file to play and how to start it. */
export interface Playable {
  detail: any;
  edition: any;
  mediaFileId: string;
  audioStream: number | null;
  subtitleIndex: number | null;
  burnedSubtitle: number | null;
  /** Where to begin, in source time. */
  resume: number;
}

/**
 * Resolve a work to a media file (first edition, first part unless told
 * otherwise) with the viewer's audio and subtitle preferences, and where to
 * resume. Throws what to tell the viewer when there is nothing to play.
 */
export async function resolvePlayable(work: any, options: PlayOptions = {}): Promise<Playable> {
  const detail = await request(new WorkDetailStore(), { id: work.id }, { policy: CachePolicy.CacheOrNetwork });
  const editions = detail.editions ?? [];
  const edition = editions.find((item: any) => item.id === options.editionId) ?? editions[0];
  if (!edition) throw new Error(`“${work.title}” has nothing playable attached yet.`);
  const media = await request(new MediaFilesStore(), { edition: edition.id }, { policy: CachePolicy.CacheOrNetwork });
  const file: any = media.mediaFiles.find((item: any) => item.id === options.mediaFileId) ?? media.mediaFiles[0];
  const mediaFileId = options.mediaFileId ?? file?.id;
  if (!mediaFileId) throw new Error("This edition has no media files on disk.");
  const audioStream = options.audioStream ?? preferredAudio(file?.probe?.audio, app.playbackPrefs);
  const subtitleIndex = options.subtitleIndex !== undefined ? options.subtitleIndex : preferredSubtitle(file?.subtitleTracks, app.playbackPrefs);
  const burnedSubtitle = subtitleIndex != null && isImageSubtitle(file?.subtitleTracks?.[subtitleIndex]) ? subtitleIndex : null;
  const userData = detail.work?.userData;
  const resume = options.positionMs ?? (options.fromStart || userData?.played ? 0 : userData?.positionMs ?? 0);
  return { detail, edition, mediaFileId, audioStream, subtitleIndex, burnedSubtitle, resume };
}

/**
 * Resolve a work down to a playable media file, open a session, and hand it
 * to the player.
 */
export async function playWork(work: any, options: PlayOptions = {}): Promise<boolean> {
  if (player.preparing) return false;
  player.preparing = work.id;
  try {
    // A chosen device plays it, resuming from the viewer's place unless told otherwise.
    if (connect.target && !options.room) {
      return await connect.cast(work, { editionId: options.editionId, mediaFileId: options.mediaFileId, positionMs: options.fromStart ? 0 : options.positionMs });
    }
    const { detail, edition, mediaFileId, audioStream, subtitleIndex, burnedSubtitle, resume } = await resolvePlayable(work, options);
    const quality = qualityChoice();
    const session = await startSession(mediaFileId, burnedSubtitle, resume, audioStream, undefined, sessionQuality(quality, null));
    player.current = {
      ...session,
      workId: work.id,
      title: detail.work?.title ?? work.title,
      subtitle: subtitleFor(detail.work ?? work),
      work: detail.work ?? work,
      resumeMs: resume,
      edition,
      mediaFileId,
      room: options.room ?? null,
      audioStream,
      subtitleIndex,
      burnedSubtitle,
      quality,
    };
    return true;
  } catch (cause) {
    toast(cause instanceof Error ? cause.message : String(cause), "error");
    return false;
  } finally {
    player.preparing = null;
  }
}

export function closePlayer(): void {
  player.current = null;
  // Give the final progress report a moment to land.
  setTimeout(() => { void syncWatchState(); }, 2000);
}

export function subtitleFor(work: any): string {
  if (!work) return "";
  if (work.kind === "EPISODE" && work.indexNumber != null) {
    const episode = work.indexEnd != null ? `E${work.indexNumber}–${work.indexEnd}` : `E${work.indexNumber}`;
    return work.parentIndex != null ? `S${work.parentIndex} · ${episode}` : episode;
  }
  return work.year ? String(work.year) : "";
}

export const byIndex = (a: any, b: any) => (a.parentIndex ?? 0) - (b.parentIndex ?? 0) || (a.indexNumber ?? 1e9) - (b.indexNumber ?? 1e9) || String(a.title).localeCompare(String(b.title));

/**
 * Every direct child of `parentId`, in index order. Pages through the
 * cursor because the request-complexity budget caps a page at 60 cards.
 */
export async function fetchChildren(libraryId: string, parentId: string): Promise<any[]> {
  const items: any[] = [];
  let cursor: string | null = null;
  do {
    const data: any = await request(new ChildPageStore(), { library: libraryId, parent: parentId, after: cursor }, { policy: CachePolicy.CacheAndNetwork });
    items.push(...data.works.nodes);
    cursor = data.works.pageInfo.hasNextPage ? data.works.pageInfo.endCursor : null;
  } while (cursor);
  return items.sort(byIndex);
}

/** The episode/track after `work` within its parent, if any. */
export async function nextSibling(work: any): Promise<any | null> {
  if (!work?.parentId || !["EPISODE", "TRACK"].includes(work.kind)) return null;
  try {
    const siblings = await fetchChildren(work.libraryId, work.parentId);
    const index = siblings.findIndex((item: any) => item.id === work.id);
    return index >= 0 ? siblings[index + 1] ?? null : null;
  } catch {
    return null;
  }
}

/**
 * Replace the current session with one starting at `positionMs` — to burn
 * in a different subtitle, seek a transcode far from its offset, or play at
 * another quality. Resolves false (after a toast) when no new session could
 * be opened.
 */
export async function restartSession(subtitle: number | null, positionMs: number, change: { quality?: QualityChoice; leftOriginal?: boolean } = {}): Promise<boolean> {
  const current = player.current;
  if (!current?.mediaFileId) return false;
  const quality = change.quality ?? current.quality ?? qualityChoice();
  const leftOriginal = change.leftOriginal ?? (change.quality ? false : current.leftOriginal ?? false);
  try {
    await stopSession(current.sessionId);
    const session = await startSession(current.mediaFileId, subtitle, positionMs, current.audioStream ?? null, undefined, sessionQuality(quality, current.decision?.source ?? null, leftOriginal));
    // The ladder a session offers depends on the limit it was asked with; the first one's is the whole of it.
    player.current = { ...current, ...session, qualities: current.qualities?.length ? current.qualities : session.qualities, resumeMs: positionMs, quality, leftOriginal };
    return true;
  } catch (cause) {
    toast(cause instanceof Error ? cause.message : String(cause), "error");
    return false;
  }
}

/** Join (or host) a watch room's stream; guests follow the host's clock. */
export async function playRoom(room: any, isHost: boolean): Promise<void> {
  if (player.preparing) return;
  player.preparing = room.id;
  try {
    const session = await startSession(room.mediaFileId, null, room.positionMs ?? 0);
    const work = room.work;
    const detail = work ? await request(new WorkDetailStore(), { id: work.id }, { policy: CachePolicy.CacheOrNetwork }).catch(() => null) : null;
    player.current = {
      ...session,
      workId: work?.id ?? null,
      title: work?.title ?? "Watch room",
      subtitle: subtitleFor(work),
      work,
      resumeMs: room.positionMs ?? 0,
      edition: detail?.editions?.[0],
      mediaFileId: room.mediaFileId,
      room: { id: room.id, isHost },
    };
  } catch (cause) {
    toast(cause instanceof Error ? cause.message : String(cause), "error");
  } finally {
    player.preparing = null;
  }
}
