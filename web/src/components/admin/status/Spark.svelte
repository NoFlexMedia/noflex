<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  /**
   * Inline sparkline. `values` oldest first; `max` pins the top (e.g. 100
   * for percentages) — otherwise it scales to the window's peak. A second
   * series (`alt`) draws as a dashed line on the same scale.
   */
  let { values = [], alt = null, max = null, label = "", tone = "accent" } = $props();
  const id = $props.id();
  const W = 120;
  const H = 36;

  let top = $derived(max ?? Math.max(1e-9, ...values, ...(alt ?? [])) * 1.15);
  const path = (series) => {
    if (series.length < 2) return "";
    const step = W / (series.length - 1);
    return series.map((v, i) => `${i ? "L" : "M"}${(i * step).toFixed(2)},${(H - (Math.min(v, top) / top) * (H - 2) - 1).toFixed(2)}`).join("");
  };
  let line = $derived(path(values));
  let area = $derived(line ? `${line}L${W},${H}L0,${H}Z` : "");
  let second = $derived(alt ? path(alt) : "");
</script>

<svg class="spark {tone}" viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label={label}>
  <defs>
    <linearGradient id="g-{id}" x1="0" x2="0" y1="0" y2="1">
      <stop offset="0" stop-color="currentColor" stop-opacity="0.28" />
      <stop offset="1" stop-color="currentColor" stop-opacity="0" />
    </linearGradient>
  </defs>
  {#if area}<path d={area} fill="url(#g-{id})" />{/if}
  {#if line}<path d={line} fill="none" stroke="currentColor" stroke-width="1.6" vector-effect="non-scaling-stroke" stroke-linejoin="round" />{/if}
  {#if second}<path class="alt" d={second} fill="none" stroke="currentColor" stroke-width="1.3" stroke-dasharray="3 3" vector-effect="non-scaling-stroke" />{/if}
</svg>

<style>
  .spark { width: 100%; height: 36px; overflow: visible; }
  .accent { color: var(--accent); }
  .blue { color: var(--focus); }
  .green { color: var(--success); }
  .warn { color: var(--warning); }
  .alt { opacity: 0.6; color: var(--focus); }
</style>
