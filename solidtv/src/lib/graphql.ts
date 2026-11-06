// SPDX-License-Identifier: AGPL-3.0-only
//
// The GraphQL client: `request` for one operation, `createQuery` for what a page
// draws. Houdini needs Svelte; what the TV needs from it fits here: auth with one
// refresh-and-retry, transport failures told apart from refusals, and cache-then-
// network so a page paints from what it last knew.

import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import { ApiError, authHeaders, currentAccessToken, graphqlUrl, hasSession, refreshAccessToken } from "$lib/tokens.js";

const AUTH_ERROR = /authentication required|invalid token|token expired|unauthenticated/i;
const UNREACHABLE = "NoFlex could not reach the server. Check your connection and try again.";

export type Document = string;
export type Vars = Record<string, unknown>;

interface GqlError { message: string; extensions?: { code?: unknown } }
interface Payload { data?: any; errors?: GqlError[] | null }

const operationName = (document: Document) => /(?:query|mutation)\s+(\w+)/.exec(document)?.[1];

async function post(document: Document, variables: Vars, refreshAllowed: boolean): Promise<any> {
  const sentAccess = currentAccessToken();
  const retry = async () => {
    await refreshAccessToken(sentAccess);
    return post(document, variables, false);
  };
  let response: Response;
  try {
    response = await fetch(graphqlUrl(), {
      method: "POST",
      headers: { Accept: "application/json", "Content-Type": "application/json", ...authHeaders() },
      body: JSON.stringify({ operationName: operationName(document), query: document, variables }),
    });
  } catch {
    throw new Error(UNREACHABLE);
  }
  if (!response.ok) {
    if (refreshAllowed && hasSession() && (response.status === 401 || response.status === 403)) return retry();
    if ([502, 503, 504].indexOf(response.status) >= 0) throw new Error("NoFlex could not reach the server. Check that it is running, then try again.");
    const message = `The server returned ${response.status}.`;
    throw response.status < 500 ? new ApiError(message) : new Error(message);
  }
  let payload: Payload;
  try { payload = await response.json(); } catch { throw new Error("The server returned an unreadable response."); }
  if (payload.errors?.length) {
    const message = payload.errors.map((error) => error.message).join("; ");
    if (refreshAllowed && hasSession() && AUTH_ERROR.test(message)) return retry();
    throw new ApiError(message, payload.errors.flatMap((error) => (typeof error.extensions?.code === "string" ? [error.extensions.code] : [])));
  }
  if (!payload.data) throw new Error("The server returned an empty response.");
  return payload.data;
}

/** Send one operation and return its data. Errors throw: `ApiError` when refused, `Error` when unreachable. */
export function request<Data = any>(document: Document, variables: Vars = {}, options: { noRefresh?: boolean } = {}): Promise<Data> {
  return post(document, variables, !options.noRefresh);
}

// ---- the page cache -------------------------------------------------------

// A page's worth of cards per entry, on devices where memory is tight.
const CACHE_ENTRIES = 48;
const cache = new Map<string, unknown>();

const cacheKey = (document: Document, variables: Vars) => `${operationName(document)}:${JSON.stringify(variables)}`;

function remember(key: string, data: unknown): void {
  cache.delete(key);
  cache.set(key, data);
  if (cache.size > CACHE_ENTRIES) cache.delete(cache.keys().next().value as string);
}

export function resetCache(): void { cache.clear(); }
export function cached<Data = any>(document: Document, variables: Vars = {}): Data | undefined { return cache.get(cacheKey(document, variables)) as Data | undefined; }

/** Fetch into the cache ahead of need, on intent: a card holding focus, say. Failures are the page's to report. */
export function preload(document: Document, variables: Vars = {}): void {
  const key = cacheKey(document, variables);
  if (!cache.has(key)) request(document, variables).then((data) => remember(key, data), () => {});
}

export interface Query<Data> {
  data: Accessor<Data | undefined>;
  error: Accessor<string>;
  fetching: Accessor<boolean>;
  refetch: () => Promise<void>;
}

/**
 * What a page draws: paints from the cache at once, then from the network. A
 * failure keeps what's on screen and reports alongside it. `variables` returning
 * null holds the query until they're known.
 */
export function createQuery<Data = any>(document: Document, variables: () => Vars | null = () => ({})): Query<Data> {
  const [data, setData] = createSignal<Data | undefined>(undefined);
  const [error, setError] = createSignal("");
  const [fetching, setFetching] = createSignal(true);
  let run = 0;
  let disposed = false;
  onCleanup(() => { disposed = true; });

  const fetchInto = async (input: Vars) => {
    const ticket = ++run;
    const key = cacheKey(document, input);
    setFetching(true);
    try {
      const fresh = await request<Data>(document, input);
      remember(key, fresh);
      if (ticket !== run || disposed) return;
      setData(() => fresh);
      setError("");
    } catch (cause) {
      if (ticket !== run || disposed) return;
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (ticket === run && !disposed) setFetching(false);
    }
  };

  createEffect((previous: string | null | undefined) => {
    const input = variables();
    if (!input) return null;
    const key = cacheKey(document, input);
    if (key === previous) return key;
    const known = cache.get(key) as Data | undefined;
    setData(() => known);
    void fetchInto(input);
    return key;
  });

  return { data, error, fetching, refetch: async () => { const input = variables(); if (input) await fetchInto(input); } };
}

/** Compose a document with its fragments, each once however often they're reached. */
export function gql(text: string, ...fragments: string[]): Document {
  const seen = new Set<string>();
  const parts = [text];
  for (const fragment of fragments) {
    for (const piece of fragment.split(/(?=\bfragment\s+\w+\s+on\b)/)) {
      const body = piece.trim();
      if (!body || seen.has(body)) continue;
      seen.add(body);
      parts.push(body);
    }
  }
  return parts.join("\n");
}
