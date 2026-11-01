// SPDX-License-Identifier: AGPL-3.0-only
//
// Subscriptions over one shared `graphql-transport-ws` socket, adapted to
// Houdini's subscription plugin. Browsers can't put headers on an upgrade,
// so the bearer token rides in `connection_init`.

import { createClient } from "graphql-ws";
import type { SubscriptionClient } from "$houdini/runtime/plugins/subscription";
import { authHeaders, currentAccessToken, refreshAccessToken } from "$lib/tokens.js";

// The server closes a refused `connection_init` with 4403 (Forbidden).
const REFUSED = new Set([4403]);

const reconnectListeners = new Set<() => void>();
let refusedHandler: () => void = () => {};
let everConnected = false;
let initAccess: string | null = null;
let refreshing: Promise<void> | null = null;
// Whether a token refresh was already spent on a refusal since the last ack.
let refreshed = false;

const isRefusal = (error: unknown) => typeof error === "object" && error !== null && "code" in error && REFUSED.has((error as { code: number }).code);

const socket = createClient({
  url: () => `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/graphql/ws`,
  connectionParams: () => {
    initAccess = currentAccessToken();
    return authHeaders();
  },
  lazy: true,
  retryAttempts: Infinity,
  // Drops (the server's periodic re-validation, a restart, a flaky network)
  // reconnect; a refusal needs a fresh token first, handled per subscription.
  shouldRetry: (event) => typeof event === "object" && event !== null && "code" in event && !isRefusal(event),
  retryWait: (retries) => new Promise((resolve) => setTimeout(resolve, Math.min(30_000, 500 * 2 ** retries) * (0.75 + Math.random() * 0.5))),
  on: {
    connected: () => {
      refreshed = false;
      if (everConnected) for (const listener of reconnectListeners) listener();
      everConnected = true;
    },
  },
});

/** Called when the socket is still refused after a token refresh. */
export function onLiveRefused(handler: () => void): void { refusedHandler = handler; }

/**
 * Run `listener` after every reconnect: events that fired while the socket
 * was down are lost, so subscribers refetch what they mirror. Returns the
 * disposer.
 */
export function onReconnect(listener: () => void): () => void {
  reconnectListeners.add(listener);
  return () => { reconnectListeners.delete(listener); };
}

export const live: SubscriptionClient = {
  subscribe(payload, handlers) {
    let active = true;
    let dispose = () => {};
    const start = () => {
      dispose = socket.subscribe(payload as { query: string }, {
        next: (value) => handlers.next(value as Parameters<typeof handlers.next>[0]),
        complete: handlers.complete,
        error: (error) => {
          if (!active) return;
          if (!isRefusal(error)) { handlers.error(error as {}); return; }
          // Usually an expired access token: refresh once and resubscribe.
          // Refused again with a fresh token means the session itself is gone.
          if (!refreshing && refreshed) {
            refusedHandler();
            handlers.error(new Error("Live updates need you to sign in again."));
            return;
          }
          refreshed = true;
          refreshing ??= refreshAccessToken(initAccess).finally(() => { refreshing = null; });
          refreshing.then(() => { if (active) start(); }, (cause) => { if (active) handlers.error(cause); });
        },
      });
    };
    start();
    return () => {
      active = false;
      dispose();
    };
  },
};
