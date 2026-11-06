// SPDX-License-Identifier: AGPL-3.0-only
//
// The server this TV talks to and its session, with the single-flight refresh
// ported from web/src/lib/tokens.ts.
//
// Unlike the web's tab-scoped session this one is stored durably: a TV is an
// appliance, and nobody wants to type a password on a remote each evening.

import { host } from "$lib/host.js";

export interface SessionResult { token?: string; accessToken?: string; refreshToken: string }

/** The server answered and refused the request (GraphQL errors or a 4xx), as opposed to being unreachable. */
export class ApiError extends Error {
  constructor(message: string, readonly codes: readonly string[] = []) { super(message); this.name = "ApiError"; }
}

/** Why the session ended; `reused` means the server saw a replayed refresh token and revoked the session. */
export class SessionExpiredError extends Error {
  constructor(readonly reason: "expired" | "reused") {
    super(reason === "reused" ? "You were signed out because this session was used from somewhere else." : "Your session has expired.");
    this.name = "SessionExpiredError";
  }
}

const SERVER_KEY = "noflex.server";
const ACCESS_KEY = "noflex.accessToken";
const REFRESH_KEY = "noflex.refreshToken";
const REFRESH_MUTATION = `mutation Refresh($token: String!) { refreshSession(refreshToken: $token) { token refreshToken } }`;

let origin: string | null = null;
let accessToken: string | null = null;
let refreshToken: string | null = null;
let refreshPromise: Promise<void> | null = null;
let expiredHandler: (error: SessionExpiredError) => void = () => {};

/** Read what's stored. The host's storage has to exist first, so this isn't done at import. */
export function restore(): void {
  origin = host.storage.get(SERVER_KEY);
  accessToken = host.storage.get(ACCESS_KEY);
  refreshToken = host.storage.get(REFRESH_KEY);
}

/** "tv.lan" → "http://tv.lan:8096": people type what they'd say out loud. */
export function normalizeOrigin(input: string): string {
  const text = input.trim().replace(/\/+$/, "");
  const withScheme = /^https?:\/\//i.test(text) ? text : `http://${text}`;
  const hasPort = /^https?:\/\/(\[[^\]]+\]|[^/:]+):\d+/i.test(withScheme);
  return hasPort || /^https:/i.test(withScheme) ? withScheme : `${withScheme}:8096`;
}

export function serverOrigin(): string | null { return origin; }
export function useServer(next: string | null): void {
  origin = next;
  if (next) host.storage.set(SERVER_KEY, next); else host.storage.remove(SERVER_KEY);
}

export function graphqlUrl(): string { return `${origin ?? ""}/graphql`; }
/** A URL from an API response as this client must request it: responses use paths relative to the server. */
export function mediaUrl<Url extends string | null | undefined>(url: Url): Url | string { return url?.charAt(0) === "/" ? `${origin ?? ""}${url}` : url; }

export function onSessionExpired(handler: (error: SessionExpiredError) => void): void { expiredHandler = handler; }
export function hasSession(): boolean { return Boolean(refreshToken); }
export function currentAccessToken(): string | null { return accessToken; }
export function authHeaders(): Record<string, string> { return accessToken ? { Authorization: `Bearer ${accessToken}` } : {}; }

export function saveSession(result: SessionResult): void {
  const nextAccess = result.token ?? result.accessToken;
  if (!nextAccess || !result.refreshToken) throw new Error("The server returned an incomplete session.");
  accessToken = nextAccess;
  refreshToken = result.refreshToken;
  host.storage.set(ACCESS_KEY, accessToken);
  host.storage.set(REFRESH_KEY, refreshToken);
}

export function clearSession(): void {
  accessToken = null;
  refreshToken = null;
  host.storage.remove(ACCESS_KEY);
  host.storage.remove(REFRESH_KEY);
}

function expire(error: SessionExpiredError): never {
  clearSession();
  expiredHandler(error);
  throw error;
}

async function redeem(token: string): Promise<SessionResult> {
  let response: Response;
  try {
    response = await fetch(graphqlUrl(), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ query: REFRESH_MUTATION, variables: { token } }),
    });
  } catch {
    throw new Error("NoFlex could not reach the server. Check your connection and try again.");
  }
  if (response.status >= 400 && response.status < 500) throw new ApiError(`The server returned ${response.status}.`);
  if (!response.ok) throw new Error(`The server returned ${response.status}.`);
  let payload: { data?: { refreshSession?: SessionResult }; errors?: { message: string; extensions?: { code?: unknown } }[] };
  try { payload = await response.json(); }
  catch { throw new Error("The server returned an unreadable response."); }
  if (payload.errors?.length) {
    const codes = payload.errors.flatMap((error) => (typeof error.extensions?.code === "string" ? [error.extensions.code] : []));
    throw new ApiError(payload.errors.map((error) => error.message).join("; "), codes);
  }
  if (!payload.data?.refreshSession) throw new Error("The server returned an empty response.");
  return payload.data.refreshSession;
}

/**
 * Rotate the token pair, then let the caller retry with `authHeaders()`.
 * Single-flight: a second refresh with the same token looks like replay to the
 * server. Pass the access token a rejected request carried to skip refreshing
 * when another caller already replaced it. A refusal clears the session and
 * fires the expired handler once; an unreachable server keeps it.
 */
export function refreshAccessToken(staleAccess?: string | null): Promise<void> {
  if (staleAccess !== undefined && staleAccess !== accessToken && accessToken) return Promise.resolve();
  refreshPromise ??= (async () => {
    const token = refreshToken;
    if (!token) return expire(new SessionExpiredError("expired"));
    try {
      saveSession(await redeem(token));
    } catch (error) {
      if (!(error instanceof ApiError)) throw error;
      // A sign-in that finished mid-refresh owns the session now.
      if (refreshToken !== token) return;
      return expire(new SessionExpiredError(error.codes.includes("REFRESH_TOKEN_REUSED") ? "reused" : "expired"));
    }
  })().then((value) => { refreshPromise = null; return value; }, (error) => { refreshPromise = null; throw error; });
  return refreshPromise;
}
