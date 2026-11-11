// SPDX-License-Identifier: AGPL-3.0-only
//
// Opening and closing sessions, ported from web/src/lib/playback.svelte.ts.

import { CHILD_PAGE, MEDIA_FILES, START_LIVE, START_PLAYBACK, STOP_PLAYBACK, WORK_DETAIL } from "$lib/graphql/documents.js";
import { cached, request } from "$lib/graphql.js";
import type { ClientCapabilities } from "$codecs";
import { decodeChoice, encodeChoice, sessionQuality, type QualityChoice } from "$quality";
import { host } from "$lib/host.js";
import { app } from "$lib/session.js";
import { changeSetting, QUALITY_HEIGHTS, settings } from "$lib/settings.js";
import { reactive } from "$lib/state.js";
import { mediaUrl } from "$lib/tokens.js";
import { isImageSubtitle, preferredAudio, preferredSubtitle } from "$lib/tracks.js";
import type { Connection, Edition, MediaFile, PlaybackSession, SubtitleTrack, Work } from "$lib/types.js";
import { toast } from "$lib/ui.js";
import { userDataOf } from "$lib/watch.js";

export interface Playback extends PlaybackSession {
  workId: string | null;
  title: string;
  subtitle?: string;
  work?: Work;
  /** Where to begin, in source time. */
  resumeMs?: number;
  edition?: Edition;
  mediaFileId?: string;
  live?: boolean;
  audioStream?: number | null;
  audioStreams?: any[];
  subtitleTracks?: SubtitleTrack[];
  /** The subtitle track to start with, as text or burned in. */
  subtitleIndex?: number | null;
  burnedSubtitle?: number | null;
  quality?: QualityChoice;
  /** Auto moved this title off the original because it struggled. */
  leftOriginal?: boolean;
}

export const player = reactive({ current: null as Playback | null, preparing: null as string | null });

/** The last stream's bandwidth estimate, so the next one starts at a quality it can hold. */
export const measured: { bandwidthBps: number | null } = { bandwidthBps: null };

/** The quality this TV plays at, Original until the viewer picks another. */
export const qualityChoice = (): QualityChoice => decodeChoice(settings.quality);
export const rememberQuality = (choice: QualityChoice) => changeSetting("quality", encodeChoice(choice));

/** What a session asks the server for; see `sessionQuality`. */
export type QualityRequest = ReturnType<typeof sessionQuality>;

export const isContainer = (work: Pick<Work, "kind"> | null | undefined) => ["SERIES", "SEASON", "ARTIST", "ALBUM"].indexOf(work?.kind ?? "") >= 0;
export const isPlayable = (work: Pick<Work, "kind"> | null | undefined) => !!work && !isContainer(work) && work.kind !== "PHOTO";
export const isAudioOnly = (work: Pick<Work, "kind"> | null | undefined) => ["TRACK", "AUDIO_BOOK"].indexOf(work?.kind ?? "") >= 0;

function absolute(session: PlaybackSession): PlaybackSession {
  return { ...session, url: mediaUrl(session.url), subtitles: session.subtitles?.map((track) => ({ ...track, url: mediaUrl(track.url) })) };
}

/** What this TV can play, less what the viewer has asked it not to. */
function limited(capabilities: ClientCapabilities): ClientCapabilities {
  const ceiling = QUALITY_HEIGHTS[app.playbackPrefs.quality];
  return {
    ...capabilities,
    maxHeight: ceiling && capabilities.maxHeight ? Math.min(ceiling, capabilities.maxHeight) : ceiling ?? capabilities.maxHeight,
    videoCodecs: settings.allowHdr ? capabilities.videoCodecs : capabilities.videoCodecs.map((codec) => ({ ...codec, videoRanges: [] })),
  };
}

/**
 * Open a session. A linear transcode (copied video) starts at `startPositionMs`, which
 * comes back as its offset; a video encode covers the whole title and comes back at 0.
 */
export async function startSession(mediaFileId: string, subtitle: number | null = null, startPositionMs = 0, audioStream: number | null = null, quality: QualityRequest = sessionQuality(qualityChoice(), null)): Promise<PlaybackSession> {
  const allowed = limited(await host.capabilities());
  const capabilities = quality.maxBitrateBps ? { ...allowed, maxBitrateBps: quality.maxBitrateBps } : allowed;
  const data = await request(START_PLAYBACK, { media: mediaFileId, capabilities, subtitle, start: Math.max(0, Math.round(startPositionMs)), audio: audioStream, adaptive: quality.adaptive });
  return absolute(data.startPlaybackSession);
}

export function stopSession(sessionId: string | null | undefined): Promise<void> {
  if (!sessionId) return Promise.resolve();
  return request(STOP_PLAYBACK, { session: sessionId }).then(() => {}, () => {});
}

export interface PlayOptions { editionId?: string; mediaFileId?: string; audioStream?: number | null; subtitleIndex?: number | null; fromStart?: boolean; positionMs?: number }

async function loadOrCached<Data>(document: string, variables: Record<string, unknown>): Promise<Data> {
  return cached<Data>(document, variables) ?? request<Data>(document, variables);
}

