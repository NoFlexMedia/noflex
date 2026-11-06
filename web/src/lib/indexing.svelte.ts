// SPDX-License-Identifier: AGPL-3.0-only
//
// Artwork showing a title that's still being indexed asks for it to be watched, and
// one request covers them all. The answer lands in the normalized cache, so whatever
// shows those works updates, and the art resolves in from the placeholder.

import { WorksByIdStore } from "$houdini";
import { request } from "$lib/graphql.js";
import { poll } from "$lib/ui.svelte.js";

const POLL_MS = 4000;
/** Whole titles, so well inside the query budget; a longer list takes turns. */
const BATCH = 30;

export const INDEXING_LABEL: Record<string, string> = { ADDING: "Adding to library", FINDING_DETAILS: "Finding details" };

const watching = new Map<string, number>();
let offset = 0;
let stop: (() => void) | null = null;

async function refresh(): Promise<void> {
  const ids = [...watching.keys()];
  if (offset >= ids.length) offset = 0;
  const turn = ids.slice(offset, offset + BATCH);
  offset += BATCH;
  if (turn.length) await request(new WorksByIdStore(), { ids: turn }).catch(() => {});
}

/** Keep `id` fresh until the returned function is called. */
export function watchIndexing(id: string): () => void {
  watching.set(id, (watching.get(id) ?? 0) + 1);
  stop ??= poll(refresh, POLL_MS);
  return () => {
    const left = (watching.get(id) ?? 1) - 1;
    if (left) watching.set(id, left);
    else watching.delete(id);
    if (!watching.size) { stop?.(); stop = null; }
  };
}
