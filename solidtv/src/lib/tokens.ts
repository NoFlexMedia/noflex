// SPDX-License-Identifier: AGPL-3.0-only
//
// The server this TV talks to and the sessions it holds, with the single-flight
// refresh ported from web/src/lib/tokens.ts.
//
// Unlike the web's tab-scoped session these are stored durably: a TV is an
// appliance, and nobody wants to type a password on a remote each evening. And
// there's one per person who has signed in here, since a TV is shared: requests go
// out as whichever is active.

import { host } from "$lib/host.js";

export interface SessionResult { token?: string; accessToken?: string; refreshToken: string }

/** Someone signed in on this TV, and the session that's theirs. */
export interface Account {
  /** The user's id. */
  id: string;
  name: string;
  /** Whether switching to them asks for their PIN. */
  hasPin: boolean;
  avatarUrl: string | null;
  /** This TV as registered to them: the server keeps a device per user, and refuses an id that's someone else's. */
  deviceId: string;
  accessToken: string;
  refreshToken: string;
}

/** The server answered and refused the request (GraphQL errors or a 4xx), as opposed to being unreachable. */
export class ApiError extends Error {
  constructor(message: string, readonly codes: readonly string[] = [], readonly extensions: Record<string, unknown> = {}) { super(message); this.name = "ApiError"; }
}

/**
 * Why the active session stopped answering. `reused` means the server saw a
 * replayed refresh token and revoked the session; `locked` that it's waiting for
 * its user's PIN, and is still theirs once they give it.
 */
export class SessionExpiredError extends Error {
  constructor(readonly reason: "expired" | "reused" | "locked") {
    super(reason === "reused" ? "You were signed out because this session was used from somewhere else." : reason === "locked" ? "This profile is locked." : "Your session has expired.");
    this.name = "SessionExpiredError";
  }
}

const SERVER_KEY = "noflex.server";
const ACCOUNTS_KEY = "noflex.accounts";
const ACTIVE_KEY = "noflex.active";
const REFRESH_MUTATION = `mutation Refresh($token: String!) { refreshSession(refreshToken: $token) { token refreshToken } }`;

let origin: string | null = null;
let accounts: Account[] = [];
let active: Account | null = null;
let refreshPromise: Promise<void> | null = null;
let expiredHandler: (error: SessionExpiredError, account: Account) => void = () => {};

function readAccounts(): Account[] {
  try {
    const stored: unknown = JSON.parse(host.storage.get(ACCOUNTS_KEY) ?? "[]");
    if (!Array.isArray(stored)) return [];
    return stored.filter((item): item is Account => !!item && typeof item.id === "string" && typeof item.refreshToken === "string" && typeof item.deviceId === "string");
  } catch {
    return [];
  }
}

function persist(): void {
  host.storage.set(ACCOUNTS_KEY, JSON.stringify(accounts));
  if (active) host.storage.set(ACTIVE_KEY, active.id); else host.storage.remove(ACTIVE_KEY);
}

/** Not a credential, only stable: crypto.randomUUID is too new for older TVs. */
function newDeviceId(): string {
  return "xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx".replace(/x/g, () => Math.floor(Math.random() * 16).toString(16));
}

/** Read what's stored. The host's storage has to exist first, so this isn't done at import. */
export function restore(): void {
  origin = host.storage.get(SERVER_KEY);
  accounts = readAccounts();
  const id = host.storage.get(ACTIVE_KEY);
  active = accounts.find((account) => account.id === id) ?? null;
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

export function onSessionExpired(handler: (error: SessionExpiredError, account: Account) => void): void { expiredHandler = handler; }
export function hasSession(): boolean { return active !== null; }
export function currentAccessToken(): string | null { return active?.accessToken ?? null; }
export function authHeaders(): Record<string, string> { return active ? { Authorization: `Bearer ${active.accessToken}` } : {}; }

/** Everyone signed in here, in the order they did. */
export function listAccounts(): readonly Account[] { return accounts; }
export function activeAccount(): Account | null { return active; }

/** Send requests as `id` from now on, or as nobody. */
export function activate(id: string | null): void {
  active = accounts.find((account) => account.id === id) ?? null;
  persist();
}

function tokensOf(result: SessionResult): { accessToken: string; refreshToken: string } {
  const accessToken = result.token ?? result.accessToken;
  if (!accessToken || !result.refreshToken) throw new Error("The server returned an incomplete session.");
  return { accessToken, refreshToken: result.refreshToken };
}

/**
 * Keep a session just issued for `user` and make it the active one. Someone
 * already here keeps their place and their device; their old session is replaced.
 */
export function adoptSession(result: SessionResult, user: { id: string; name: string; hasPin: boolean }): void {
  const tokens = tokensOf(result);
  const known = accounts.find((account) => account.id === user.id);
  const account: Account = { avatarUrl: null, deviceId: newDeviceId(), ...known, ...user, ...tokens };
  accounts = known ? accounts.map((item) => (item === known ? account : item)) : [...accounts, account];
  active = account;
  persist();
}

/** Change what's kept about `id`: new tokens for their session, or what their tile shows. */
export function updateAccount(id: string, change: Partial<Omit<Account, "id" | "deviceId">>): void {
  const known = accounts.find((account) => account.id === id);
  if (!known) return;
  const account = { ...known, ...change };
  accounts = accounts.map((item) => (item === known ? account : item));
  if (active === known) active = account;
  persist();
}

export function removeAccount(id: string): void {
  accounts = accounts.filter((account) => account.id !== id);
  if (active?.id === id) active = null;
  persist();
}

/** Forget everyone, as when the TV is pointed at another server. */
export function clearAccounts(): void {
  accounts = [];
  active = null;
  persist();
}

function expire(error: SessionExpiredError, account: Account): never {
  expiredHandler(error, account);
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

/** Resolves once no refresh is under way, so a refresh token read afterwards is the current one. */
export function refreshSettled(): Promise<void> {
  return refreshPromise ? refreshPromise.then(() => {}, () => {}) : Promise.resolve();
}

/**
 * Rotate the active session's token pair, then let the caller retry with
 * `authHeaders()`. Single-flight: a second refresh with the same token looks like
 * replay to the server. Pass the access token a rejected request carried to skip
 * refreshing when another caller already replaced it. A refusal fires the
 * expired handler once, which decides what becomes of the account; an
 * unreachable server changes nothing.
 */
export function refreshAccessToken(staleAccess?: string | null): Promise<void> {
  if (staleAccess !== undefined && staleAccess !== active?.accessToken && active) return Promise.resolve();
  refreshPromise ??= (async () => {
    const account = active;
    if (!account) throw new SessionExpiredError("expired");
    try {
      const tokens = tokensOf(await redeem(account.refreshToken));
      updateAccount(account.id, tokens);
    } catch (error) {
      if (!(error instanceof ApiError)) throw error;
      // A sign-in or a switch that finished mid-refresh owns the session now.
      const current = accounts.find((item) => item.id === account.id);
      if (active?.id !== account.id || current?.refreshToken !== account.refreshToken) return;
      const reason = error.codes.includes("REFRESH_TOKEN_REUSED") ? "reused" : error.codes.includes("SESSION_LOCKED") ? "locked" : "expired";
      return expire(new SessionExpiredError(reason), account);
    }
  })().then((value) => { refreshPromise = null; return value; }, (error) => { refreshPromise = null; throw error; });
  return refreshPromise;
}
