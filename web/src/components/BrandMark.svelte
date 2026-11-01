<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  // Keep in sync with static/favicon.svg.
  let { size = 30, intro = false } = $props();
</script>

<svg viewBox="0 0 64 64" width={size} height={size} aria-hidden="true" focusable="false">
  <path d="M32 2C61.6 2 62 2.4 62 32S61.6 62 32 62 2 61.6 2 32 2.4 2 32 2Z" fill="var(--accent)" />
  <g class:intro>
    <rect x="13.5" y="14" width="6.5" height="36" rx="3.25" fill="var(--accent-ink)" />
    <path d="M23.5 15 37 20 30.5 26.5 50 32 30.5 37.5 37 44 23.5 49Z" fill="var(--accent-ink)" stroke="var(--accent-ink)" stroke-width="3" stroke-linejoin="round" />
  </g>
</svg>

<style>
  /* The crown starts upright and is set down on its side. */
  .intro { transform-box: view-box; transform-origin: 32px 32px; animation: set-down 900ms var(--ease-spring) both 0.1s; }
  @keyframes set-down { from { transform: rotate(-90deg); } }
</style>
