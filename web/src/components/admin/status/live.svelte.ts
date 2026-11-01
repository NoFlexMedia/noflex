// SPDX-License-Identifier: AGPL-3.0-only
//
// One shared feed of the server's host/process sample for every
// diagnostics tab. The WebSocket subscription pushes a sample per server
// tick; when it goes quiet (socket down, proxy without upgrade) the feed
// falls back to polling the query, and the tabs say which they're on.

import { AdminStatusUpdatesStore, AdminSystemStatusStore } from "$houdini";
import type { AdminSystemStatus$result } from "$houdini";
import { request } from "$lib/graphql.js";
import { onReconnect } from "$lib/live.js";
import { poll } from "$lib/ui.svelte.js";

export type SystemStatus = AdminSystemStatus$result["systemStatus"];
export type Sample = AdminSystemStatus$result["systemStatusHistory"][number];

/** Matches the server's retained window (6 min at 2 s). */
const HISTORY = 180;
/** A pushed sample older than this means the socket has stopped delivering. */
const STALE_MS = 6_500;
const POLL_MS = 5_000;

class LiveStatus {
  status = $state.raw<SystemStatus | null>(null);
  history = $state.raw<Sample[]>([]);
  /** `connecting` until the first sample; `live` while pushes arrive; `polling` on fallback. */
  mode = $state<"connecting" | "live" | "polling" | "offline">("connecting");
  error = $state("");
  updatedAt = $state(0);

  apply(status: SystemStatus, via: "live" | "polling") {
    const previous = this.status;
    this.status = status;
    this.updatedAt = Date.now();
    this.mode = via;
    this.error = "";
    if (previous && previous.sampledAt === status.sampledAt) return;
    const last = this.history.at(-1);
    if (last && Date.parse(last.at) >= Date.parse(status.sampledAt)) return;
    const sample: Sample = {
      at: status.sampledAt,
      cpuPercent: status.cpu.usagePercent,
      memoryUsedBytes: status.memory.usedBytes,
      processCpuPercent: status.process.cpuPercent,
      processRssBytes: status.process.rssBytes,
      rxBytesPerSec: status.networkTotal.rxBytesPerSec,
      txBytesPerSec: status.networkTotal.txBytesPerSec,
      loadOne: status.cpu.loadAverage?.one ?? null,
    };
    this.history = [...this.history.slice(-(HISTORY - 1)), sample];
  }
}

export const liveStatus = new LiveStatus();

const query = new AdminSystemStatusStore();
const updates = new AdminStatusUpdatesStore();
let users = 0;
let stop: (() => void) | null = null;

async function refresh(via: "live" | "polling") {
  try {
    const data = await request<AdminSystemStatus$result>(query);
    // The server's history is authoritative after a gap.
    liveStatus.history = data.systemStatusHistory.slice(-HISTORY);
    liveStatus.apply(data.systemStatus, liveStatus.mode === "live" ? "live" : via);
  } catch (cause) {
    liveStatus.error = cause instanceof Error ? cause.message : String(cause);
    if (!liveStatus.status || Date.now() - liveStatus.updatedAt > STALE_MS * 2) liveStatus.mode = "offline";
  }
}

function start(): () => void {
  void refresh("polling");
  const unsubscribe = updates.subscribe((result) => {
    const status = result.data?.systemStatusUpdates;
    if (status) liveStatus.apply(status, "live");
  });
  void updates.listen();
  const stopPolling = poll(() => {
    if (Date.now() - liveStatus.updatedAt > STALE_MS) {
      if (liveStatus.mode === "live") liveStatus.mode = "polling";
      void refresh("polling");
    }
  }, POLL_MS);
  const stopReconnect = onReconnect(() => void refresh("live"));
  return () => {
    unsubscribe();
    void updates.unlisten();
    stopPolling();
    stopReconnect();
  };
}

/**
 * Keep the feed running while the caller is mounted. Call from an
 * `$effect`; returns the disposer. The feed is shared and stops with
 * its last user.
 */
export function useLiveStatus(): () => void {
  users += 1;
  if (users === 1) stop = start();
  return () => {
    users -= 1;
    if (users === 0) {
      stop?.();
      stop = null;
      liveStatus.mode = "connecting";
    }
  };
}
