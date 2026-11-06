// SPDX-License-Identifier: AGPL-3.0-only
//
// Titles still being indexed. Artwork asks for them to be watched and one request
// covers them all. The page cache holds whole responses, so what comes back lives
// beside it, as in `watch.ts`. All of this pauses under the player: a pulse keeps the
// renderer drawing every frame, and a hidden page's cards are still mounted.

import { Config } from "@solidtv/solid";
import { createSignal, type Signal } from "solid-js";
import { WORKS_BY_ID } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import { player } from "$lib/playback.js";
import type { IndexingStage, Work } from "$lib/types.js";

const POLL_MS = 4000;
const PULSE_MS = 1100;
/** A longer list takes turns. */
const BATCH = 30;

export const INDEXING_LABEL: Record<IndexingStage, string> = { ADDING: "Adding to library", FINDING_DETAILS: "Finding details" };

const answers = new Map<string, Signal<Work | null>>();
const watching = new Map<string, number>();
const [pulse, setPulse] = createSignal(false);
let offset = 0;
let poller: ReturnType<typeof setInterval> | undefined;
let pulser: ReturnType<typeof setInterval> | undefined;

/** For a transition's target: the two ends of the pulse. */
export const indexingPulse = pulse;

function slot(id: string): Signal<Work | null> {
  let entry = answers.get(id);
  if (!entry) {
    entry = createSignal<Work | null>(null);
    answers.set(id, entry);
  }
  return entry;
}

async function refresh(): Promise<void> {
  if (player.current) return;
  const ids = [...watching.keys()];
  if (offset >= ids.length) offset = 0;
  const turn = ids.slice(offset, offset + BATCH);
  offset += BATCH;
  if (!turn.length) return;
  const data = await request<{ worksById: Work[] }>(WORKS_BY_ID, { ids: turn }).catch(() => null);
  if (!data) return;
  for (const id of turn) {
    const work = data.worksById.find((item) => item.id === id);
    // Gone, or no longer visible to this viewer: either way nothing more is coming.
    slot(id)[1](work ?? ({ indexing: null } as Work));
  }
}

/** Keep `id` fresh until the returned function is called. */
export function watchIndexing(id: string): () => void {
  watching.set(id, (watching.get(id) ?? 0) + 1);
  if (!poller) {
    poller = setInterval(() => void refresh(), POLL_MS);
    if (Config.animationsEnabled) pulser = setInterval(() => { if (!player.current) setPulse((value) => !value); }, PULSE_MS);
  }
  return () => {
    const left = (watching.get(id) ?? 1) - 1;
    if (left) watching.set(id, left);
    else watching.delete(id);
    if (!watching.size) {
      clearInterval(poller);
      clearInterval(pulser);
      poller = pulser = undefined;
    }
  };
}

/** A work with whatever's been learned since its list was fetched; reactive. */
export function latest<T extends { id?: string; indexing?: IndexingStage | null }>(work: T): T {
  if (!work.id || !work.indexing) return work;
  const known = slot(work.id)[0]();
  return known ? { ...work, ...known } : work;
}

export function resetIndexing(): void { answers.clear(); }
