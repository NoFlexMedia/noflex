// SPDX-License-Identifier: AGPL-3.0-only
//
// One GraphQL subscription over `graphql-transport-ws`, kept open: the web's
// `lib/live.ts` without Houdini. The token rides in `connection_init`, because
// a browser's WebSocket can't set headers. Only the hosts with a WebSocket can
// use it; NativeScript's runtime has none.

import { authHeaders, currentAccessToken, graphqlUrl, refreshAccessToken } from "$lib/tokens.js";
import type { Document, Vars } from "$lib/graphql.js";

// The server closes a refused `connection_init` with 4403 (Forbidden).
const REFUSED = 4403;
const RETRY_MIN_MS = 500;
const RETRY_MAX_MS = 30_000;

export const canSubscribe = () => typeof WebSocket !== "undefined";

function socketUrl(): string {
  const base = graphqlUrl();
  const absolute = /^https?:/.test(base) ? base : `${location.origin}${base}`;
  return `${absolute.replace(/^http/, "ws")}/ws`;
}

/** Deliver each event's data to `onData` until the returned function is called, reconnecting when the socket drops. */
export function subscribe(document: Document, variables: Vars, onData: (data: any) => void): () => void {
  let socket: WebSocket | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  let failures = 0;

  const retry = () => {
    if (stopped) return;
    const wait = Math.min(RETRY_MAX_MS, RETRY_MIN_MS * Math.pow(2, failures++));
    timer = setTimeout(open, wait * (0.75 + Math.random() * 0.5));
  };

  function open(): void {
    if (stopped) return;
    const sent = currentAccessToken();
    const current = new WebSocket(socketUrl(), "graphql-transport-ws");
    socket = current;
    current.onopen = () => current.send(JSON.stringify({ type: "connection_init", payload: authHeaders() }));
    current.onmessage = (event) => {
      let message: any;
      try { message = JSON.parse(String(event.data)); } catch { return; }
      switch (message.type) {
        case "connection_ack":
          failures = 0;
          current.send(JSON.stringify({ id: "1", type: "subscribe", payload: { query: document, variables } }));
          break;
        case "next":
          if (message.payload?.data) onData(message.payload.data);
          break;
        case "ping":
          current.send(JSON.stringify({ type: "pong" }));
          break;
        case "error":
        case "complete":
          current.close();
          break;
      }
    };
    current.onclose = (event) => {
      if (socket === current) socket = null;
      if (stopped) return;
      // Usually an expired access token: take a fresh one, then reconnect. A session that is gone ends up signed out.
      if (event.code === REFUSED) refreshAccessToken(sent).then(retry, () => {});
      else retry();
    };
  }

  open();
  return () => {
    stopped = true;
    clearTimeout(timer);
    socket?.close();
  };
}
