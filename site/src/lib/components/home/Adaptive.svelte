<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import Info from "phosphor-svelte/lib/Info";
  import { prefersReducedMotion, reveal } from "$lib/actions";

  // The default H.264 ladder's steps and rates (noflex-server/src/config.rs, `ladder_steps`).
  const steps = [
    { label: "Original", rate: "1080p · 20 Mb/s" },
    { label: "1080p", rate: "8.0 Mb/s" },
    { label: "720p", rate: "4.0 Mb/s" },
    { label: "480p", rate: "2.0 Mb/s" },
    { label: "360p", rate: "1.1 Mb/s" },
  ];
  // What Auto does as the connection changes (lib/ts/quality.ts).
  const moments = [
    { speed: 25, connection: "25 Mb/s", step: 0, playing: "Original", note: "The file plays as stored." },
    { speed: 6, connection: "6 Mb/s", step: 0, playing: "Original, stalling", note: "Two stalls in a minute: the stream reopens at the same second, as a ladder encoded on demand.", warn: true },
    { speed: 6, connection: "6 Mb/s", step: 2, playing: "720p", note: "The player picks the step the connection holds." },
    { speed: 1.6, connection: "1.6 Mb/s", step: 4, playing: "360p", note: "It steps down again without a pause." },
    { speed: 30, connection: "30 Mb/s for a minute", step: 0, playing: "Original", note: "Room again, so it's back to the original." },
  ];

  let index = $state(0);
  let now = $derived(moments[index]);
  let timer = 0;
  function start() {
    if (prefersReducedMotion()) return;
    timer = setInterval(() => (index = (index + 1) % moments.length), 3400);
  }
  $effect(() => () => clearInterval(timer));
</script>

<div class="grid" use:reveal={start}>
  <div class="panel menu">
    <span class="eyebrow">Quality</span>
    <ul>
      <li class="auto"><Check size={16} /><strong>Auto</strong><small>adapts to your connection</small></li>
      {#each steps as step, i}
        <li class:here={now.step === i}><strong>{step.label}</strong><small>{step.rate}</small></li>
      {/each}
    </ul>
  </div>

  <div class="panel story">
    <span class="eyebrow">Auto, as it happens</span>
    <div class="meter" aria-hidden="true"><span style:width="{Math.min(100, (now.speed / 30) * 100)}%" class:low={now.speed < 10}></span></div>
    {#key index}
      <dl>
        <dt>Connection</dt><dd>{now.connection}</dd>
        <dt>Playing</dt><dd><span class="plan" class:warn={now.warn}>{now.playing}</span></dd>
      </dl>
      <p class="why"><Info size={16} /><span>{now.note}</span></p>
    {/key}
    <ol class="history" aria-label="The connection over time, and the quality played">
      {#each moments as moment, i}
        <li class:here={i === index} class:warn={moment.warn || moment.step > 0} aria-current={i === index ? "step" : undefined}>
          <span class="bar" style:height="{Math.max(12, (moment.speed / 30) * 100)}%"></span>
          <small>{steps[moment.step].label}</small>
        </li>
      {/each}
    </ol>
  </div>
</div>

<style>
  .grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr); gap: 16px; }
  ul { display: grid; gap: 4px; margin-top: 14px; }
  .menu li { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: baseline; gap: 12px; padding: 9px 12px; border-radius: var(--r-s); border: 1px solid transparent; transition: background var(--t) var(--ease-out), border-color var(--t) var(--ease-out); }
  .menu li small { color: var(--text-3); font-size: 0.8rem; font-variant-numeric: tabular-nums; }
  .menu li strong { font-weight: 550; }
  .menu li.auto { grid-template-columns: auto minmax(0, 1fr) auto; margin-bottom: 6px; background: var(--surface-2); }
  .menu li.auto :global(svg) { color: var(--accent); align-self: center; }
  .menu li.here { background: var(--accent-soft); border-color: rgb(212 255 90 / 0.4); }
  .menu li.here strong { color: var(--accent); }
  .story { display: grid; grid-template-rows: auto auto auto auto 1fr; gap: 18px; }
  .history { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 10px; align-self: end; min-height: 120px; }
  .history li { display: grid; grid-template-rows: 1fr auto; justify-items: center; gap: 8px; opacity: 0.45; transition: opacity var(--t) var(--ease-out); }
  .history li.here { opacity: 1; }
  .history .bar { align-self: end; width: 100%; border-radius: var(--r-s) var(--r-s) 4px 4px; background: var(--accent); }
  .history li.warn .bar { background: var(--warning); }
  .history small { font-size: 0.74rem; color: var(--text-3); }
  .meter { height: 6px; border-radius: 99px; background: var(--surface-3); overflow: hidden; }
  .meter span { display: block; height: 100%; border-radius: inherit; background: var(--accent); transition: width 900ms var(--ease-out), background var(--t) var(--ease-out); }
  .meter span.low { background: var(--warning); }
  dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 10px 18px; align-items: center; animation: fade-up 480ms var(--ease-out) both; }
  dt { font-size: 0.72rem; color: var(--text-3); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; }
  dd { font-weight: 600; }
  .plan { display: inline-flex; align-items: center; height: 24px; padding: 0 10px; border-radius: 99px; background: var(--accent-soft); color: var(--accent); font-size: 0.8rem; font-weight: 650; }
  .plan.warn { background: rgb(255 193 100 / 0.14); color: var(--warning); }
  .why { display: flex; gap: 10px; color: var(--text-2); font-size: 0.9rem; animation: fade-up 480ms var(--ease-out) both; }
  .why :global(svg) { flex: none; margin-top: 3px; color: var(--text-3); }
  @media (max-width: 899px) { .grid { grid-template-columns: 1fr; } }
</style>
