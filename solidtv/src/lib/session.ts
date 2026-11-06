// SPDX-License-Identifier: AGPL-3.0-only

import { availableCategories } from "$lib/categories.js";
import { BOOTSTRAP, LOGIN, LOGOUT, REGISTER_DEVICE, SET_PREFERENCE, SERVER_INFO } from "$lib/graphql/documents.js";
import { request, resetCache } from "$lib/graphql.js";
import { host } from "$lib/host.js";
import { reactive } from "$lib/state.js";
import { ApiError, clearSession, hasSession, normalizeOrigin, onSessionExpired, restore, saveSession, serverOrigin, SessionExpiredError, useServer } from "$lib/tokens.js";
import { noPrefs, readPlaybackPrefs } from "$lib/tracks.js";
import type { Library, ServerInfo, Viewer } from "$lib/types.js";
import { resetWatch } from "$lib/watch.js";

/** Preference key: list libraries in the navigation alongside content types. */
const SHOW_LIBRARIES_KEY = "browse.showLibraries";
const DEVICE_KEY = "noflex.deviceId";

export type Phase = "boot" | "connect" | "error" | "setup" | "auth" | "app";

export const app = reactive({
  phase: "boot" as Phase,
  server: null as ServerInfo | null,
  viewer: null as Viewer | null,
  playbackPrefs: noPrefs,
  libraries: [] as Library[],
  categories: [] as { category: string; workCount: number }[],
  showLibraries: false,
  error: "",
  authError: "",
});

export const destinations = () => availableCategories(app.categories);

function signedOut(authError: string): void {
  resetCache();
  resetWatch();
  app.viewer = null;
  app.libraries = [];
  app.categories = [];
  app.showLibraries = false;
  app.authError = authError;
  app.phase = "auth";
}

onSessionExpired((error) => {
  signedOut(error.reason === "reused"
    ? `${error.message} Sign in again, and change your password if that wasn’t you.`
    : "Your session expired. Sign in again to continue.");
});

function deviceId(): string {
  let id = host.storage.get(DEVICE_KEY);
  if (!id) {
    // Not a credential, only stable: crypto.randomUUID is too new for older TVs.
    id = "xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx".replace(/x/g, () => Math.floor(Math.random() * 16).toString(16));
    host.storage.set(DEVICE_KEY, id);
  }
  return id;
}

/** Tell the server what this TV can play, so it can prepare versions. Best effort. */
async function registerThisDevice(): Promise<void> {
  try {
    await request(REGISTER_DEVICE, { id: deviceId(), name: host.deviceName(), capabilities: await host.capabilities() });
  } catch { /* retried on the next launch */ }
}

// Whole, so fields another client wrote survive being saved from here.
let storedPlayback: any = null;

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

export const thisDeviceId = deviceId;

async function bootstrap(): Promise<void> {
  try {
    const data = await request(BOOTSTRAP);
    app.viewer = data.viewer;
    app.server = data.serverInfo;
    app.libraries = data.libraries;
    app.categories = data.contentCategories;
    app.showLibraries = data.preferences.some((item: { key: string; value: unknown }) => item.key === SHOW_LIBRARIES_KEY && item.value === true);
    storedPlayback = data.preferences.find((item: { key: string; value: unknown }) => item.key === "playback")?.value ?? null;
    app.playbackPrefs = readPlaybackPrefs(storedPlayback);
    app.authError = "";
    app.phase = "app";
    void registerThisDevice();
  } catch (cause) {
    // An unreachable server is not a sign-out; `start` shows a retry.
    if (cause instanceof SessionExpiredError) return;
    if (!(cause instanceof ApiError)) throw cause;
    clearSession();
    app.viewer = null;
    app.authError = cause.message;
    app.phase = "auth";
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
    if (!hasSession()) { app.phase = "auth"; return; }
    await bootstrap();
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
  clearSession();
  resetCache();
  app.authError = "";
  app.phase = app.server?.isConfigured ? "auth" : "setup";
}

export function changeServer(): void {
  clearSession();
  useServer(null);
  signedOut("");
  app.phase = "connect";
}

export async function signIn(username: string, password: string): Promise<void> {
  const data = await request(LOGIN, { username, password, client: host.clientName, device: host.deviceName() }, { noRefresh: true });
  saveSession(data.login);
  await bootstrap();
}

export async function logout(): Promise<void> {
  try { await request(LOGOUT, {}, { noRefresh: true }); } catch { /* signing out locally regardless */ }
  clearSession();
  signedOut("");
}
