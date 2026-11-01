<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import FunnelSimple from "phosphor-svelte/lib/FunnelSimple";
  import X from "phosphor-svelte/lib/X";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Popover } from "bits-ui";
  import { inview } from "$lib/actions/index.js";
  import { titleCase } from "$lib/format.js";
  import { graphql } from "$houdini";
  import { attempt } from "$lib/ui.svelte.js";
  import { failure, fetchQuery, nextPage, refetch } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";
  import WorkCard from "$components/WorkCard.svelte";
  import Select from "$components/ui/Select.svelte";

  let { data: route } = $props();

  const SORTS = [
    { value: "DATE_ADDED_DESC", label: "Recently added" },
    { value: "TITLE_ASC", label: "Title A–Z" },
    { value: "TITLE_DESC", label: "Title Z–A" },
    { value: "YEAR_DESC", label: "Newest release" },
    { value: "YEAR_ASC", label: "Oldest release" },
    { value: "DATE_RELEASED_DESC", label: "Release date" },
    { value: "RANDOM", label: "Shuffle" },
  ];
  const KINDS = ["MOVIE", "SERIES", "SEASON", "EPISODE", "ARTIST", "ALBUM", "TRACK", "AUDIO_BOOK", "PHOTO"];

  let library = $derived(app.libraries.find((item) => item.id === route.id));
  let params = $derived(page.url.searchParams);
  let sort = $derived(params.get("sort") ?? "DATE_ADDED_DESC");
  let filters = $derived({
    kind: params.get("kind") ?? "",
    watched: params.get("watched") ?? "",
    favorites: params.get("favorites") === "1",
    tag: params.get("tag") ?? "",
    yearMin: params.get("from") ?? "",
    yearMax: params.get("to") ?? "",
    rating: params.get("rating") ?? "",
    all: params.get("all") === "1",
  });

  const tags = graphql(`
    query LibraryTags @cache(policy: CacheOrNetwork) {
      tags(namespaces: ["genre", "mood", "studio", "person", "parental_rating"], first: 200) {
        nodes { tag { id namespace display } workCount }
      }
    }
  `);
  $effect(() => { fetchQuery(tags); });
  let tagItems = $derived(($tags.data?.tags.nodes ?? []).map((item) => ({ value: item.tag.id, label: item.tag.display, hint: `${titleCase(item.tag.namespace)} · ${item.workCount}` })));

  function update(changes) {
    const next = new URLSearchParams(page.url.searchParams);
    for (const [key, value] of Object.entries(changes)) {
      if (value === "" || value === false || value == null) next.delete(key); else next.set(key, value === true ? "1" : String(value));
    }
    goto(`?${next}`, { replaceState: true, noScroll: true, keepFocus: true });
  }

  let works = $derived(route.works);
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

  let chips = $derived([
    filters.kind && { key: "kind", label: titleCase(filters.kind) },
    filters.watched && { key: "watched", label: filters.watched === "watched" ? "Watched" : "Unwatched" },
    filters.favorites && { key: "favorites", label: "Favorites" },
    filters.tag && { key: "tag", label: tagItems.find((item) => item.value === filters.tag)?.label ?? "Tag" },
    (filters.yearMin || filters.yearMax) && { key: ["from", "to"], label: `${filters.yearMin || "…"}–${filters.yearMax || "…"}` },
    filters.rating && { key: "rating", label: `★ ${filters.rating}+` },
    filters.all && { key: "all", label: "Including episodes & tracks" },
  ].filter(Boolean));
  const clearKey = (key) => update(Object.fromEntries([key].flat().map((name) => [name, ""])));
  const clearAll = () => update({ kind: "", watched: "", favorites: "", tag: "", from: "", to: "", rating: "", all: "" });
</script>

<svelte:head><title>{library?.name ?? "Library"} · NoFlex</title></svelte:head>

