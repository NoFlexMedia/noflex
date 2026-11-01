<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CaretLeft from "phosphor-svelte/lib/CaretLeft";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import WorkCard from "$components/WorkCard.svelte";

  let { title, eyebrow = null, items = [], wide = false, href = null, id = title.toLowerCase().replace(/\W+/g, "-") } = $props();

  let track = $state();
  let atStart = $state(true);
  let atEnd = $state(false);

  function measure() {
    if (!track) return;
    atStart = track.scrollLeft < 8;
    atEnd = track.scrollLeft + track.clientWidth >= track.scrollWidth - 8;
  }
  function page(direction) {
    track?.scrollBy({ left: direction * track.clientWidth * 0.85, behavior: "smooth" });
  }
  $effect(() => {
    if (!track) return;
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(track);
    return () => observer.disconnect();
  });
</script>

<section class="rail" class:wide aria-labelledby="rail-{id}">
  <header>
    <div>
      {#if eyebrow}<span class="eyebrow">{eyebrow}</span>{/if}
      <h2 id="rail-{id}">{#if href}<a {href}>{title}<CaretRight size={18} /></a>{:else}{title}{/if}</h2>
    </div>
    <div class="nav">
      <button class="icon-btn sm solid" aria-label="Scroll {title} back" disabled={atStart} onclick={() => page(-1)}><CaretLeft size={18} /></button>
      <button class="icon-btn sm solid" aria-label="Scroll {title} forward" disabled={atEnd} onclick={() => page(1)}><CaretRight size={18} /></button>
    </div>
  </header>
  <div class="viewport" class:fade-start={!atStart} class:fade-end={!atEnd}>
    <div class="track stagger" bind:this={track} onscroll={measure}>
      {#each items as work, index (work.id)}
        <div class="slot" style:--i={index}><WorkCard {work} {wide} key="{id}:{work.id}" /></div>
      {/each}
    </div>
  </div>
</section>

<style>
  .rail { margin-top: 38px; }
  header { display: flex; align-items: flex-end; justify-content: space-between; gap: 16px; margin-bottom: 14px; }
  h2 { font-size: 1.16rem; }
  h2 a { display: inline-flex; align-items: center; gap: 4px; transition: color var(--t); }
  h2 a :global(svg) { transition: transform var(--t) var(--ease-out); color: var(--text-3); }
  h2 a:hover :global(svg) { transform: translateX(3px); color: var(--text); }
  .nav { display: flex; gap: 6px; opacity: 0; transition: opacity var(--t) var(--ease-out); }
  .rail:hover .nav, .nav:focus-within { opacity: 1; }
  .nav button:disabled { opacity: 0.3; }
  .viewport { position: relative; margin: 0 calc(var(--gutter) * -1); }
  .viewport::before, .viewport::after { content: ""; position: absolute; top: 0; bottom: 0; width: var(--gutter); z-index: 2; pointer-events: none; opacity: 0; transition: opacity var(--t); }
  .viewport::before { left: 0; background: linear-gradient(to right, var(--bg), transparent); }
  .viewport::after { right: 0; background: linear-gradient(to left, var(--bg), transparent); }
  .viewport.fade-start::before, .viewport.fade-end::after { opacity: 1; }
  .track {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: clamp(132px, 14vw, 184px);
    gap: clamp(12px, 1.3vw, 18px);
    overflow-x: auto;
    overscroll-behavior-x: contain;
    scroll-snap-type: x mandatory;
    scroll-padding-inline: var(--gutter);
    padding: 8px var(--gutter) 14px;
    scrollbar-width: none;
  }
  .track::-webkit-scrollbar { display: none; }
  .wide .track { grid-auto-columns: clamp(250px, 26vw, 360px); }
  .slot { scroll-snap-align: start; }
  @media (hover: none) { .nav { display: none; } }
</style>
