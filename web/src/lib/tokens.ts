// SPDX-License-Identifier: AGPL-3.0-only
//
// Token storage and single-flight refresh, free of any framework import so
// every transport (the fetch client, the WebSocket client, a future
// GraphQL-client plugin) shares one copy of the session.

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

export const GRAPHQL_URL = "/graphql";
const REFRESH_MUTATION = `mutation Refresh($token: String!) { refreshSession(refreshToken: $token) { token refreshToken } }`;
const ACCESS_KEY = "noflex.accessToken";
const REFRESH_KEY = "noflex.refreshToken";
const TAB_KEY = "noflex.tabId";
// Long enough for a reloading tab's previous document to release its lock.
const TAB_CLAIM_WAIT_MS = 3000;

const browser = typeof window !== "undefined" && typeof sessionStorage !== "undefined";

// Tokens live in sessionStorage by design: they die with the tab and are
// never readable from another origin's storage partition.
let accessToken: string | null = browser ? sessionStorage.getItem(ACCESS_KEY) : null;
let refreshToken: string | null = browser ? sessionStorage.getItem(REFRESH_KEY) : null;
let refreshPromise: Promise<void> | null = null;
let expiredHandler: (error: SessionExpiredError) => void = () => {};

// "Duplicate tab" clones sessionStorage, so two tabs would spend one refresh
// token and the server's reuse detection would revoke the session for both.
// Each tab holds a lock named after its id while it lives; a tab that cannot
// take the lock for an inherited id is the copy and drops the refresh token.
const tabClaim: Promise<void> = browser ? claimTab() : Promise.resolve();

function holdTabLock(id: string, waitMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    navigator.locks
      .request(`noflex.tab:${id}`, { signal: AbortSignal.timeout(waitMs) }, () => { resolve(true); return new Promise<never>(() => {}); })
      .catch((error: unknown) => resolve(!(error instanceof DOMException && (error.name === "TimeoutError" || error.name === "AbortError"))));
  });
}

async function claimTab(): Promise<void> {
  if (!("locks" in navigator)) return;
  const inherited = sessionStorage.getItem(TAB_KEY);
  if (inherited && await holdTabLock(inherited, TAB_CLAIM_WAIT_MS)) return;
  if (inherited) {
    refreshToken = null;
    sessionStorage.removeItem(REFRESH_KEY);
  }
  const id = crypto.randomUUID();
  sessionStorage.setItem(TAB_KEY, id);
  await holdTabLock(id, TAB_CLAIM_WAIT_MS);
}

/** Register the one handler told when the server ends the session. */
export function onSessionExpired(handler: (error: SessionExpiredError) => void): void { expiredHandler = handler; }
export function hasSession(): boolean { return Boolean(refreshToken); }
/** The access token requests are currently sent with; pass it back to `refreshAccessToken` after a rejection. */
export function currentAccessToken(): string | null { return accessToken; }

/** The server requests currently go to, when it isn't this page's own. */
let activeServer: { origin: string; apiKey: string } | null = null;
/** Direct application requests and media URLs at a linked server, or back home with `null`. */
export function useServer(server: { origin: string; apiKey: string } | null): void { activeServer = server; }
export function usingLinkedServer(): boolean { return activeServer !== null; }
export function activeGraphqlUrl(): string { return activeServer ? `${activeServer.origin}${GRAPHQL_URL}` : GRAPHQL_URL; }
/** Credentials for the active server. `authHeaders` always means this page's own. */
export function activeHeaders(): Record<string, string> { return activeServer ? { Authorization: `Bearer ${activeServer.apiKey}` } : authHeaders(); }
/** A URL from an API response as this page must request it: responses use paths relative to the server that sent them. */
export function mediaUrl<Url extends string | null | undefined>(url: Url): Url | string { return activeServer && url?.startsWith("/") ? `${activeServer.origin}${url}` : url; }

export function authHeaders(): Record<string, string> { return accessToken ? { Authorization: `Bearer ${accessToken}` } : {}; }

export function saveSession(result: SessionResult): void {
  const nextAccess = result.token ?? result.accessToken;
  if (!nextAccess || !result.refreshToken) throw new Error("The server returned an incomplete session.");
  accessToken = nextAccess;
  refreshToken = result.refreshToken;
  if (browser) {
    sessionStorage.setItem(ACCESS_KEY, accessToken);
    sessionStorage.setItem(REFRESH_KEY, refreshToken);
  }
}

export function clearSession(): void {
  accessToken = null;
  refreshToken = null;
  if (!browser) return;
  sessionStorage.removeItem(ACCESS_KEY);
  sessionStorage.removeItem(REFRESH_KEY);
}

function expire(error: SessionExpiredError): never {
  clearSession();
  expiredHandler(error);
  throw error;
}

async function redeem(token: string): Promise<SessionResult> {
  let response: Response;
  try {
    response = await fetch(GRAPHQL_URL, {
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
    const codes = payload.errors.flatMap((error) => typeof error.extensions?.code === "string" ? [error.extensions.code] : []);
    throw new ApiError(payload.errors.map((error) => error.message).join("; "), codes);
  }
  if (!payload.data?.refreshSession) throw new Error("The server returned an empty response.");
  return payload.data.refreshSession;
}

/**
 * Rotate the token pair, then let the caller retry with `authHeaders()`.
 * Single-flight: every caller during one refresh shares it, because a
 * second refresh with the same token looks like replay to the server.
 * Pass the access token a rejected request carried (`currentAccessToken()`
 * at send time) to skip refreshing when another caller already replaced it.
 * A server refusal, or no session at all, clears the session, fires the
 * expired handler once, and rejects with `SessionExpiredError`; an
 * unreachable server rejects with a plain `Error` and keeps the session.
 */
export function refreshAccessToken(staleAccess?: string | null): Promise<void> {
  if (staleAccess !== undefined && staleAccess !== accessToken && accessToken) return Promise.resolve();
  refreshPromise ??= (async () => {
    await tabClaim;
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
  })().finally(() => { refreshPromise = null; });
  return refreshPromise;
}
