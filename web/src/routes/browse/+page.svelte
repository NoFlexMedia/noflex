<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import { availableCategories } from "$lib/categories.js";
  import { app } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";

  let categories = $derived(availableCategories(app.categories));
</script>

<svelte:head><title>Browse · NoFlex</title></svelte:head>

<div class="page">
  <header class="page-head"><div><p class="eyebrow">Your media</p><h1>Browse</h1></div></header>
  {#if categories.length}
    <div class="tiles stagger">
      {#each categories as category, index (category.value)}
        <a class="tile" href="/browse/{category.slug}" style:--i={index}><category.icon size={24} /><span><strong>{category.label}</strong><small>{category.workCount} {category.workCount === 1 ? "title" : "titles"}</small></span></a>
      {/each}
    </div>
  {:else}
    <State icon={Books} title="Nothing to browse yet" message={app.viewer.isAdmin ? "Add a library and run a scan." : "An administrator hasn’t shared a library with you yet."} />
  {/if}
  {#if app.libraries.length}<p class="alt"><a class="btn ghost sm" href="/libraries"><Books size={16} />Browse by library</a></p>{/if}
</div>

<style>
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 240px), 1fr)); gap: 14px; }
  .tile { display: flex; align-items: center; gap: 14px; padding: 18px; border: 1px solid var(--line); border-radius: var(--r); color: var(--text); transition: border-color var(--t); }
  .tile:hover { border-color: var(--text-3); }
  .tile span { display: grid; gap: 2px; }
  .tile small { color: var(--text-3); }
  .alt { margin-top: 22px; }
</style>
