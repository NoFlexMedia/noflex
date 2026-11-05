<script>
  import FunnelSimple from "phosphor-svelte/lib/FunnelSimple";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { inview } from "$lib/actions/index.js";
  import { titleCase } from "$lib/format.js";
  import { attempt } from "$lib/ui.svelte.js";
  import { failure, nextPage, refetch } from "$lib/graphql.js";
  import FilterBar from "$components/filters/FilterBar.svelte";
  import State from "$components/State.svelte";
  import WorkCard from "$components/WorkCard.svelte";
  import Select from "$components/ui/Select.svelte";

  /**
   * The poster grid with its sort and filter controls, driven by URL
   * parameters. `works` is a paginated store selecting `works.edges`;
   * `kinds` lists the types its filter offers (none hides the control);
   * `allowRecursive` offers listing episodes and tracks as well;
   * `toolbar` renders beside the sort control; `missing` replaces the grid
   * when the thing being browsed doesn't exist. `scope` is `{ library }` or
   * `{ category }`, whose values the filters offer. `above` renders before the title.
   */
  let { works, title, eyebrow, kinds = [], allowRecursive = true, emptyTitle, emptyMessage, toolbar, missing, scope, above = null } = $props();

  const SORTS = [
    { value: "DATE_ADDED_DESC", label: "Recently added" },
    { value: "TITLE_ASC", label: "Title A–Z" },
    { value: "TITLE_DESC", label: "Title Z–A" },
    { value: "YEAR_DESC", label: "Newest release" },
    { value: "YEAR_ASC", label: "Oldest release" },
    { value: "DATE_RELEASED_DESC", label: "Release date" },
    { value: "RANDOM", label: "Shuffle" },
  ];
  let params = $derived(page.url.searchParams);
  let sort = $derived(params.get("sort") ?? "DATE_ADDED_DESC");
  let kind = $derived(params.get("kind") ?? "");
  let includeAll = $derived(params.get("all") === "1");

  function update(changes) {
    const next = new URLSearchParams(page.url.searchParams);
    for (const [key, value] of Object.entries(changes)) {
      if (value === "" || value === false || value == null) next.delete(key); else next.set(key, value === true ? "1" : String(value));
    }
    goto(`?${next}`, { replaceState: true, noScroll: true, keepFocus: true });
  }

  let items = $derived($works.data?.works.edges.map((edge) => edge.node) ?? []);
  let loading = $derived($works.fetching);
  let error = $derived(failure($works));
  let more = $state(false);

  async function loadMore() {
    if (!$works.pageInfo.hasNextPage || more || loading) return;
    more = true;
    await attempt(() => nextPage(works));
    more = false;
  }

  let filtered = $derived(!!params.get("rules") || !!kind || includeAll);
  const clearAll = () => update({ rules: "", kind: "", all: "" });
</script>

<div class="page">
  {@render above?.()}
  <header class="page-head">
    <div>
      <p class="eyebrow">{eyebrow}</p>
      <h1>{title}</h1>
    </div>
    <div class="actions">
      {@render toolbar?.()}
      <Select pill value={sort} items={SORTS} label="Sort" onchange={(value) => update({ sort: value === "DATE_ADDED_DESC" ? "" : value })} />
    </div>
  </header>

  {#if !missing}
    <FilterBar {scope} {update}>
      {#snippet extra()}
        {#if kinds.length}<Select pill value={kind} items={[{ value: "", label: "Top level" }, ...kinds.map((item) => ({ value: item, label: titleCase(item) }))]} label="Type" onchange={(value) => update({ kind: value })} />{/if}
      {/snippet}
      {#snippet more()}
        {#if allowRecursive}<label class="check recursive"><input type="checkbox" checked={includeAll} onchange={(event) => update({ all: event.currentTarget.checked })} />Include episodes & tracks</label>{/if}
      {/snippet}
    </FilterBar>
  {/if}

  {#if missing}
    {@render missing()}
  {:else if error && !items.length}
    <State kind="error" title="Couldn’t open this page" message={error} action={() => refetch(works)} />
  {:else if loading && !items.length}
    <div class="poster-grid" aria-busy="true">{#each Array(18) as _}<div class="skeleton" style:aspect-ratio="2 / 3"></div>{/each}</div>
  {:else if !items.length}
    <State icon={FunnelSimple} title={filtered ? "Nothing matches these filters" : emptyTitle} message={filtered ? "Loosen a filter or two." : emptyMessage}>
      {#if filtered}<button class="btn sm" onclick={clearAll}>Clear filters</button>{/if}
    </State>
  {:else}
    <div class="poster-grid stagger" class:refreshing={loading}>
      {#each items as work, index (work.id)}<div style:--i={index % 24}><WorkCard {work} key="grid:{work.id}" /></div>{/each}
    </div>
    {#if $works.pageInfo.hasNextPage}
      <!-- A fresh observer per page: one still in view after a short page fires again. -->
      {#key items.length}<div class="sentinel" use:inview={{ callback: loadMore }}>{#if more}<span class="spinner"></span>{/if}</div>{/key}
    {:else if items.length > 24}
      <p class="end">That’s everything — {items.length} titles.</p>
    {/if}
  {/if}
</div>

<style>
  :global(.recursive) { padding: 4px 8px; }
  .refreshing { opacity: 0.55; transition: opacity var(--t); }
  .sentinel { display: grid; place-items: center; height: 120px; }
  .end { text-align: center; color: var(--text-3); font-size: 0.86rem; padding: 40px 0 0; }
</style>
