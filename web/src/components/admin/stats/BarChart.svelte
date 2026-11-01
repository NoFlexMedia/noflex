<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { bucketLabel, compact, hours } from "./fmt.js";

  /**
   * Single-series column chart over time buckets. `points`: [{ start,
   * plays, watchedMs }] oldest first; `metric` picks the measure. One
   * series, so no legend: the panel title names it.
   */
  let { points = [], metric = "plays", bucket = "DAY", label = "" } = $props();

  const value = (point) => (metric === "plays" ? point.plays : point.watchedMs / 3_600_000);
  const show = (v) => (metric === "plays" ? `${compact(v)}` : `${v < 10 && v > 0 ? v.toFixed(1) : compact(Math.round(v))}`);

  // Ticks on clean steps so the axis reads at a glance.
  let ticks = $derived.by(() => {
    const peak = Math.max(0, ...points.map(value));
    if (peak === 0) return [0, 1, 2, 3, 4].map((n) => n * (metric === "plays" ? 1 : 0.25));
    const raw = peak / 4;
    const magnitude = 10 ** Math.floor(Math.log10(raw));
    const step = [1, 2, 2.5, 5, 10].map((m) => m * magnitude).find((s) => s >= raw) ?? raw;
    const clean = metric === "plays" ? Math.max(1, Math.ceil(step)) : step;
    return [0, 1, 2, 3, 4].map((n) => n * clean);
  });
  let top = $derived(ticks.at(-1) || 1);
  // Thin x labels so they never collide: at most ~8 across.
  let every = $derived(Math.max(1, Math.ceil(points.length / 8)));
  let hovered = $state(null);
  let unit = $derived(metric === "plays" ? "plays" : "hours");
</script>

<figure class="chart" aria-label={label}>
  <div class="plot">
    <div class="grid" aria-hidden="true">
      {#each [...ticks].reverse() as tick}<div class="tick"><span>{show(tick)}</span></div>{/each}
    </div>
    <div class="cols" style:--n={points.length} role="list">
      {#each points as point, index (point.start)}
        {@const v = value(point)}
        <div
          class="col"
          class:on={hovered === index}
          style:--h="{(v / top) * 100}%"
          role="listitem"
          aria-label="{bucketLabel(point.start, bucket, true)}: {point.plays} plays, {hours(point.watchedMs)}"
          onpointerenter={() => (hovered = index)}
          onpointerleave={() => (hovered = null)}
        >
          <span class="bar" class:zero={v === 0} style:height="{(v / top) * 100}%" style:--d="{Math.min(index, 40) * 12}ms"></span>
          {#if hovered === index}
            <span class="tip" class:left={index > points.length * 0.7} class:right={index < points.length * 0.3}>
              <strong>{bucketLabel(point.start, bucket, true)}</strong>
              <span>{point.plays} {point.plays === 1 ? "play" : "plays"} · {hours(point.watchedMs)}</span>
            </span>
          {/if}
        </div>
      {/each}
    </div>
  </div>
  <div class="axis" style:--n={points.length} aria-hidden="true">
    {#each points as point, index (point.start)}
      <span>{index % every === 0 ? bucketLabel(point.start, bucket) : ""}</span>
    {/each}
  </div>
  <table class="sr-only">
    <caption>{label} ({unit})</caption>
    <thead><tr><th>Period</th><th>Plays</th><th>Watched</th></tr></thead>
    <tbody>{#each points as point}<tr><td>{bucketLabel(point.start, bucket, true)}</td><td>{point.plays}</td><td>{hours(point.watchedMs)}</td></tr>{/each}</tbody>
  </table>
</figure>

<style>
  .chart { display: grid; gap: 8px; --axis-w: 40px; }
  .plot { position: relative; height: 220px; }
  .grid { position: absolute; inset: 0; display: flex; flex-direction: column; justify-content: space-between; pointer-events: none; }
  .tick { position: relative; height: 0; border-top: 1px solid var(--line); margin-left: var(--axis-w); }
  .tick:last-child { border-top-color: var(--line-strong); }
  .tick span { position: absolute; right: calc(100% + 10px); top: -0.6em; font-size: 0.72rem; color: var(--text-3); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .cols { position: absolute; inset: 0 0 0 var(--axis-w); display: grid; grid-template-columns: repeat(var(--n), minmax(0, 1fr)); align-items: end; }
  .col { position: relative; height: 100%; display: flex; align-items: flex-end; justify-content: center; padding: 0 1px; cursor: default; }
  .col::before { content: ""; position: absolute; inset: 0 1px; border-radius: 6px; background: var(--surface-2); opacity: 0; transition: opacity var(--t-fast); }
  .col.on::before { opacity: 1; }
  .bar { position: relative; width: min(24px, 100%); min-height: 2px; border-radius: 4px 4px 0 0; background: var(--accent); opacity: 0.85; transition: height var(--t-slow) var(--ease-out), opacity var(--t-fast); animation: grow 700ms var(--ease-out) both; animation-delay: var(--d); transform-origin: bottom; }
  .bar.zero { background: var(--line-strong); opacity: 1; }
  .col.on .bar { opacity: 1; }
  @keyframes grow { from { transform: scaleY(0); } }
  /* Just above the hovered bar; a tall bar keeps it inside the plot. */
  .tip { position: absolute; bottom: min(calc(var(--h) + 10px), calc(100% - 46px)); left: 50%; transform: translateX(-50%); z-index: 3; display: grid; gap: 2px; padding: 8px 11px; border-radius: 10px; background: #f4f5f1; color: #0b0c0f; font-size: 0.78rem; white-space: nowrap; box-shadow: var(--shadow-2); pointer-events: none; animation: pop-in 140ms var(--ease-out); }
  .tip.left { left: auto; right: 0; transform: none; }
  .tip.right { left: 0; transform: none; }
  .tip strong { font-weight: 700; }
  .tip span { color: rgb(11 12 15 / 0.7); font-variant-numeric: tabular-nums; }
  .axis { display: grid; grid-template-columns: repeat(var(--n), minmax(0, 1fr)); margin-left: var(--axis-w); font-size: 0.72rem; color: var(--text-3); font-variant-numeric: tabular-nums; }
  .axis span { text-align: center; white-space: nowrap; overflow: visible; }
  @media (max-width: 560px) { .plot { height: 180px; } .chart { --axis-w: 32px; } }
</style>
