// SPDX-License-Identifier: AGPL-3.0-only
//
// Carries watch state between the home server and linked ones. Servers never talk
// to each other, so the client is the courier: each describes its recent activity
// by provider ID, and the other applies whatever is newer than what it has.

import { LinkedServerSyncedStore, SyncWatchStateStore, WatchSyncItemsStore } from "$houdini";
import { connectedServers, serverRequest, type LinkedServer } from "$lib/servers.svelte.js";
import { currentAccessToken } from "$lib/tokens.js";

/** How far back the first sync with a server looks. */
const FIRST_SYNC_DAYS = 90;
const SYNC_DEADLINE_MS = 30_000;

let running: Promise<void> | null = null;

const raw = (Store: new () => { artifact: { raw: string } }) => new Store().artifact.raw;

interface SyncItem { externalIds: { kind: string; value: string }[]; season: number | null; episode: number | null; played: boolean; positionMs: number | null; lastPlayed: string }

/** Houdini selects `__typename` everywhere, which an input type refuses, so an answer can't go straight back as input. */
const asInput = (items: SyncItem[]): SyncItem[] => items.map(({ externalIds, season, episode, played, positionMs, lastPlayed }) => ({
  externalIds: externalIds.map(({ kind, value }) => ({ kind, value })),
  season,
  episode,
  played,
  positionMs,
  lastPlayed,
}));

async function syncWith(server: LinkedServer, home: { origin: string; apiKey: string | null }): Promise<void> {
  const startedAt = new Date().toISOString();
  const since = server.syncedUntil ?? new Date(Date.now() - FIRST_SYNC_DAYS * 86_400_000).toISOString();
  const list = (from: typeof home) => serverRequest<{ watchSyncItems: SyncItem[] }>(from, raw(WatchSyncItemsStore), { since }, SYNC_DEADLINE_MS);
  const [ours, theirs] = await Promise.all([list(home), list(server)]);
  const apply = (to: typeof home, items: SyncItem[]) => items.length ? serverRequest(to, raw(SyncWatchStateStore), { items: asInput(items) }, SYNC_DEADLINE_MS) : Promise.resolve();
  await Promise.all([apply(server, ours.watchSyncItems), apply(home, theirs.watchSyncItems)]);
  await serverRequest(home, raw(LinkedServerSyncedStore), { id: server.id, until: startedAt }, SYNC_DEADLINE_MS);
  server.syncedUntil = startedAt;
}

/**
 * Sync with every linked server that allows it. Best effort and self-correcting:
 * a server that's away catches up from where it left off next time, and replays
 * are harmless because the older side of any pair loses.
 */
export function syncWatchState(): Promise<void> {
  running ??= (async () => {
    const token = currentAccessToken();
    if (!token) return;
    const home = { origin: "", apiKey: token };
    await Promise.all(connectedServers().filter((server) => server.syncEnabled).map((server) => syncWith(server, home).catch((error: unknown) => {
      console.warn(`Couldn't sync watch state with ${server.name}; will retry.`, error);
    })));
  })().finally(() => { running = null; });
  return running;
}
