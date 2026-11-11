<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import DeviceMobile from "phosphor-svelte/lib/DeviceMobile";
  import Pause from "phosphor-svelte/lib/Pause";
  import Television from "phosphor-svelte/lib/Television";
  import WifiHigh from "phosphor-svelte/lib/WifiHigh";
  import { prefersReducedMotion, reveal } from "$lib/actions";
  import { backdrop } from "$lib/site";

  // What the planner would want for one viewer: the next episodes, their
  // unfinished film, a mark, and a likely pick.
  const rows = [
    { slug: "caminandes", title: "Caminandes", sub: "S1 · E2 · Gran Dillama", why: "Next episode", device: "Living room TV", spec: "AV1 · 1080p" },
    { slug: "caminandes", title: "Caminandes", sub: "S1 · E3 · Llamigos", why: "Next episode", device: "Living room TV", spec: "AV1 · 1080p" },
    { slug: "sprite-fright", title: "Sprite Fright", sub: "41 minutes left", why: "Continue watching", device: "Pixel 9", spec: "AV1 · 720p" },
    { slug: "elephants-dream", title: "Elephants Dream", sub: "2006 · Movie", why: "Marked by you", device: "Pixel 9", spec: "HEVC · 720p" },
    { slug: "sintel", title: "Sintel", sub: "2010 · Movie", why: "Recommended", device: "Living room TV", spec: "AV1 · 4K HDR10" },
  ];

  let progress = $state(rows.map(() => 0));
  let paused = $state(false);
  let running = false;
  let timer = 0;
  $effect(() => () => clearInterval(timer));

  // Fill each row in turn, pausing midway as someone presses play, then start over.
  function start() {
    if (running) return;
    running = true;
    if (prefersReducedMotion()) { progress = rows.map((_, i) => (i < 3 ? 1 : 0.42)); return; }
    let row = 0;
    let hold = 0;
    let pausedOnce = false;
    const tick = () => {
      if (hold > 0) { hold--; return; }
      if (row >= rows.length) { progress = rows.map(() => 0); row = 0; pausedOnce = false; hold = 14; return; }
      if (!pausedOnce && row === 3 && progress[row] > 0.4) { pausedOnce = true; paused = true; setTimeout(() => (paused = false), 2600); hold = 26; return; }
      if (paused) return;
      progress[row] = Math.min(1, progress[row] + 0.035 + Math.random() * 0.03);
      if (progress[row] >= 1) { row++; hold = 6; }
    };
    timer = setInterval(tick, 100);
  }
</script>

<div class="queue panel" use:reveal={start}>
  <div class="head">
    <div>
      <p class="eyebrow">Smart transcoding</p>
      <h3>Getting ready for tonight</h3>
    </div>
    <span class="state" class:paused>
      {#if paused}<Pause size={13} weight="fill" />Paused, someone's watching{:else}<span class="dot on"></span>Working below 50% CPU{/if}
    </span>
  </div>
  <ul>
    {#each rows as row, i}
      {@const done = progress[i] >= 1}
      <li class:done>
        <img src={backdrop(row.slug)} alt="" width="1600" height="900" loading="lazy" decoding="async" />
        <div class="text">
          <strong>{row.title}</strong>
          <small>{row.sub}</small>
        </div>
        <span class="why chip">{row.why}</span>
        <div class="status">
          <span class="device">{#if row.device.includes("TV")}<Television size={14} />{:else}<DeviceMobile size={14} />{/if}{row.device}</span>
          {#if done}
            <span class="ready"><Check size={12} weight="bold" />{row.spec}</span>
          {:else}
            <span class="bar" aria-label="{Math.round(progress[i] * 100)}% prepared"><span style:transform="scaleX({progress[i]})"></span></span>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
  <div class="foot">
    <DeviceMobile size={18} />
    <div>
      <strong>Smart Downloads</strong>
      <small><WifiHigh size={12} /> Synced to Pixel 9 on Wi-Fi · 4.2 of 10 GB</small>
    </div>
    <span class="budget"><span></span></span>
  </div>
</div>

<style>
  .queue { padding: 0; overflow: hidden; background: linear-gradient(180deg, rgb(255 255 255 / 0.05), rgb(255 255 255 / 0.02)); box-shadow: var(--shadow-3); }
  .head { display: flex; align-items: flex-end; justify-content: space-between; gap: 16px; flex-wrap: wrap; padding: 20px 22px 16px; border-bottom: 1px solid var(--line); }
  h3 { font-size: 1.08rem; margin-top: 4px; }
  .state { display: inline-flex; align-items: center; gap: 8px; height: 28px; padding: 0 12px; border-radius: 999px; background: var(--surface-2); font-size: 0.78rem; font-weight: 600; color: var(--text-2); transition: background var(--t), color var(--t); }
  .state.paused { background: rgb(255 193 100 / 0.14); color: var(--warning); animation: nudge 500ms var(--ease-spring); }
  @keyframes nudge { from { transform: scale(0.92); } }
  ul { display: grid; padding: 6px 10px; }
  li { display: grid; grid-template-columns: 76px minmax(0, 1fr) auto 150px; align-items: center; gap: 14px; padding: 10px 12px; border-radius: var(--r); transition: background var(--t); }
  li:hover { background: var(--surface); }
  img { width: 76px; aspect-ratio: 16 / 9; height: auto; object-fit: cover; border-radius: 7px; outline: 1px solid rgb(255 255 255 / 0.06); outline-offset: -1px; }
  .text { display: grid; min-width: 0; }
  .text strong { font-size: 0.9rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .text small { color: var(--text-3); font-size: 0.78rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .why { height: 22px; font-size: 0.72rem; }
  .status { display: grid; gap: 6px; justify-items: end; }
  .device { display: inline-flex; align-items: center; gap: 5px; color: var(--text-3); font-size: 0.74rem; font-weight: 550; white-space: nowrap; }
  .bar { width: 100%; height: 4px; border-radius: 99px; background: rgb(255 255 255 / 0.1); overflow: hidden; }
  .bar span { display: block; height: 100%; border-radius: inherit; background: var(--accent); transform-origin: left; transition: transform 120ms linear; }
  .ready { display: inline-flex; align-items: center; gap: 5px; height: 20px; padding: 0 8px 0 6px; border-radius: 99px; background: var(--accent-soft); color: var(--accent); font-size: 0.72rem; font-weight: 650; animation: pop 520ms var(--ease-spring); }
  @keyframes pop { from { transform: scale(0.6); opacity: 0; } }
  .foot { display: grid; grid-template-columns: auto minmax(0, 1fr) 120px; align-items: center; gap: 12px; padding: 14px 22px; border-top: 1px solid var(--line); background: rgb(0 0 0 / 0.18); color: var(--text-2); }
  .foot div { display: grid; }
  .foot strong { font-size: 0.86rem; color: var(--text); }
  .foot small { display: inline-flex; align-items: center; gap: 5px; color: var(--text-3); font-size: 0.76rem; }
  .budget { height: 4px; border-radius: 99px; background: rgb(255 255 255 / 0.1); overflow: hidden; }
  .budget span { display: block; width: 42%; height: 100%; background: var(--text-2); border-radius: inherit; }
  @media (max-width: 599px) {
    li { grid-template-columns: 60px minmax(0, 1fr) 96px; }
    img { width: 60px; }
    .why { display: none; }
  }
</style>
