// SPDX-License-Identifier: AGPL-3.0-only
//
// The app's lifecycle. A TV is shared, so it keeps a session for everyone who has
// signed in on it and asks who's watching: `tokens.ts` holds the sessions, and this
// decides whose is in use and what the screen is showing because of it.

import { availableCategories } from "$lib/categories.js";
import { BOOTSTRAP, LOCK_SESSION, LOGIN, LOGOUT, CREATE_DEVICE_CODE, POLL_DEVICE_CODE, REGISTER_DEVICE, REMEMBERED_PROFILES, SET_PREFERENCE, SERVER_INFO, UNLOCK_SESSION } from "$lib/graphql/documents.js";
import { request, resetCache } from "$lib/graphql.js";
import { host } from "$lib/host.js";
import { reactive } from "$lib/state.js";
import { activate, activeAccount, adoptSession, ApiError, clearAccounts, listAccounts, normalizeOrigin, onSessionExpired, refreshSettled, removeAccount, restore, serverOrigin, SessionExpiredError, updateAccount, useServer, type Account, type SessionResult } from "$lib/tokens.js";
import { noPrefs, readPlaybackPrefs } from "$lib/tracks.js";
import type { Library, Profile, ServerInfo, Viewer } from "$lib/types.js";
import { resetIndexing } from "$lib/indexing.js";
import { resetWatch } from "$lib/watch.js";

/** Preference key: list libraries in the navigation alongside content types. */
const SHOW_LIBRARIES_KEY = "browse.showLibraries";

export type Phase = "boot" | "connect" | "error" | "setup" | "profiles" | "auth" | "app";

export const app = reactive({
  phase: "boot" as Phase,
  server: null as ServerInfo | null,
  viewer: null as Viewer | null,
  playbackPrefs: noPrefs,
  libraries: [] as Library[],
  categories: [] as { category: string; workCount: number }[],
  showLibraries: false,
  /** Everyone signed in on this TV, for "Who's watching?". */
  profiles: [] as Profile[],
  /** Whose tile led to the sign-in screen, when it's there to bring them back. */
  rejoining: null as Profile | null,
  error: "",
  authError: "",
});

export const destinations = () => availableCategories(app.categories);

interface SignedIn extends SessionResult { user: { id: string; name: string; hasPin: boolean } }

/** A PIN the server wouldn't take. `final` means there's no trying again: the profile is back among the tiles, or gone. */
export class PinRefused extends Error {
  constructor(message: string, readonly final: boolean) { super(message); this.name = "PinRefused"; }
}

// Accounts whose session the server no longer knows. They stay as tiles, so the
// household's row doesn't reshuffle and signing in again is one step away.
let signedOut: string[] = [];
const markSignedOut = (id: string) => { if (signedOut.indexOf(id) < 0) signedOut = [...signedOut, id]; };
const markSignedIn = (id: string) => { signedOut = signedOut.filter((other) => other !== id); };

function syncProfiles(): void {
  app.profiles = listAccounts().map((account) => ({ id: account.id, name: account.name, hasPin: account.hasPin, avatarUrl: account.avatarUrl, signedOut: signedOut.indexOf(account.id) >= 0 }));
}

// Whole, so fields another client wrote survive being saved from here.
let storedPlayback: any = null;

/** Nothing of the last viewer's stays for the next. */
function dropViewerState(): void {
  resetCache();
  resetWatch();
  resetIndexing();
  storedPlayback = null;
  app.viewer = null;
  app.playbackPrefs = noPrefs;
  app.libraries = [];
  app.categories = [];
  app.showLibraries = false;
}

/** With nobody active: on to who's watching, or to signing in when nobody's here to choose. */
function settle(message: string): void {
  dropViewerState();
  syncProfiles();
  app.authError = message;
  const profiles = app.profiles;
  // One person, who has to sign in again: their tile would only be a step in the way.
  app.rejoining = profiles.length === 1 && profiles[0]!.signedOut ? profiles[0]! : null;
  app.phase = profiles.length && !app.rejoining ? "profiles" : "auth";
  if (app.phase === "profiles") void refreshProfiles();
}

onSessionExpired((error, account) => {
  activate(null);
  if (error.reason === "locked") { settle(""); return; }
  markSignedOut(account.id);
  settle(error.reason === "reused"
    ? `${account.name} was signed out because their session was used from somewhere else. Sign in again, and change the password if that wasn’t them.`
    : `${account.name}’s session expired. Sign in again to continue.`);
});

/** This TV as the server knows it for whoever is watching: a device belongs to one user, so each has their own. */
export function thisDeviceId(): string { return activeAccount()?.deviceId ?? ""; }

/** Tell the server what this TV can play, so it can prepare versions. Best effort. */
async function registerThisDevice(): Promise<void> {
  const id = thisDeviceId();
  if (!id) return;
  try {
    await request(REGISTER_DEVICE, { id, name: host.deviceName(), client: host.clientName, capabilities: await host.capabilities() });
  } catch { /* retried on the next launch */ }
}