/** Resolve a work down to a media file (first edition, first part unless told otherwise), open a session, hand it to the player. */
export async function playWork(work: Work, options: PlayOptions = {}): Promise<boolean> {
  if (player.preparing) return false;
  player.preparing = work.id;
  try {
    const detail = await loadOrCached<{ work: Work; editions: Edition[] }>(WORK_DETAIL, { id: work.id });
    const editions = detail.editions ?? [];
    const edition = editions.find((item) => item.id === options.editionId) ?? editions[0];
    if (!edition) throw new Error(`“${work.title}” has nothing playable attached yet.`);
    const media = await loadOrCached<{ mediaFiles: MediaFile[] }>(MEDIA_FILES, { edition: edition.id });
    const file = media.mediaFiles.find((item) => item.id === options.mediaFileId) ?? media.mediaFiles[0];
    if (!file) throw new Error("This edition has no media files on disk.");
    const audioStream = options.audioStream !== undefined ? options.audioStream : preferredAudio(file.probe?.audio, app.playbackPrefs);
    const subtitleIndex = options.subtitleIndex !== undefined ? options.subtitleIndex : preferredSubtitle(file.subtitleTracks, app.playbackPrefs);
    const burnedSubtitle = subtitleIndex != null && isImageSubtitle(file.subtitleTracks?.[subtitleIndex]) ? subtitleIndex : null;
    const known = userDataOf(detail.work ?? work);
    const resume = options.positionMs ?? (options.fromStart || known?.played ? 0 : known?.positionMs ?? 0);
    const quality = qualityChoice();
    const session = await startSession(file.id, burnedSubtitle, resume, audioStream, sessionQuality(quality, null));
    player.current = {
      ...session,
      workId: work.id,
      title: detail.work?.title ?? work.title,
      subtitle: subtitleFor(detail.work ?? work),
      work: detail.work ?? work,
      resumeMs: resume,
      edition,
      mediaFileId: file.id,
      audioStream,
      audioStreams: file.probe?.audio ?? [],
      subtitleTracks: file.subtitleTracks ?? [],
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

export async function playChannel(channel: { id: string; name: string }): Promise<void> {
  if (player.preparing) return;
  player.preparing = channel.id;
  try {
    const data = await request(START_LIVE, { id: channel.id });
    player.current = { ...absolute(data.startLiveTvChannel), workId: null, title: channel.name, live: true };
  } catch (cause) {
    toast(cause instanceof Error ? cause.message : String(cause), "error");
  } finally {
    player.preparing = null;
  }
}

export interface SessionChange { audioStream?: number | null; quality?: QualityChoice; leftOriginal?: boolean }

/**
 * Replace the current session with one starting at `positionMs`: to burn in a subtitle,
 * change the audio of a transcode, seek a linear one far from its offset, or play at
 * another quality.
 */
export async function restartSession(subtitle: number | null, positionMs: number, change: SessionChange = {}): Promise<boolean> {
  const current = player.current;
  if (!current?.mediaFileId) return false;
  const quality = change.quality ?? current.quality ?? qualityChoice();
  // A new choice starts over on the original, if that's where it starts.
  const leftOriginal = change.leftOriginal ?? (change.quality ? false : current.leftOriginal ?? false);
  try {
    await stopSession(current.sessionId);
    const audio = change.audioStream !== undefined ? change.audioStream : current.audioStream ?? null;
    const session = await startSession(current.mediaFileId, subtitle, positionMs, audio, sessionQuality(quality, current.decision?.source ?? null, leftOriginal));
    // The ladder a session offers depends on the limit it was asked with; the first one's is the whole of it.
    const qualities = current.qualities?.length ? current.qualities : session.qualities;
    player.current = { ...current, ...session, qualities, resumeMs: positionMs, audioStream: audio, burnedSubtitle: subtitle, quality, leftOriginal };
    return true;
  } catch (cause) {
    toast(cause instanceof Error ? cause.message : String(cause), "error");
    return false;
  }
}

export function closePlayer(): void { player.current = null; }

export function subtitleFor(work: Work | null | undefined): string {
  if (!work) return "";
  if (work.kind === "EPISODE" && work.indexNumber != null) {
    const episode = work.indexEnd != null ? `E${work.indexNumber}–${work.indexEnd}` : `E${work.indexNumber}`;
    return work.parentIndex != null ? `S${work.parentIndex} · ${episode}` : episode;
  }
  return work.year ? String(work.year) : "";
}

export const byIndex = (a: Work, b: Work) => (a.parentIndex ?? 0) - (b.parentIndex ?? 0) || (a.indexNumber ?? 1e9) - (b.indexNumber ?? 1e9) || String(a.title).localeCompare(String(b.title));

/** Every direct child of `parentId`, in index order. Pages through because the complexity budget caps a page at 60 cards. */
export async function fetchChildren(libraryId: string, parentId: string): Promise<Work[]> {
  const items: Work[] = [];
  let cursor: string | null = null;
  do {
    const data: { works: Connection<Work> } = await request(CHILD_PAGE, { library: libraryId, parent: parentId, after: cursor });
    items.push(...data.works.nodes);
    cursor = data.works.pageInfo?.hasNextPage ? data.works.pageInfo.endCursor ?? null : null;
  } while (cursor);
  return items.sort(byIndex);
}

/** The episode or track after `work` within its parent, if any. */
export async function nextSibling(work: Work): Promise<Work | null> {
  if (!work.parentId || ["EPISODE", "TRACK"].indexOf(work.kind) < 0) return null;
  try {
    const siblings = await fetchChildren(work.libraryId, work.parentId);
    const index = siblings.findIndex((item) => item.id === work.id);
    return index >= 0 ? siblings[index + 1] ?? null : null;
  } catch {
    return null;
  }
}
