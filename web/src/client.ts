// SPDX-License-Identifier: AGPL-3.0-only
//
// The Houdini client. Every operation goes through `send`, which owns auth
// (bearer header, one refresh-and-retry on rejection) and turns transport
// failures into GraphQL-shaped errors so query stores carry them like any
// other error. Mutations throw theirs, for `attempt()` to toast.

import { fetch as fetchPlugin, HoudiniClient, subscription } from "$houdini";
import type { RequestHandler } from "$houdini/runtime/plugins/fetch";
import type { GraphQLError } from "houdini/runtime";
import { live } from "$lib/live.js";
import { activeGraphqlUrl, activeHeaders, ApiError, authHeaders, currentAccessToken, GRAPHQL_URL, hasSession, refreshAccessToken, usingLinkedServer } from "$lib/tokens.js";

const AUTH_ERROR = /authentication required|invalid token|token expired|unauthenticated/i;

interface Payload { data?: Record<string, unknown> | null; errors?: GraphQLError[] | null }

/**
 * The error a failed operation stands for: the original `Error` of a
 * transport failure (so callers can tell "unreachable" from "refused"),
 * otherwise an `ApiError` carrying the server's messages and codes.
 */
export function toError(errors: readonly GraphQLError[]): Error {
  const cause = errors.find((error) => error.extensions?.error instanceof Error)?.extensions?.error;
  if (cause) return cause;
  const codes = errors.flatMap((error) => (typeof error.extensions?.code === "string" ? [error.extensions.code] : []));
  return new ApiError(errors.map((error) => error.message).join("; ") || "The server returned an invalid response.", codes);
}

const failed = (error: unknown): Payload => {
  const cause = error instanceof Error ? error : new Error(String(error));
  return { data: null, errors: [{ message: cause.message, extensions: { error: cause } }] };
};

async function post(fetch: typeof globalThis.fetch, body: string, refreshAllowed: boolean, ownServer: boolean): Promise<Payload> {
  // A linked server's key doesn't refresh, and this page's session has no say there.
  const mayRefresh = refreshAllowed && (ownServer || !usingLinkedServer());
  const sentAccess = currentAccessToken();
  const retry = async () => {
    try { await refreshAccessToken(sentAccess); } catch (error) { return failed(error); }
    return post(fetch, body, false, ownServer);
  };
  let response: Response;
  try {
    response = await fetch(ownServer ? GRAPHQL_URL : activeGraphqlUrl(), { method: "POST", headers: { Accept: "application/json", "Content-Type": "application/json", ...(ownServer ? authHeaders() : activeHeaders()) }, body });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") throw error;
    return failed(new Error("NoFlex could not reach the server. Check your connection and try again."));
  }
  if (!response.ok) {
    if (mayRefresh && hasSession() && (response.status === 401 || response.status === 403)) return retry();
    if ([502, 503, 504].includes(response.status)) return failed(new Error("NoFlex could not reach the server. Check that it is running, then try again."));
    const message = `The server returned ${response.status}.`;
    return failed(response.status < 500 ? new ApiError(message) : new Error(message));
  }
  let payload: Payload;
  try { payload = await response.json(); } catch { return failed(new Error("The server returned an unreadable response.")); }
  if (payload.errors?.length) {
    if (mayRefresh && hasSession() && AUTH_ERROR.test(payload.errors.map((error) => error.message).join("; "))) return retry();
  } else if (!payload.data) {
    return failed(new Error("The server returned an empty response."));
  }
  return payload;
}

const send: RequestHandler = ({ fetch, name, text, variables, metadata }) =>
  post(fetch, JSON.stringify({ operationName: name, query: text, variables }), !metadata?.noRefresh, !!metadata?.ownServer) as ReturnType<RequestHandler>;

export default new HoudiniClient({
  plugins: [subscription(() => live), fetchPlugin(send)],
  throwOnError: { operations: ["mutation"], error: (errors) => toError(errors) },
});