/** Change the viewer's playback preference, which follows them to every client. `change` receives what's stored. */
export async function savePlaybackPrefs(change: (stored: any) => any): Promise<void> {
  const base = storedPlayback && typeof storedPlayback === "object" && !Array.isArray(storedPlayback) ? storedPlayback : {};
  const value = change(base);
  await request(SET_PREFERENCE, { key: "playback", value });
  storedPlayback = value;
  app.playbackPrefs = readPlaybackPrefs(value);
}

export async function setShowLibraries(show: boolean): Promise<void> {
  await request(SET_PREFERENCE, { key: SHOW_LIBRARIES_KEY, value: show });
  app.showLibraries = show;
}

async function bootstrap(): Promise<void> {
  const account = activeAccount();
  try {
    const data = await request(BOOTSTRAP);
    // Someone else was chosen while this was out.
    if (activeAccount()?.id !== account?.id) return;
    app.viewer = data.viewer;
    app.server = data.serverInfo;
    app.libraries = data.libraries;
    app.categories = data.contentCategories;
    app.showLibraries = data.preferences.some((item: { key: string; value: unknown }) => item.key === SHOW_LIBRARIES_KEY && item.value === true);
    storedPlayback = data.preferences.find((item: { key: string; value: unknown }) => item.key === "playback")?.value ?? null;
    app.playbackPrefs = readPlaybackPrefs(storedPlayback);
    updateAccount(data.viewer.id, { name: data.viewer.name, hasPin: data.viewer.hasPin });
    syncProfiles();
    app.authError = "";
    app.rejoining = null;
    app.phase = "app";
    void registerThisDevice();
  } catch (cause) {
    // The expiry handler has already moved on. An unreachable server is not a sign-out; the caller says so.
    if (cause instanceof SessionExpiredError) return;
    activate(null);
    if (!(cause instanceof ApiError)) throw cause;
    if (account) markSignedOut(account.id);
    settle(cause.message);
  }
}

export async function start(): Promise<void> {
  app.phase = "boot";
  app.error = "";
  restore();
  const preset = import.meta.env.VITE_NOFLEX_SERVER as string | undefined;
  if (!serverOrigin() && preset) useServer(preset);
  if (!serverOrigin()) { app.phase = "connect"; return; }
  try {
    app.server = (await request(SERVER_INFO, {}, { noRefresh: true })).serverInfo;
    if (!app.server?.isConfigured) { app.phase = "setup"; return; }
    const last = activeAccount();
    // One person, still in their profile, goes straight back to it. Anyone else
    // starts from who's watching: the TV is shared, or its one viewer stepped
    // away on purpose. Whoever was last watching is locked like anyone who has.
    if (last && listAccounts().length === 1) {
      try {
        await bootstrap();
      } catch (cause) {
        // Out of reach isn't stepping away: the retry goes straight in too.
        activate(last.id);
        throw cause;
      }
      return;
    }
    activate(null);
    if (last) void lock(last);
    settle("");
  } catch (cause) {
    app.error = cause instanceof Error ? cause.message : String(cause);
    app.phase = "error";
  }
}

/** Point at a server, throwing what to tell the viewer if nothing there answers. */
export async function connect(address: string): Promise<void> {
  const origin = normalizeOrigin(address);
  const previous = serverOrigin();
  useServer(origin);
  try {
    app.server = (await request(SERVER_INFO, {}, { noRefresh: true })).serverInfo;
  } catch {
    useServer(previous);
    throw new Error(`Nothing answered at ${origin}. Check the address, and that the server is running.`);
  }
  // Sessions belong to the server that issued them.
  if (origin !== previous) clearAccounts();
  activate(null);
  if (app.server?.isConfigured) settle("");
  else app.phase = "setup";
}

/** Forget this server, and with it everyone signed in to it here. */
export function changeServer(): void {
  clearAccounts();
  signedOut = [];
  useServer(null);
  dropViewerState();
  syncProfiles();
  app.authError = "";
  app.rejoining = null;
  app.phase = "connect";
}

// ---- who's watching ---------------------------------------------------------

/** Leave a session waiting for its PIN. Best effort: the PIN is asked for here either way, and this is what makes the server ask too. */
async function lock(account: Account): Promise<void> {
  if (!account.hasPin) return;
  try { await request(LOCK_SESSION, { token: account.refreshToken }, { noRefresh: true }); } catch { /* tried again the next time they step away */ }
}

/**
 * Bring the tiles up to date with the server: names, pictures and PINs change
 * elsewhere, and a session revoked from another device needs signing in again.
 * Best effort, since the tiles already show what was last known.
 */
