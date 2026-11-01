<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { tone as toneOf } from "./fmt.js";

  /** Horizontal utilisation bar; `pct` 0–100. Colour follows `warn`/`bad` thresholds. */
  let { pct = 0, warn = 80, bad = 92, label = "", thin = false } = $props();
  let clamped = $derived(Math.max(0, Math.min(100, pct ?? 0)));
</script>

<div class="meter {toneOf(clamped, warn, bad)}" class:thin role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(clamped)} aria-label={label}>
  <span style:width="{clamped}%"></span>
</div>

<style>
  .meter { position: relative; height: 6px; border-radius: 99px; background: rgb(255 255 255 / 0.08); overflow: hidden; }
  .meter.thin { height: 4px; }
  .meter > span { position: absolute; inset: 0 auto 0 0; border-radius: inherit; background: var(--accent); transition: width 600ms var(--ease-out), background var(--t); }
  .meter.warn > span { background: var(--warning); }
  .meter.bad > span { background: var(--danger); }
</style>
