<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import { CategoryShelvesStore } from "$houdini";
  import { failure, refetch } from "$lib/graphql.js";
  import { mergeRanked } from "$lib/merge.js";
  import { askServersForLists, connectedServers } from "$lib/servers.svelte.js";
  import CategoryGrid from "$components/CategoryGrid.svelte";
  import CategoryTabs from "$components/CategoryTabs.svelte";
  import Rail from "$components/Rail.svelte";
  import State from "$components/State.svelte";

  let { data: route } = $props();

  const released = (work) => work.releaseDate && Date.parse(work.releaseDate) <= Date.now();
  const SHELVES = {
    recommendations: (data) => data.recommendations.nodes,
    newlyReleased: (data) => data.newlyReleased.nodes.filter(released),
    recentlyAdded: (data) => data.recentlyAdded.nodes,
  };
  const listsOf = (data) => Object.fromEntries(Object.entries(SHELVES).map(([shelf, pick]) => [shelf, pick(data)]));

  let shelves = $derived(route.shelves);
  let own = $derived(shelves && $shelves.data ? listsOf($shelves.data) : null);

  // Other servers fill in as they answer; this server's rails never wait on them.
  let elsewhere = $state([]);
  let unreachable = $state([]);
  $effect(() => {
    const category = route.category;
    elsewhere = [];
    unreachable = [];
    if (!category.shelves || !connectedServers().length) return;
    let stale = false;
    askServersForLists(new CategoryShelvesStore(), { category: category.value }, listsOf).then((asked) => {
      if (stale) return;
      elsewhere = asked.answers;
      unreachable = asked.missing;
    });
    return () => { stale = true; };
  });
  const merged = (shelf) => (elsewhere.length ? mergeRanked([own[shelf], ...elsewhere.map((answer) => answer.lists[shelf])]) : own[shelf]);
  let empty = $derived(own && !merged("recommendations").length && !merged("recentlyAdded").length);
  let allHref = $derived(`/browse/${route.category.slug}/all`);
</script>

<svelte:head><title>{route.category.label} · NoFlex</title></svelte:head>

{#if !route.category.shelves}
  <CategoryGrid category={route.category} works={route.works} />
{:else}
  <div class="page">
    <CategoryTabs category={route.category} current="recommended" />
    <header class="page-head"><div><p class="eyebrow">Browse</p><h1>{route.category.label}</h1></div></header>
    {#if failure($shelves) && !own}
      <State kind="error" title="Couldn’t open this page" message={failure($shelves)} action={() => refetch(shelves)} />
    {:else if !own}
      <div class="rails" aria-busy="true">{#each [0, 1, 2] as _}<div class="skeleton shelf-skel"></div>{/each}</div>
    {:else if empty}
      <State icon={Sparkle} title="Nothing here yet" message="Check back after the next scan." />
    {:else}
      <div class="rails">
        {#if unreachable.length}<p class="away" role="status">{unreachable.join(", ")} can’t be reached right now.</p>{/if}
        {#if merged("recommendations").length}<Rail title="Recommended for you" eyebrow="Because of what you watch" items={merged("recommendations")} />{/if}
        {#if merged("newlyReleased").length}<Rail title="Newly released" items={merged("newlyReleased")} href="{allHref}?sort=DATE_RELEASED_DESC" />{/if}
        {#if merged("recentlyAdded").length}<Rail title="Recently added" items={merged("recentlyAdded")} href={allHref} />{/if}
      </div>
      <p class="all"><a class="btn outline" href={allHref}>Browse all</a></p>
    {/if}
  </div>
{/if}

<style>
  .rails { display: grid; gap: 28px; min-width: 0; }
  .shelf-skel { height: 300px; border-radius: var(--r-l); }
  .away { font-size: 0.86rem; color: var(--text-3); }
  .all { margin-top: 28px; }
</style>