export async function refreshProfiles(): Promise<void> {
  const asked = listAccounts().slice();
  if (!asked.length) return;
  let answers: ({ userId: string; name: string; hasPin: boolean; avatarUrl: string | null } | null)[];
  try {
    answers = (await request(REMEMBERED_PROFILES, { tokens: asked.map((account) => account.refreshToken) }, { noRefresh: true })).rememberedProfiles;
  } catch {
    return;
  }
  asked.forEach((account, index) => {
    // Answered for a token that has been rotated or replaced since.
    const now = listAccounts().find((other) => other.id === account.id);
    if (!now || now.refreshToken !== account.refreshToken) return;
    const answer = answers[index];
    if (!answer || answer.userId !== account.id) { markSignedOut(account.id); return; }
    markSignedIn(account.id);
    updateAccount(account.id, { name: answer.name, hasPin: answer.hasPin, avatarUrl: answer.avatarUrl ?? null });
  });
  syncProfiles();
}

/** Step away from the active profile to "Who's watching?", locking it behind its PIN if it has one. */
export async function switchProfile(): Promise<void> {
  // A refresh under way would leave the lock holding a token that's about to be replaced.
  await refreshSettled();
  const account = activeAccount();
  activate(null);
  settle("");
  if (account) void lock(account);
}

/**
 * Start watching as `id`, with their PIN if they have one.
 *
 * Throws `PinRefused` for a PIN the server won't take, and an `Error` when the
 * server can't be reached. A session that has ended lands back on the tiles
 * without throwing, with the reason in `app.authError`.
 */
export async function chooseProfile(id: string, pin?: string): Promise<void> {
  const account = listAccounts().find((other) => other.id === id);
  if (!account) return;
  if (account.hasPin) {
    let opened: SignedIn;
    try {
      opened = (await request(UNLOCK_SESSION, { token: account.refreshToken, pin: pin ?? "" }, { noRefresh: true })).unlockSession;
    } catch (cause) {
      throw pinRefusal(cause, account);
    }
    adoptSession(opened, opened.user);
  } else {
    activate(id);
  }
  await bootstrap();
}

function pinRefusal(cause: unknown, account: Account): unknown {
  if (!(cause instanceof ApiError)) return cause;
  if (cause.codes.indexOf("PIN_INCORRECT") >= 0) {
    const left = Number(cause.extensions.attemptsLeft) || 0;
    return new PinRefused(`That PIN isn’t right. ${left} ${left === 1 ? "try" : "tries"} left.`, false);
  }
  // Either way the session is over, and like any that ends its tile stays.
  markSignedOut(account.id);
  settle(cause.codes.indexOf("PIN_ATTEMPTS_EXHAUSTED") >= 0
    ? "Too many wrong PINs. Sign in again to use this profile."
    : `${account.name} was signed out here. Sign in again to continue.`);
  return new PinRefused(app.authError, true);
}

/** Open the sign-in screen for someone new. */
export function addProfile(): void {
  app.rejoining = null;
  app.authError = "";
  app.phase = "auth";
}

/** Open the sign-in screen to bring back someone whose session here has ended. */
export function rejoin(profile: Profile): void {
  app.rejoining = profile;
  app.authError = "";
  app.phase = "auth";
}

/** Back from the sign-in screen to the tiles, when there are any. */
export function leaveSignIn(): boolean {
  if (!app.profiles.length) return false;
  app.rejoining = null;
  app.authError = "";
  app.phase = "profiles";
  return true;
}

/** Take someone off this TV whose session has already ended, so there's nothing to tell the server. */
export function removeProfile(id: string): void {
  removeAccount(id);
  markSignedIn(id);
  settle("");
}

// ---- signing in and out -----------------------------------------------------

async function signedIn(result: SignedIn): Promise<void> {
  adoptSession(result, result.user);
  markSignedIn(result.user.id);
  await bootstrap();
}

export async function signIn(username: string, password: string): Promise<void> {
  const data = await request(LOGIN, { username, password, client: host.clientName, device: host.deviceName() }, { noRefresh: true });
  await signedIn(data.login);
}

/** A code to approve from a device that's already signed in, so nobody types a password on a remote. */
export async function requestDeviceCode(): Promise<{ code: string; expiresAt: string }> {
  const data = await request(CREATE_DEVICE_CODE, { client: host.clientName, device: host.deviceName() }, { noRefresh: true });
  return data.createDeviceCode;
}

/** Sign in if the code has been approved. Null until then, and for a code that has expired. */
export async function checkDeviceCode(code: string): Promise<boolean> {
  const asked = serverOrigin();
  const data = await request(POLL_DEVICE_CODE, { code }, { noRefresh: true });
  // The viewer may have moved on to another server while this was out.
  if (!data.pollDeviceCode || serverOrigin() !== asked) return false;
  await signedIn(data.pollDeviceCode);
  return true;
}

/** Sign the active profile out of this TV. Anyone else here stays. */
export async function logout(): Promise<void> {
  try { await request(LOGOUT, {}, { noRefresh: true }); } catch { /* signing out locally regardless */ }
  const account = activeAccount();
  if (account) removeAccount(account.id);
  settle("");
}
