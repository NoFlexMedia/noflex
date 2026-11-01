<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { liveStatus } from "./live.svelte.js";
  import { clock } from "./fmt.js";

  const LABEL = { connecting: "Connecting…", live: "Live", polling: "Polling", offline: "Offline" };
  const DETAIL = {
    connecting: "Waiting for the first sample.",
    live: "Streaming over WebSocket, one sample every 2 s.",
    polling: "The live connection is unavailable; refreshing every 5 s.",
    offline: "The server isn’t answering.",
  };
</script>

<span class="live {liveStatus.mode}" title={liveStatus.error || DETAIL[liveStatus.mode]} aria-live="polite">
  <span class="dot" class:on={liveStatus.mode === "live"} class:warn={liveStatus.mode === "polling"} class:bad={liveStatus.mode === "offline"}></span>
  {LABEL[liveStatus.mode]}
  {#if liveStatus.status}<span class="at tabular">· {clock(liveStatus.status.sampledAt)}</span>{/if}
</span>

<style>
  .live { display: inline-flex; align-items: center; gap: 8px; height: 28px; padding: 0 11px; border-radius: 999px; background: var(--surface-2); font-size: 0.78rem; font-weight: 600; color: var(--text-2); white-space: nowrap; }
  .live.live .dot { animation: breathe 2s var(--ease-in-out) infinite; }
  @keyframes breathe { 50% { box-shadow: 0 0 0 6px rgb(110 231 168 / 0.06); } }
  .at { color: var(--text-3); font-weight: 500; }
</style>
