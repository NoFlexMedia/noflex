// SPDX-License-Identifier: AGPL-3.0-only
//
// The other servers this user has linked. Each keeps its own data: merged views
// ask them all and combine the answers here in the client, and a title from a
// linked server opens with that server active, so the ordinary pages and player
// work on it unchanged.

import { LinkedServersStore } from "$houdini";
import { request } from "$lib/graphql.js";
import { useServer } from "$lib/tokens.js";

export interface LinkedServer {
  id: string;
  origin: string;
  name: string;
  /** Empty until the user has signed in to it. */
  apiKey: string;
  syncEnabled: boolean;
  syncedUntil: string | null;
}

/** How long a merged view waits on one server: a slow one must never hold up the page. */
const DEADLINE_MS = 5000;

export interface ServerLibrary { id: string; name: string; kind: string }

/** A connected server's libraries as last asked. `failed` means it didn't answer the last time. */
export interface ServerLibraries { items: ServerLibrary[]; failed: boolean }

export const servers = $state({
  linked: [] as LinkedServer[],
  active: null as LinkedServer | null,
  loaded: false,
  libraries: {} as Record<string, ServerLibraries>,
});

export const connectedServers = (): LinkedServer[] => servers.linked.filter((server) => server.apiKey);

export function setLinkedServers(list: readonly LinkedServer[]): void {
  servers.linked = [...list];
  servers.loaded = true;
  servers.libraries = Object.fromEntries(Object.entries(servers.libraries).filter(([id]) => list.some((server) => server.id === id)));
}

/** Send requests and resolve media URLs against `id`'s server. False when there's no such connected server. */
export function activateServer(id: string): boolean {
  const server = connectedServers().find((item) => item.id === id);
  if (!server) return false;
  servers.active = server;
  useServer({ origin: server.origin, apiKey: server.apiKey });
  return true;
}

export function deactivateServer(): void {
  servers.active = null;
  useServer(null);
}

/** For a page opened directly by URL, which can run before the session has loaded the list. */
export async function ensureLinkedServers(): Promise<void> {
  if (servers.loaded) return;
  const data = await request(new LinkedServersStore(), {}, { metadata: { ownServer: true } });
  if (!servers.loaded) setLinkedServers(data.linkedServers);
}

/** One GraphQL operation against any server. Rejects on failure or after `timeoutMs`. */
export async function serverRequest<Data = any>(server: { origin: string; apiKey?: string | null }, query: string, variables: Record<string, unknown> = {}, timeoutMs = DEADLINE_MS): Promise<Data> {
  const response = await fetch(`${server.origin}/graphql`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...(server.apiKey ? { Authorization: `Bearer ${server.apiKey}` } : {}) },
    body: JSON.stringify({ query, variables }),
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) throw new Error(`The server returned ${response.status}.`);
  const payload = await response.json();
  if (payload.errors?.length) throw new Error(payload.errors.map((error: { message: string }) => error.message).join("; "));
  return payload.data as Data;
}

/** Refresh each connected server's library list. A server that fails keeps what it last said, marked `failed`. */
export async function loadServerLibraries(asked: LinkedServer[] = connectedServers()): Promise<void> {
  await Promise.all(asked.map(async (server) => {
    try {
      const data = await serverRequest<{ libraries: ServerLibrary[] }>(server, "query { libraries { id name kind } }");
      servers.libraries[server.id] = { items: data.libraries, failed: false };
    } catch {
      servers.libraries[server.id] = { items: servers.libraries[server.id]?.items ?? [], failed: true };
    }
  }));
}

/** `works` as this client shows them: tagged with their server, and with image URLs that resolve from here. */
export function tagged(works: any[], server: LinkedServer): any[] {
  return works.map((work) => ({
    ...work,
    server: { id: server.id, name: server.name },
    images: work.images?.map((image: any) => ({ ...image, url: image.url?.startsWith("/") ? `${server.origin}${image.url}` : image.url })),
  }));
}

export interface ServerAnswer<Lists> { server: LinkedServer; lists: Lists }

/**
 * Ask every connected server the same question, concurrently, and pull named
 * lists of works from each answer. Servers that fail or run out of time are left
 * out, and listed in `missing`.
 */
export async function askServersForLists<Name extends string>(store: { artifact: { raw: string } }, variables: Record<string, unknown>, pick: (data: any) => Record<Name, any[]>): Promise<{ answers: ServerAnswer<Record<Name, any[]>>[]; missing: string[] }> {
  const asked = connectedServers();
  const settled = await Promise.allSettled(asked.map((server) => serverRequest(server, store.artifact.raw, variables)));
  const answers: ServerAnswer<Record<Name, any[]>>[] = [];
  const missing: string[] = [];
  settled.forEach((result, index) => {
    const server = asked[index]!;
    if (result.status !== "fulfilled") { missing.push(server.name); return; }
    const lists = Object.fromEntries(Object.entries<any[]>(pick(result.value)).map(([name, works]) => [name, tagged(works, server)])) as Record<Name, any[]>;
    answers.push({ server, lists });
  });
  return { answers, missing };
}

/** {@link askServersForLists} for the common case of one list per server. */
export async function askServers(store: { artifact: { raw: string } }, variables: Record<string, unknown>, pick: (data: any) => any[]): Promise<{ answers: { server: LinkedServer; works: any[] }[]; missing: string[] }> {
  const { answers, missing } = await askServersForLists(store, variables, (data) => ({ works: pick(data) }));
  return { answers: answers.map(({ server, lists }) => ({ server, works: lists.works })), missing };
}
