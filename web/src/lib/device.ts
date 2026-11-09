// SPDX-License-Identifier: AGPL-3.0-only
//
// This browser as a registered device, so the server can prepare versions it
// can play. The id isn't a credential, so unlike tokens it belongs in
// localStorage: it has to outlive the tab for the device to stay the same one.

import { RegisterDeviceStore } from "$houdini";
import { deviceName } from "$lib/api.js";
import { clientCapabilities } from "$codecs";
import { request } from "$lib/graphql.js";

const DEVICE_KEY = "noflex.deviceId";

let viewer: string | null = null;

/** Whose device this browser is from now on; null once nobody is signed in. */
export function useDeviceOf(userId: string | null): void { viewer = userId; }

/**
 * This browser's device id for whoever is signed in. One each, since the server
 * keeps a device per user and refuses an id that's someone else's: two people
 * sharing a browser would otherwise leave the second with no device at all.
 * Null with nobody signed in, and where storage is unavailable, as then it
 * couldn't stay stable.
 */
export function deviceId(): string | null {
  if (!viewer) return null;
  const key = `${DEVICE_KEY}.${viewer}`;
  try {
    let id = localStorage.getItem(key);
    if (!id) { id = crypto.randomUUID(); localStorage.setItem(key, id); }
    return id;
  } catch { return null; }
}

/** Tell the server what this browser can play. Best effort: nothing depends on it right away. */
export async function registerThisDevice(): Promise<void> {
  const id = deviceId();
  if (!id) return;
  try {
    const capabilities = await clientCapabilities();
    await request(new RegisterDeviceStore(), { id, name: deviceName(), capabilities }, { metadata: { ownServer: true } });
  } catch { /* retried on the next sign-in */ }
}
