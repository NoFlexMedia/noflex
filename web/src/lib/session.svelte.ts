// SPDX-License-Identifier: AGPL-3.0-only

import { invalidateAll } from "$app/navigation";
import {
  BootstrapStore,
  cache,
  CompleteExternalLoginStore,
  CompleteIdentityLinkStore,
  LibrariesStore,
  LogoutStore,
  ServerInfoStore,
} from "$houdini";
import { deviceName, takeExternalCallback } from "$lib/api.js";
import { forgetDeviceOf, registerThisDevice, useDeviceOf } from "$lib/device.js";
import { deactivateServer, setLinkedServers, type LinkedServer } from "$lib/servers.svelte.js";
import { syncWatchState } from "$lib/sync.js";
import { request } from "$lib/graphql.js";
import { onLiveRefused } from "$lib/live.js";
import { ApiError, clearSession, hasSession, onSessionExpired, saveSession, SessionExpiredError, type SessionResult } from "$lib/tokens.js";
import { noPrefs, readPlaybackPrefs } from "$lib/tracks.js";
import { toast } from "$lib/ui.svelte.js";

/** Preference key: list libraries in the navigation alongside content types. */
export const SHOW_LIBRARIES_KEY = "browse.showLibraries";

/** Preference key: the viewer's named browse filters. */
export const SAVED_FILTERS_KEY = "browse.savedFilters";

export interface SavedFilter { name: string; rules: string }

/**
 * `resetting`: the server is erasing itself back to first-run setup. `gone`:
 * the viewer just deleted their own account.
 */
export type Phase = "boot" | "error" | "setup" | "auth" | "app" | "resetting" | "gone";

export const app = $state({
  phase: "boot" as Phase,
  server: null as any,
  viewer: null as any,
  playbackPrefs: noPrefs,
  libraries: [] as any[],
  /** `contentCategories` rows: what the viewer has anything to browse in. */
  categories: [] as { category: string; workCount: number }[],
  showLibraries: false,
  savedFilters: [] as SavedFilter[],
  error: "",
  authError: "",
});

// Mirrors the cached library list; library mutations write through the
// cache, so every view reading `app.libraries` follows along.
let unwatchLibraries = () => {};
async function watchLibraries(): Promise<void> {
  unwatchLibraries();
  const store = new LibrariesStore();
  unwatchLibraries = store.subscribe((result) => { if (result.data) app.libraries = [...result.data.libraries]; });
  await store.fetch();
}

function signedOut(authError: string): void {
  unwatchLibraries();
  cache.reset();
  app.viewer = null;
  useDeviceOf(null);
  app.libraries = [];
  app.categories = [];
  app.showLibraries = false;
  app.savedFilters = [];
  deactivateServer();
  setLinkedServers([]);
  app.authError = authError;
  app.phase = "auth";
}

onSessionExpired((error) => {
  signedOut(error.reason === "reused"
    ? `${error.message} Sign in again, and change your password if that wasn’t you.`
    : "Your session expired. Sign in again to continue.");
  void followReset();
});

onLiveRefused(() => {
  clearSession();
  signedOut("Your session expired. Sign in again to continue.");
  void followReset();
});

const RESET_POLL_MS = 2000;
let awaitingReset = false;

async function serverInfo() {
  return (await request(new ServerInfoStore(), {}, { metadata: { noRefresh: true, ownServer: true } })).serverInfo;
}

/**
 * Wait out the server erasing itself, then start over, which lands on setup.
 * It restarts on the way, so not answering is part of it.
 */
async function awaitReset(): Promise<void> {
  app.phase = "resetting";
  if (awaitingReset) return;
  awaitingReset = true;
  try {
    for (;;) {
      await new Promise((resolve) => setTimeout(resolve, RESET_POLL_MS));
      try { if (!(await serverInfo()).resetting) break; } catch { /* restarting */ }
    }
  } finally {
    awaitingReset = false;
  }
  await start();
}

