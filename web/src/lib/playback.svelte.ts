// SPDX-License-Identifier: AGPL-3.0-only

import { CachePolicy, ChildPageStore, MediaFilesStore, StartPlaybackStore, StopPlaybackStore, WorkDetailStore } from "$houdini";
import { clientCapabilities } from "$lib/codecSupport.js";
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
  route: string;
  url: string;
  jobId?: string | null;
  /** Source position of the stream's time zero: a transcode's start offset, 0 for direct play. */
  startPositionMs?: number;
  decision?: PlaybackDecision;
  subtitles?: SubtitleDelivery[];
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
}

export const player = $state<{ current: Playback | null; preparing: string | null }>({ current: null, preparing: null });

// Probe while the user browses so the first play doesn't wait on it.
if (typeof window !== "undefined") (window.requestIdleCallback ?? setTimeout)(() => { clientCapabilities().catch(() => {}); });

/** Open a session; a transcode starts at `startPositionMs` so far seeks don't wait on ffmpeg. */
export async function startSession(mediaFileId: string, subtitle: number | null = null, startPositionMs = 0): Promise<Session> {
  const capabilities = await clientCapabilities();
  const data = await request(new StartPlaybackStore(), { media: mediaFileId, capabilities, subtitle, start: Math.max(0, Math.round(startPositionMs)) });
  return data.startPlaybackSession as Session;
}

/** End a session server-side; best effort, a dead session is already gone. */
export function stopSession(sessionId: string | null | undefined): Promise<void> {
  if (!sessionId) return Promise.resolve();
  return request(new StopPlaybackStore(), { session: sessionId }).then(() => {}, () => {});
}

export interface PlayOptions { editionId?: string; mediaFileId?: string; subtitle?: number | null; fromStart?: boolean; positionMs?: number; room?: Playback["room"] }

/**
 * Resolve a work down to a playable media file (first edition, first part
 * unless told otherwise), open a session, and hand it to the player.
 */
export async function playWork(work: any, options: PlayOptions = {}): Promise<boolean> {
  if (player.preparing) return false;
  player.preparing = work.id;
  try {
    const detail = await request(new WorkDetailStore(), { id: work.id }, { policy: CachePolicy.CacheOrNetwork });
    const editions = detail.editions ?? [];
    const edition = editions.find((item: any) => item.id === options.editionId) ?? editions[0];
    if (!edition) throw new Error(`“${work.title}” has nothing playable attached yet.`);
    let mediaFileId = options.mediaFileId;
    if (!mediaFileId) {
      const media = await request(new MediaFilesStore(), { edition: edition.id }, { policy: CachePolicy.CacheOrNetwork });
      mediaFileId = media.mediaFiles[0]?.id;
    }
    if (!mediaFileId) throw new Error("This edition has no media files on disk.");
    const userData = detail.work?.userData;
    const resume = options.positionMs ?? (options.fromStart || userData?.played ? 0 : userData?.positionMs ?? 0);
    const session = await startSession(mediaFileId, options.subtitle ?? null, resume);
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
 * in a different subtitle, or to seek a transcode far from its offset.
 * Resolves false (after a toast) when no new session could be opened.
 */
export async function restartSession(subtitle: number | null, positionMs: number): Promise<boolean> {
  const current = player.current;
  if (!current?.mediaFileId) return false;
  try {
    await stopSession(current.sessionId);
    const session = await startSession(current.mediaFileId, subtitle, positionMs);
    player.current = { ...current, ...session, resumeMs: positionMs };
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
