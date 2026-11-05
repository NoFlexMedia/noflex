<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
  import Stack from "phosphor-svelte/lib/Stack";
  import Television from "phosphor-svelte/lib/Television";
  import { graphql } from "$houdini";
  import { titleCase } from "$lib/format.js";
  import { fetchQuery } from "$lib/graphql.js";
  import { serverRequest, tagged } from "$lib/servers.svelte.js";
  import Artwork from "$components/Artwork.svelte";

  const ICON = { MOVIES: FilmStrip, SHOWS: Television, MUSIC: MusicNotes, AUDIOBOOKS: BookOpen, PHOTOS: ImageIcon, MIXED: Stack };
  /** `server`: the linked server the library is on; omit for this one. */
  let { library, server = null } = $props();
  let Glyph = $derived(ICON[library.kind] ?? Books);
  const peek = graphql(`
    query LibraryPeek($library: ID!) {
      works(libraryId: $library, filter: { recursive: false }, sort: DATE_ADDED_DESC, first: 3) { nodes { ...WorkCard } }
    }
  `);
  let libraryId = $derived(library.id);
  let linkedPosters = $state([]);
  $effect(() => {
    if (!server) { fetchQuery(peek, { library: libraryId }); return; }
    let stale = false;
    serverRequest(server, peek.artifact.raw, { library: libraryId }).then(
      (data) => { if (!stale) linkedPosters = tagged(data.works.nodes, server); },
      () => {},
    );
    return () => { stale = true; };
  });
  let posters = $derived(server ? linkedPosters : $peek.data?.works.nodes ?? []);
</script>

<a class="tile" href={server ? `/libraries/${library.id}?server=${server.id}` : `/libraries/${library.id}`} data-sveltekit-preload-data={server ? "off" : undefined}>
  <div class="text">
    <span class="glyph"><Glyph size={20} /></span>
    <h2>{library.name}</h2>
    <p>{titleCase(library.kind)}{#if library.config?.watchEnabled} · <span class="watching"><span class="dot on"></span>Watching</span>{/if}</p>
  </div>
  <div class="fan" aria-hidden="true">
    {#each posters as work, index (work.id)}
      <div class="card" style:--n={index}><Artwork {work} width={120} /></div>
    {:else}
      {#each [0, 1, 2] as index}<div class="card ghost" style:--n={index}></div>{/each}
    {/each}
  </div>
</a>

<style>
  .tile { position: relative; display: flex; justify-content: space-between; align-items: flex-end; min-height: 200px; padding: 24px; border-radius: var(--r-xl); background: radial-gradient(120% 120% at 100% 0%, rgb(var(--ambient) / 0.25), transparent 60%), var(--surface); border: 1px solid var(--line); overflow: hidden; perspective: 900px; transition: scale 160ms var(--ease-out), border-color var(--t), transform 400ms var(--ease-out), box-shadow 400ms; }
  .tile:hover { border-color: var(--line-strong); transform: translateY(-3px); box-shadow: var(--shadow-2); }
  .tile:active { scale: 0.985; }
  .text { position: relative; z-index: 1; display: grid; gap: 4px; }
  .glyph { display: grid; place-items: center; width: 42px; height: 42px; border-radius: 13px; background: var(--surface-3); margin-bottom: 14px; color: var(--accent); }
  h2 { font-size: 1.36rem; }
  p { color: var(--text-3); font-size: 0.88rem; display: flex; gap: 6px; align-items: center; }
  .watching { display: inline-flex; align-items: center; gap: 6px; }
  .fan { position: absolute; right: 26px; top: 26px; width: 120px; height: 180px; transform-style: preserve-3d; }
  .card { position: absolute; inset: 0; border-radius: 10px; box-shadow: 0 18px 30px -12px rgb(0 0 0 / 0.9); transform-origin: 50% 100%; transform: translateX(calc(var(--n) * -26px)) rotateY(-18deg) rotateZ(calc(var(--n) * -6deg)) translateZ(calc(var(--n) * -30px)); transition: transform 600ms var(--ease-spring); z-index: calc(3 - var(--n)); }
  .card :global(.art) { border-radius: 10px; }
  .card.ghost { background: var(--surface-2); }
  .tile:hover .card { transform: translateX(calc(var(--n) * -54px)) rotateY(-8deg) rotateZ(calc(var(--n) * -9deg)) translateZ(calc(var(--n) * -20px)) translateY(-6px); }
  @media (max-width: 480px) { .fan { opacity: 0.35; } }
</style>
