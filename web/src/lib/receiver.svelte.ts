// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex Connect from the receiving side, for a browser: with the viewer's say-so,
// this tab listens for commands from their other devices and tells the server what
// it's playing, which is also how it stays listed as there. Tabs share one device
// id, so one of them listens at a time (a Web Lock), and a tab that is closed lets
// the next take over.

import { DeviceCommandsStore, ReportDeviceStateStore } from "$houdini";
import { deviceId } from "$lib/device.js";
import { request } from "$lib/graphql.js";
import { playWork } from "$lib/playback.svelte.js";
import { toast } from "$lib/ui.svelte.js";
import { transport } from "$lib/transport.svelte.js";

const KEY = "noflex.receive";
const LOCK = "noflex-receiver";
// Well inside the server's 40 s of silence before a device counts as gone.
const REPORT_MS = 5000;

interface Command { kind: "LOAD" | "PLAY" | "PAUSE" | "SEEK" | "STOP"; workId?: string | null; editionId?: string | null; mediaFileId?: string | null; positionMs?: number | null }

const commands = new DeviceCommandsStore();

function stored(): boolean {
  try { return localStorage.getItem(KEY) === "true"; } catch { return false; }
}

/** Whether this browser may be played on. Off until the viewer turns it on. */
export const receiving = $state({ on: stored() });

export function setReceiving(on: boolean): void {
  receiving.on = on;
  try { localStorage.setItem(KEY, String(on)); } catch { /* applies to this tab only */ }
}

/** Say what this browser is doing now. Best effort: the next report says it again. */
function report(device: string): void {
  const playback = transport.current?.state() ?? null;
  request(new ReportDeviceStateStore(), { device, playback }).catch(() => {});
}

async function load(command: Command): Promise<void> {
  if (!command.workId) return;
  await transport.current?.finish();
  // Browsers hold back sound from a page nobody has touched; the player then waits to be started.
  const played = await playWork({ id: command.workId, title: "" }, {
    editionId: command.editionId ?? undefined,
    mediaFileId: command.mediaFileId ?? undefined,
    positionMs: command.positionMs ?? undefined,
  });
  if (!played) return;
  setTimeout(() => { if (transport.current?.state()?.state === "PAUSED") toast("Press play to start. This browser holds back sound until it has been used.", "info"); }, 1500);
}

function handle(command: Command): void {
  const current = transport.current;
  switch (command.kind) {
    case "LOAD": void load(command); break;
    case "PLAY": current?.play(); break;
    case "PAUSE": current?.pause(); break;
    case "SEEK": if (command.positionMs != null) current?.seekTo(command.positionMs / 1000); break;
    case "STOP": current?.stop(); break;
  }
}

function listen(device: string): () => void {
  const unsubscribe = commands.subscribe((result) => {
    const command = result.data?.deviceCommands;
    if (command) handle(command as Command);
  });
  void commands.listen({ device });
  report(device);
  const timer = setInterval(() => report(device), REPORT_MS);
  return () => {
    unsubscribe();
    void commands.unlisten();
    clearInterval(timer);
  };
}

/**
 * Listen while this browser is allowed to be played on and this tab holds the
 * lock. Call from an `$effect`; returns the disposer.
 */
export function useReceiver(): () => void {
  const device = deviceId();
  if (!receiving.on || !device || !navigator.locks) return () => {};
  const abort = new AbortController();
  let stop = () => {};
  let release = () => {};
  navigator.locks.request(LOCK, { signal: abort.signal }, () => new Promise<void>((resolve) => {
    release = resolve;
    stop = listen(device);
  })).catch(() => { /* withdrawn before the lock came */ });
  return () => { abort.abort(); stop(); release(); };
}

/** Report at once when the player starts, pauses or seeks, rather than at the next beat. */
export function reportSoon(): void {
  const device = deviceId();
  if (receiving.on && device) report(device);
}