<div class="page">
  <header class="page-head">
    <div>
      <p class="eyebrow">{library ? titleCase(library.kind) : "Library"}</p>
      <h1>{library?.name ?? "Library"}</h1>
    </div>
    <div class="actions">
      <Select pill value={sort} items={SORTS} label="Sort" onchange={(value) => update({ sort: value === "DATE_ADDED_DESC" ? "" : value })} />
      <Popover.Root>
        <Popover.Trigger class="btn {chips.length ? 'outline' : ''}"><FunnelSimple size={16} />Filter{#if chips.length}<span class="count">{chips.length}</span>{/if}</Popover.Trigger>
        <Popover.Portal>
          <Popover.Content class="nf-menu filters" sideOffset={8} align="end" collisionPadding={12}>
            <div class="filter-grid">
              <label class="field">Type<Select value={filters.kind} items={[{ value: "", label: "Top level" }, ...KINDS.map((kind) => ({ value: kind, label: titleCase(kind) }))]} onchange={(value) => update({ kind: value })} /></label>
              <label class="field">Watch state<Select value={filters.watched} items={[{ value: "", label: "Any" }, { value: "unwatched", label: "Unwatched" }, { value: "watched", label: "Watched" }]} onchange={(value) => update({ watched: value })} /></label>
              <label class="field full">Genre, mood, studio or person<Select value={filters.tag} items={[{ value: "", label: "Any" }, ...tagItems]} onchange={(value) => update({ tag: value })} /></label>
              <label class="field">From year<input type="number" min="1800" max="2200" value={filters.yearMin} placeholder="Any" onchange={(event) => update({ from: event.currentTarget.value })} /></label>
              <label class="field">To year<input type="number" min="1800" max="2200" value={filters.yearMax} placeholder="Any" onchange={(event) => update({ to: event.currentTarget.value })} /></label>
              <label class="field">Minimum rating<input type="number" min="0" max="10" step="0.5" value={filters.rating} placeholder="Any" onchange={(event) => update({ rating: event.currentTarget.value })} /></label>
              <div class="toggles full">
                <label class="check"><input type="checkbox" checked={filters.favorites} onchange={(event) => update({ favorites: event.currentTarget.checked })} />Favorites only</label>
                <label class="check"><input type="checkbox" checked={filters.all} onchange={(event) => update({ all: event.currentTarget.checked })} />Include episodes & tracks</label>
              </div>
            </div>
            {#if chips.length}<div class="form-actions"><button class="btn ghost sm" onclick={clearAll}>Reset filters</button></div>{/if}
          </Popover.Content>
        </Popover.Portal>
      </Popover.Root>
    </div>
  </header>

  {#if chips.length}
    <div class="chips">
      {#each chips as chip (chip.label)}<button class="chip outline" onclick={() => clearKey(chip.key)}>{chip.label}<X size={13} /></button>{/each}
      <button class="btn ghost sm" onclick={clearAll}>Clear all</button>
    </div>
  {/if}

  {#if !library}
    <State icon={Books} title="Library not found" message="It may have been removed, or you no longer have access."><a class="btn sm" href="/libraries">All libraries</a></State>
  {:else if error && !items.length}
    <State kind="error" title="Couldn’t open this library" message={error} action={() => refetch(works)} />
  {:else if loading && !items.length}
    <div class="poster-grid" aria-busy="true">{#each Array(18) as _}<div class="skeleton" style:aspect-ratio="2 / 3"></div>{/each}</div>
  {:else if !items.length}
    <State icon={FunnelSimple} title={chips.length ? "Nothing matches these filters" : "This library is empty"} message={chips.length ? "Loosen a filter or two." : app.viewer.isAdmin ? "Run a scan from Administration once media is in place." : "Check back after the next scan."}>
      {#if chips.length}<button class="btn sm" onclick={clearAll}>Clear filters</button>{/if}
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
  .actions :global(.count) { display: inline-grid; place-items: center; min-width: 20px; height: 20px; padding: 0 6px; border-radius: 99px; background: var(--accent); color: var(--accent-ink); font-size: 0.72rem; font-weight: 750; }
  :global(.filters) { width: min(440px, calc(100vw - 24px)); padding: 18px !important; }
  .filter-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .filter-grid .full { grid-column: 1 / -1; }
  .toggles { display: flex; flex-wrap: wrap; gap: 10px 20px; padding-top: 4px; }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: -10px 0 22px; }
  .chips .chip { height: 30px; cursor: pointer; transition: border-color var(--t), color var(--t); }
  .chips .chip:hover { border-color: var(--text-3); color: var(--text); }
  .refreshing { opacity: 0.55; transition: opacity var(--t); }
  .sentinel { display: grid; place-items: center; height: 120px; }
  .end { text-align: center; color: var(--text-3); font-size: 0.86rem; padding: 40px 0 0; }
</style>
