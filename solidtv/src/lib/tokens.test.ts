// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { configureHost } from "$lib/host.js";
import { activeAccount, adoptSession, listAccounts, onSessionExpired, refreshAccessToken, restore, SessionExpiredError, useServer } from "$lib/tokens.js";

const vera = { id: "vera", name: "Vera", hasPin: true };
const kit = { id: "kit", name: "Kit", hasPin: false };
const session = (name: string) => ({ token: `${name}-access`, refreshToken: `${name}-refresh` });

let stored: Record<string, string>;

beforeEach(() => {
  stored = {};
  configureHost({ storage: { get: (key) => stored[key] ?? null, set: (key, value) => { stored[key] = value; }, remove: (key) => { delete stored[key]; } } });
  restore();
  useServer("http://tv.test:28793");
});

afterEach(() => { vi.unstubAllGlobals(); });

function refuseRefreshWith(code: string): void {
  vi.stubGlobal("fetch", async () => ({ ok: true, status: 200, json: async () => ({ errors: [{ message: "invalid refresh token", extensions: { code } }] }) }));
}

describe("accounts on a shared TV", () => {
  it("keeps someone's place and device when they sign in again", () => {
    adoptSession(session("vera-1"), vera);
    adoptSession(session("kit"), kit);
    const device = listAccounts()[0]!.deviceId;
    expect(listAccounts()[1]!.deviceId).not.toBe(device);

    adoptSession(session("vera-2"), { ...vera, name: "Vera V" });
    expect(listAccounts().map((account) => account.name)).toEqual(["Vera V", "Kit"]);
    expect(listAccounts()[0]).toMatchObject({ deviceId: device, refreshToken: "vera-2-refresh" });
    expect(activeAccount()?.id).toBe("vera");
  });

  it("comes back from storage, and from nothing when storage is unreadable", () => {
    adoptSession(session("vera"), vera);
    adoptSession(session("kit"), kit);
    restore();
    expect(listAccounts().map((account) => account.id)).toEqual(["vera", "kit"]);
    expect(activeAccount()?.id).toBe("kit");

    stored["noflex.accounts"] = "{not json";
    restore();
    expect(listAccounts()).toEqual([]);
    expect(activeAccount()).toBeNull();
  });

  it("tells a locked session from one that has ended, and forgets neither on its own", async () => {
    adoptSession(session("vera"), vera);
    const reasons: string[] = [];
    onSessionExpired((error) => { reasons.push(error.reason); });

    refuseRefreshWith("SESSION_LOCKED");
    await expect(refreshAccessToken()).rejects.toBeInstanceOf(SessionExpiredError);
    refuseRefreshWith("REFRESH_TOKEN_INVALID");
    await expect(refreshAccessToken()).rejects.toBeInstanceOf(SessionExpiredError);

    expect(reasons).toEqual(["locked", "expired"]);
    expect(listAccounts().map((account) => account.id)).toEqual(["vera"]);
  });
});
