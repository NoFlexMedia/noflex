<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { page } from "$app/state";
  import ArrowLeft from "phosphor-svelte/lib/ArrowLeft";
  import Lifebuoy from "phosphor-svelte/lib/Lifebuoy";
  import { poster, stills } from "$lib/site";

  let missing = $derived(page.status === 404);
</script>

<svelte:head><title>{missing ? "Page not found" : "Something went wrong"} · NoFlex</title></svelte:head>

<section class="error wrap">
  <div class="cards" aria-hidden="true">
    {#each stills.slice(0, 3) as item, i}<img src={poster(item.slug)} alt="" width="480" height="720" style:--i={i} />{/each}
  </div>
  <p class="eyebrow">{page.status}</p>
  <h1>{missing ? "This page wandered off." : "Something went wrong."}</h1>
  <p class="lede">{missing ? "It may have moved, or the link was mistyped. Everything else is where you left it." : "Try again in a moment. If it keeps happening, let us know."}</p>
  <div class="ctas">
    <a class="btn light lg" href="/"><ArrowLeft size={18} />Back home</a>
    <a class="btn glass lg" href="/support"><Lifebuoy size={18} />Support</a>
  </div>
</section>

<style>
  .error { min-height: 100svh; display: grid; place-content: center; justify-items: center; gap: 18px; text-align: center; padding-top: var(--topbar-h); padding-bottom: 80px; }
  .cards { position: relative; width: 220px; height: 170px; margin-bottom: 18px; }
  .cards img { position: absolute; left: 50%; bottom: 0; width: 96px; height: auto; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 10px; box-shadow: var(--shadow-3); outline: 1px solid rgb(255 255 255 / 0.08); transform-origin: 50% 120%; translate: -50% 0; animation: fan 1s var(--ease-spring) both; animation-delay: calc(var(--i) * 80ms); }
  .cards img:nth-child(1) { rotate: -16deg; }
  .cards img:nth-child(2) { rotate: 0deg; z-index: 1; }
  .cards img:nth-child(3) { rotate: 16deg; }
  @keyframes fan { from { rotate: 0deg; opacity: 0; translate: -50% 20px; } }
  h1 { font-size: clamp(2.4rem, 6vw, 4.4rem); font-weight: 750; letter-spacing: -0.05em; line-height: 1; }
  .lede { color: var(--text-2); max-width: 48ch; }
  .ctas { display: flex; flex-wrap: wrap; justify-content: center; gap: 12px; margin-top: 8px; }
</style>
