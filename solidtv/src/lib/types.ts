// SPDX-License-Identifier: AGPL-3.0-only
//
// What the client reads from the API. Generated types would need a codegen step
// on every target's build; `graphql.test.ts` holds the documents to the schema.

export interface Image { kind: string; url: string }
export interface UserData { played: boolean; positionMs?: number | null; favorite?: boolean | null }
export type IndexingStage = "ADDING" | "FINDING_DETAILS";

export interface Work {
  indexing?: IndexingStage | null;
  id: string;
  kind: string;
  parentId?: string | null;
  libraryId: string;
  title: string;
  year?: number | null;
  communityRating?: number | null;
  indexNumber?: number | null;
  indexEnd?: number | null;
  parentIndex?: number | null;
  runtimeMs?: number | null;
  images?: Image[] | null;
  userData?: UserData | null;
  overview?: string | null;
  tagline?: string | null;
  seriesTitle?: string | null;
  tags?: { id: string; namespace: string; display: string }[] | null;
  credits?: { personId: string; name: string; role: string; character?: string | null }[] | null;
}
export interface Edition {
  id: string;
  workId: string;
  name?: string | null;
  runtimeMs?: number | null;
  introStartMs?: number | null;
  introEndMs?: number | null;
  creditsStartMs?: number | null;
  chapters?: { positionMs: number; title?: string | null }[] | null;
  trickplay?: Trickplay | null;
}
export interface Trickplay {
  chapterCues?: { positionMs: number; title?: string | null }[] | null;
  intervalMs: number;
  thumbWidth: number;
  thumbHeight: number;
  tileColumns: number;
  tileRows: number;
  sheets: { index: number; tileCount: number; url: string }[];
}
export interface SubtitleTrack { language?: string | null; codec?: string | null; default?: boolean | null; forced?: boolean | null; title?: string | null; srtCompatible?: boolean | null }
export interface MediaFile { id: string; editionId: string; partIndex: number; path: string; sizeBytes?: number | null; container?: string | null; probe?: any; subtitleTracks?: SubtitleTrack[] | null }
export interface Library { id: string; name: string; kind: string }
export interface Viewer { id: string; name: string; username: string; avatarUrl?: string | null; isAdmin: boolean; hasPin: boolean }
/** Someone signed in on this TV, as "Who's watching?" draws them. */
export interface Profile {
  id: string;
  name: string;
  hasPin: boolean;
  avatarUrl: string | null;
  /** The server no longer knows their session here: choosing them means signing in again. */
  signedOut: boolean;
}
export interface ServerInfo { name: string; version: string; backend: string; isConfigured: boolean }
export interface Connection<T> { nodes: T[]; pageInfo?: { hasNextPage: boolean; endCursor?: string | null } }
export interface SubtitleDelivery { index: number; language?: string | null; title?: string | null; forced: boolean; default: boolean; url: string }
export interface PlaybackSession {
  sessionId: string;
  route: string;
  url: string;
  jobId?: string | null;
  startPositionMs?: number;
  subtitles?: SubtitleDelivery[];
  decision?: PlaybackDecision | null;
}
export interface StreamSummary {
  container?: string | null;
  videoCodec?: string | null;
  videoProfile?: string | null;
  videoLevel?: number | null;
  bitDepth?: number | null;
  width?: number | null;
  height?: number | null;
  videoRange?: string | null;
  audioCodec?: string | null;
  audioChannels?: number | null;
  bitrateBps?: number | null;
}
export interface PlaybackDecision {
  plan: string;
  reasons: { code: string; description: string }[];
  source: StreamSummary;
  target: StreamSummary;
  encoder?: string | null;
  audioEncoder?: string | null;
  hardware: boolean;
  tonemap: boolean;
}
export interface Device {
  id: string;
  name: string;
  smartTranscode: boolean;
  maxBitrateBps?: number | null;
}
export type SmartMarkState = "WAITING" | "PREPARING" | "READY" | "NOT_NEEDED" | "TOO_LARGE" | "NO_DEVICES" | "NO_SPACE" | "FAILED" | "DISABLED";
export interface SmartMark {
  work: Work;
  device?: { id: string; name: string } | null;
  markedAt: string;
  state: SmartMarkState;
  progress: number;
}

export interface PreparedVersion {
  id: string;
  description: string;
  sizeBytes?: number | null;
  lastUsedAt?: string | null;
  work?: { id: string; title: string } | null;
}
