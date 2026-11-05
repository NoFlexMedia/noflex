// SPDX-License-Identifier: AGPL-3.0-only
//
// This browser as a registered device, so the server can prepare versions it
// can play. The id isn't a credential, so unlike tokens it belongs in
// localStorage: it has to outlive the tab for the device to stay the same one.

import { RegisterDeviceStore } from "$houdini";
import { deviceName } from "$lib/api.js";
import { clientCapabilities } from "$lib/codecSupport.js";
import { request } from "$lib/graphql.js";

const DEVICE_KEY = "noflex.deviceId";

/** This browser's device id; null where storage is unavailable, as then it couldn't stay stable. */
export function deviceId(): string | null {
  try {
    let id = localStorage.getItem(DEVICE_KEY);
    if (!id) { id = crypto.randomUUID(); localStorage.setItem(DEVICE_KEY, id); }
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
