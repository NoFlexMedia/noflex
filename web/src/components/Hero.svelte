<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Info from "phosphor-svelte/lib/Info";
  import Play from "phosphor-svelte/lib/Play";
  import Star from "phosphor-svelte/lib/Star";
  import { onDestroy } from "svelte";
  import { ambientColor, pickImage } from "$lib/images.js";
  import { runtime, titleCase } from "$lib/format.js";
  import { player, playWork } from "$lib/playback.svelte.js";
  import { ambient } from "$lib/ui.svelte.js";
  import Artwork from "$components/Artwork.svelte";

  let { items = [], eyebrow = "Just added" } = $props();
  const DWELL = 9000;
  let index = $state(0);
  let paused = $state(false);
  let current = $derived(items[index] ?? items[0]);
  let timer;

  $effect(() => {
    if (items.length < 2 || paused) return;
    index;
    timer = setTimeout(() => (index = (index + 1) % items.length), DWELL);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    const url = pickImage(current?.images, "backdrop");
    if (!url) return;
    let stale = false;
    ambientColor(url).then((color) => { if (!stale && color) ambient.color = color; });
    return () => { stale = true; };
  });
  onDestroy(() => { ambient.color = null; });

  function resumeLabel(work) {
    const data = work.userData;
    if (data && !data.played && data.positionMs > 60_000) return `Resume · ${runtime(work.runtimeMs ? work.runtimeMs - data.positionMs : 0) || "in progress"}${work.runtimeMs ? " left" : ""}`;
    return "Play";
  }
</script>

<svelte:document onvisibilitychange={() => (paused = document.hidden)} />

{#if current}
  <section class="hero" aria-roledescription="carousel" aria-label="Featured" onpointerenter={() => (paused = true)} onpointerleave={() => (paused = false)} onfocusin={() => (paused = true)} onfocusout={() => (paused = false)}>
    <div class="stage" aria-hidden="true">
      {#each items as work, i (work.id)}
        <div class="slide" class:active={i === index}><Artwork {work} kind="backdrop" width={1600} eager={i === 0} rounded={false} /></div>
      {/each}
    </div>
    <div class="shade" aria-hidden="true"></div>

    {#key current.id}
      <div class="content" aria-live="polite">
        <p class="eyebrow">{eyebrow}</p>
        {#if pickImage(current.images, "logo")}
          <h1 class="logo"><span class="sr-only">{current.title}</span><Artwork work={current} kind="logo" width={420} rounded={false} /></h1>
        {:else}
          <h1>{current.title}</h1>
        {/if}
        <p class="meta">
          {#if current.communityRating}<span class="rating"><Star size={13} weight="fill" />{current.communityRating.toFixed(1)}</span>{/if}
          {#each [current.year, titleCase(current.kind), runtime(current.runtimeMs)].filter(Boolean) as part}<span>{part}</span>{/each}
        </p>
        {#if current.overview}<p class="overview clamp-3">{current.overview}</p>{/if}
        <div class="row actions">
          <button class="btn light lg" onclick={() => playWork(current)} disabled={player.preparing === current.id}>
            {#if player.preparing === current.id}<span class="spinner sm"></span>{:else}<Play size={18} weight="fill" />{/if}{resumeLabel(current)}
          </button>
          <a class="btn glass lg" href="/works/{current.id}"><Info size={18} />More info</a>
        </div>
      </div>
    {/key}

    {#if items.length > 1}
      <div class="dots" role="tablist" aria-label="Featured titles">
        {#each items as work, i (work.id)}
          <button role="tab" aria-selected={i === index} aria-label={work.title} class:active={i === index} class:paused onclick={() => (index = i)} style:--dwell="{DWELL}ms"><span></span></button>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .hero { position: relative; height: clamp(440px, 74vh, 760px); margin: calc(var(--topbar-h) * -1 - 20px) calc(var(--gutter) * -1) 0; display: flex; align-items: flex-end; padding: 0 var(--gutter) clamp(56px, 9vh, 92px); overflow: hidden; isolation: isolate; }
  .stage { position: absolute; inset: 0; z-index: -2; }
  .slide { position: absolute; inset: 0; opacity: 0; transition: opacity 1.1s var(--ease-in-out); }
  .slide :global(.art) { position: absolute; inset: 0; aspect-ratio: auto; height: 100%; }
  .slide :global(img) { transform: scale(1.08); transition: opacity 600ms var(--ease-out), filter 600ms var(--ease-out), transform 12s linear; }
  .slide.active { opacity: 1; }
  .slide.active :global(.loaded img) { transform: scale(1.0) translate3d(-1.2%, -0.6%, 0); }
  .shade { position: absolute; inset: 0; z-index: -1; background: linear-gradient(90deg, rgb(7 8 11 / 0.9) 0%, rgb(7 8 11 / 0.55) 38%, transparent 70%), linear-gradient(0deg, var(--bg) 0%, rgb(7 8 11 / 0.6) 22%, transparent 55%), linear-gradient(180deg, rgb(7 8 11 / 0.6), transparent 22%); }
  .content { max-width: min(620px, 92%); display: grid; gap: 14px; }
  .content > * { animation: hero-in 800ms var(--ease-out) both; }
  .content > :nth-child(2) { animation-delay: 60ms; }
  .content > :nth-child(3) { animation-delay: 120ms; }
  .content > :nth-child(4) { animation-delay: 180ms; }
  .content > :nth-child(5) { animation-delay: 240ms; }
  @keyframes hero-in { from { opacity: 0; transform: translateY(16px); filter: blur(4px); } }
  h1 { font-size: clamp(2.3rem, 5vw, 4.3rem); font-weight: 750; letter-spacing: -0.045em; line-height: 0.98; text-shadow: 0 4px 40px rgb(0 0 0 / 0.5); }
  h1.logo { width: min(420px, 80%); }
  h1.logo :global(.art) { aspect-ratio: 3 / 1; }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 14px; color: var(--text-2); font-size: 0.92rem; font-weight: 550; }
  .meta span + span::before { content: "·"; margin-right: 14px; color: var(--text-3); }
  .meta .rating { display: inline-flex; align-items: center; gap: 5px; color: var(--warning); }
  .meta .rating + span::before { content: ""; margin: 0; }
  .overview { color: var(--text-2); font-size: 1.02rem; line-height: 1.55; max-width: 560px; }
  .actions { margin-top: 8px; gap: 12px; }
  .dots { position: absolute; right: var(--gutter); bottom: clamp(56px, 9vh, 92px); display: flex; gap: 8px; }
  .dots button { width: 28px; height: 20px; display: grid; align-items: center; }
  .dots span { display: block; height: 3px; border-radius: 9px; background: rgb(255 255 255 / 0.25); overflow: hidden; position: relative; transition: background var(--t); }
  .dots button:hover span { background: rgb(255 255 255 / 0.45); }
  .dots .active span::after { content: ""; position: absolute; inset: 0; background: #fff; transform-origin: left; animation: fill var(--dwell) linear forwards; }
  .dots .active.paused span::after { animation-play-state: paused; }
  @keyframes fill { from { transform: scaleX(0); } }
  @media (max-width: 759px) {
    .hero { height: 68vh; min-height: 420px; padding-bottom: 40px; }
    .dots { left: var(--gutter); right: auto; bottom: 16px; }
    .overview { display: none; }
  }
</style>
