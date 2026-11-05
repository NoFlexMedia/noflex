<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { CategoryWorksStore } from "$houdini";
  import { worksVariables } from "$lib/workFilters.js";
  import { askServers, connectedServers } from "$lib/servers.svelte.js";
  import Rail from "$components/Rail.svelte";
  import Segmented from "$components/ui/Segmented.svelte";
  import WorkBrowser from "$components/WorkBrowser.svelte";

  /** A category's full grid. `works` is its `CategoryWorks` store; `above` renders before the title. */
  let { category, works, above = null } = $props();

  const ANIME_KINDS = [{ value: "", label: "All" }, { value: "SERIES", label: "Series" }, { value: "MOVIE", label: "Movies" }];
  const TYPE_FILTERS = { MUSIC: ["ARTIST", "ALBUM", "TRACK"] };

  // A grid can't page through several servers at once, so each gets its own rail.
  let elsewhere = $state([]);
  $effect(() => {
    const variables = { category: category.value, ...worksVariables(page.url.searchParams) };
    elsewhere = [];
    if (!connectedServers().length) return;
    let stale = false;
    askServers(new CategoryWorksStore(), variables, (data) => data.works.edges.map((edge) => edge.node)).then((asked) => {
      if (!stale) elsewhere = asked.answers.filter((answer) => answer.works.length);
    });
    return () => { stale = true; };
  });

  function showKind(kind) {
    const next = new URLSearchParams(page.url.searchParams);
    if (kind) next.set("kind", kind); else next.delete("kind");
    goto(`?${next}`, { replaceState: true, noScroll: true, keepFocus: true });
  }
</script>

<WorkBrowser {works} {above} scope={{ category: category.value }} title={category.label} eyebrow="Browse" kinds={TYPE_FILTERS[category.value] ?? []} allowRecursive={false} emptyTitle="Nothing here yet" emptyMessage="Check back after the next scan.">
  {#snippet toolbar()}
    {#if category.value === "ANIME"}<Segmented value={page.url.searchParams.get("kind") ?? ""} items={ANIME_KINDS} label="Anime type" onchange={showKind} />{/if}
  {/snippet}
</WorkBrowser>

{#if elsewhere.length}
  <div class="page elsewhere">
    {#each elsewhere as answer (answer.server.id)}<Rail title="On {answer.server.name}" items={answer.works} />{/each}
  </div>
{/if}

<style>
  .elsewhere { display: grid; gap: 28px; padding-top: 0; }
</style>
