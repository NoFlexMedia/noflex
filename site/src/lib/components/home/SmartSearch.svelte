<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import { prefersReducedMotion, reveal } from "$lib/actions";
  import { art, poster } from "$lib/site";

  const bySlug = Object.fromEntries(art.map((a) => [a.slug, a]));
  // Real results and similarities from the demo server's EmbeddingGemma index:
  // synopses are what it embeds, so no query names a title.
  const queries = [
    { text: "the one where a girl raises a dragon", hits: [["sintel", "", 0.85], ["sprite-fright", "", 0.81], ["caminandes", "S1 · E3 · Llamigos", 0.8]] },
    { text: "robots attack amsterdam", hits: [["tears-of-steel", "", 0.87], ["charge", "", 0.8], ["elephants-dream", "", 0.8]] },
    { text: "a llama who won't stay behind the fence", hits: [["caminandes", "S1 · E1 · Llama Drama", 0.87], ["caminandes", "S1 · E3 · Llamigos", 0.85], ["caminandes", "", 0.85]] },
    { text: "a giant rabbit gets revenge", hits: [["big-buck-bunny", "", 0.89], ["caminandes", "S1 · E2 · Gran Dillama", 0.83], ["caminandes", "S1 · E3 · Llamigos", 0.82]] },
  ];

  let query = $state(0);
  let typed = $state(queries[0].text);
  let showing = $state(true);
  let timers = [];

  function wait(ms) { return new Promise((resolve) => timers.push(setTimeout(resolve, ms))); }

  async function start() {
    if (prefersReducedMotion()) return;
    for (let i = 0; ; i = (i + 1) % queries.length) {
      showing = false;
      query = i;
      const text = queries[i].text;
      for (let n = 0; n <= text.length; n++) { typed = text.slice(0, n); await wait(28 + Math.random() * 40); }
      await wait(260);
      showing = true;
      await wait(4200);
      for (let n = text.length; n >= 0; n -= 3) { typed = text.slice(0, n); await wait(12); }
    }
  }
  $effect(() => () => timers.forEach(clearTimeout));
</script>

<div class="panel search" use:reveal={start}>
  <div class="field" role="img" aria-label="Searching “{queries[query].text}”">
    <MagnifyingGlass size={18} />
    <span class="typed">{typed}<span class="caret"></span></span>
    <kbd>Ctrl K</kbd>
  </div>
  <ol class="hits" aria-hidden="true">
    {#if showing}
      {#each queries[query].hits as [slug, episode, similarity], i (`${query}-${i}`)}
        {@const work = bySlug[slug]}
        <li style:--i={i} class:top={i === 0}>
          <img src={poster(slug)} alt="" width="480" height="720" loading="lazy" decoding="async" />
          <span class="meta">
            <strong>{work.title}</strong>
            <small>{episode || `${work.year} · ${work.kind}`}</small>
            {#if i === 0}<span class="overview clamp">{work.overview}</span>{/if}
          </span>
          <span class="match">{Math.round(similarity * 100)}% match</span>
        </li>
      {/each}
    {/if}
  </ol>
</div>

<style>
  .search { display: grid; gap: 14px; padding: 16px; box-shadow: var(--shadow-3); background: rgb(14 16 21 / 0.9); }
  .field { display: flex; align-items: center; gap: 12px; height: 50px; padding: 0 14px 0 16px; border-radius: var(--r); background: rgb(255 255 255 / 0.05); border: 1px solid rgb(165 220 255 / 0.45); box-shadow: 0 0 0 4px rgb(165 220 255 / 0.1); color: var(--text-3); }
  .typed { flex: 1; min-width: 0; color: var(--text); font-size: 0.98rem; white-space: nowrap; overflow: hidden; }
  .caret { display: inline-block; width: 2px; height: 1.1em; margin-left: 1px; vertical-align: -0.18em; background: var(--focus); animation: blink 1s steps(1) infinite; }
  @keyframes blink { 50% { opacity: 0; } }
  kbd { padding: 2px 7px; border-radius: 6px; border: 1px solid var(--line-strong); color: var(--text-3); font-size: 0.72rem; }
  .hits { display: grid; gap: 4px; min-height: 252px; align-content: start; }
  li { display: grid; grid-template-columns: 46px minmax(0, 1fr) auto; align-items: center; gap: 14px; padding: 8px; border-radius: var(--r); animation: hit-in 560ms var(--ease-out) both; animation-delay: calc(var(--i) * 70ms); }
  li.top { grid-template-columns: 74px minmax(0, 1fr) auto; background: var(--surface-2); align-items: start; }
  @keyframes hit-in { from { opacity: 0; transform: translateY(10px) scale(0.98); filter: blur(4px); } }
  img { width: 100%; height: auto; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 7px; }
  .meta { display: grid; gap: 2px; min-width: 0; }
  .meta strong { font-size: 0.92rem; }
  .meta small { color: var(--text-3); font-size: 0.78rem; }
  .overview { margin-top: 6px; color: var(--text-2); font-size: 0.82rem; line-height: 1.45; }
  .clamp { display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .match { font-size: 0.72rem; font-weight: 650; color: var(--text-3); white-space: nowrap; }
  .top .match { color: var(--accent); }
</style>