/** Everyone's session ends when the server is erased: say that's why. */
async function followReset(): Promise<void> {
  try { if ((await serverInfo()).resetting) await awaitReset(); } catch { /* the sign-in screen stands */ }
}

/**
 * The viewer's account is gone: sign out and forget what this browser kept for
 * them. When it was the last administrator's, the server is erasing every
 * account it held, this browser's other ones included.
 */
export function accountDeleted(serverErased: boolean): void {
  forgetDeviceOf(serverErased ? "everyone" : app.viewer.id);
  clearSession();
  signedOut("");
  if (serverErased) void awaitReset();
  else app.phase = "gone";
}

// A page opened directly on a linked server's title or library has already
// activated it by now, so everything about the session says which server it means.
export async function bootstrap(): Promise<void> {
  try {
    const data = await request(new BootstrapStore(), {}, { metadata: { ownServer: true } });
    app.viewer = data.viewer;
    useDeviceOf(data.viewer.id);
    app.server = data.serverInfo;
    app.categories = [...data.contentCategories];
    setLinkedServers(data.linkedServers as LinkedServer[]);
    app.showLibraries = data.preferences.some((item: { key: string; value: unknown }) => item.key === SHOW_LIBRARIES_KEY && item.value === true);
    const saved = data.preferences.find((item: { key: string; value: unknown }) => item.key === SAVED_FILTERS_KEY)?.value;
    app.savedFilters = Array.isArray(saved) ? saved : [];
    app.playbackPrefs = readPlaybackPrefs(data.preferences.find((item: { key: string; value: unknown }) => item.key === "playback")?.value);
    await watchLibraries();
    app.authError = "";
    app.phase = "app";
    void registerThisDevice();
    void syncWatchState();
  } catch (cause) {
    // An unreachable server is not a sign-out; `start` shows a retry.
    if (!(cause instanceof ApiError) && !(cause instanceof SessionExpiredError)) throw cause;
    if (cause instanceof SessionExpiredError) return;
    clearSession();
    app.viewer = null;
    app.authError = cause.message;
    app.phase = "auth";
  }
}

/**
 * Adopt a freshly issued session. Route loads that ran without one left
 * their queries idle, so they run again now.
 */
export async function signIn(result: SessionResult): Promise<void> {
  saveSession(result);
  await bootstrap();
  if (app.phase === "app") await invalidateAll();
}

export async function start(): Promise<void> {
  app.phase = "boot";
  app.error = "";
  try {
    app.server = await serverInfo();
    if (app.server.resetting) { await awaitReset(); return; }
    if (!app.server.isConfigured) { app.phase = "setup"; return; }
    const callback = takeExternalCallback();
    if (callback?.kind === "login") {
      try {
        const data = await request(new CompleteExternalLoginStore(), { code: callback.code, nonce: callback.nonce, device: deviceName() }, { metadata: { noRefresh: true, ownServer: true } });
        await signIn(data.completeExternalLogin);
      } catch (cause) {
        app.authError = cause instanceof Error ? cause.message : String(cause);
        app.phase = "auth";
      }
      return;
    }
    if (!hasSession()) { app.phase = "auth"; return; }
    await bootstrap();
    if (callback?.kind === "link" && (app.phase as Phase) === "app") {
      try {
        const linked = (await request(new CompleteIdentityLinkStore(), { code: callback.code, nonce: callback.nonce }, { metadata: { ownServer: true } })).completeExternalIdentityLink;
        toast(`${linked.displayName || linked.pluginId} is now linked to your account.`, "success");
      } catch (cause) {
        toast(cause instanceof Error ? cause.message : String(cause), "error");
      }
    }
  } catch (cause) {
    app.error = cause instanceof Error ? cause.message : String(cause);
    app.phase = "error";
  }
}

export async function logout(): Promise<void> {
  try { await request(new LogoutStore(), {}, { metadata: { noRefresh: true, ownServer: true } }); } catch { /* signing out locally regardless */ }
  clearSession();
  signedOut("");
}
