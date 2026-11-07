// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex Connect from the receiving side: this TV listens for commands from the
// viewer's other devices and tells the server what it's playing, which is also how
// it stays listed as there. The player hands over its transport while it's open, so
// a command steers it as the remote does.

import { DEVICE_COMMANDS, REPORT_DEVICE_STATE } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import { canSubscribe, subscribe } from "$lib/live.js";
import { playWork } from "$lib/playback.js";
import { thisDeviceId } from "$lib/session.js";
import type { Work } from "$lib/types.js";

// Well inside the server's 40 s of silence before a device counts as gone.
const REPORT_MS = 5000;

type PlayState = "PLAYING" | "PAUSED" | "BUFFERING";

export interface DeviceState { workId: string; state: PlayState; positionMs: number; durationMs: number | null }

/** What the open player lets a command do. */
export interface Transport {
  play(): void;
  pause(): void;
  seekTo(seconds: number): void;
  /** Close the player, as Back does. */
  stop(): void;
  /** End the current session so another title can replace it, as moving to the next episode does. */
  finish(): Promise<void>;
  state(): DeviceState | null;
}

interface Command { kind: "LOAD" | "PLAY" | "PAUSE" | "SEEK" | "STOP"; workId: string | null; editionId: string | null; mediaFileId: string | null; positionMs: number | null }

let transport: Transport | null = null;

// A title this TV has been asked to start and hasn't got playing yet. Said as loading, or the idle
// reports between the command and the player would take back what the controllers were shown.
let loading: DeviceState | null = null;
const LOADING_MS = 30_000;

export function setTransport(next: Transport | null): void {
  transport = next;
  if (next) reportNow();
}

/** What to say now: the title being started until the player has taken it over, else what's playing. */
function current(): DeviceState | null {
  const playing = transport?.state() ?? null;
  if (loading && playing?.workId === loading.workId) loading = null;
  return loading ?? playing;
}

/** Drop `mine` if it is still the transport, and say so at once: the controllers' bar shouldn't wait for the next report to go. */
export function clearTransport(mine: Transport): void {
  if (transport !== mine) return;
  transport = null;
  reportNow();
}

/** Say what this TV is doing now, for the moments it changes. Best effort: the next report says it again. */
export function reportNow(): void {
  if (!listening) return;
  const playback = current();
  request(REPORT_DEVICE_STATE, { device: thisDeviceId(), playback }).catch(() => {});
}

async function load(command: Command): Promise<void> {
  if (!command.workId) return;
  loading = { workId: command.workId, state: "BUFFERING", positionMs: command.positionMs ?? 0, durationMs: null };
  const mine = loading;
  setTimeout(() => { if (loading === mine) loading = null; }, LOADING_MS);
  reportNow();
  await transport?.finish();
  // The title is resolved by its id alone; the player takes everything else from the server.
  const played = await playWork({ id: command.workId, title: "" } as Work, {
    editionId: command.editionId ?? undefined,
    mediaFileId: command.mediaFileId ?? undefined,
    positionMs: command.positionMs ?? undefined,
  });
  // Played, the player says so itself once it mounts; failed, nothing will.
  if (!played && loading === mine) {
    loading = null;
    reportNow();
  }
}

function handle(command: Command): void {
  switch (command.kind) {
    case "LOAD": void load(command); break;
    case "PLAY": transport?.play(); break;
    case "PAUSE": transport?.pause(); break;
    case "SEEK": if (command.positionMs != null) transport?.seekTo(command.positionMs / 1000); break;
    case "STOP": transport?.stop(); break;
  }
}

let listening = false;

/** Listen for the viewer's other devices for as long as the returned function isn't called. */
export function startRemote(): () => void {
  if (!canSubscribe()) return () => {};
  listening = true;
  const stop = subscribe(DEVICE_COMMANDS, { device: thisDeviceId() }, (data) => handle(data.deviceCommands));
  reportNow();
  const timer = setInterval(reportNow, REPORT_MS);
  return () => {
    listening = false;
    stop();
    clearInterval(timer);
  };
}
