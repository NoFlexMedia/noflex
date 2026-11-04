<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import Segmented from "$components/ui/Segmented.svelte";
  import WorkBrowser from "$components/WorkBrowser.svelte";

  let { data: route } = $props();

  const ANIME_KINDS = [{ value: "", label: "All" }, { value: "SERIES", label: "Series" }, { value: "MOVIE", label: "Movies" }];
  const TYPE_FILTERS = { MUSIC: ["ARTIST", "ALBUM", "TRACK"] };

  function showKind(kind) {
    const next = new URLSearchParams(page.url.searchParams);
    if (kind) next.set("kind", kind); else next.delete("kind");
    goto(`?${next}`, { replaceState: true, noScroll: true, keepFocus: true });
  }
</script>

<svelte:head><title>{route.category.label} · NoFlex</title></svelte:head>

<WorkBrowser works={route.works} title={route.category.label} eyebrow="Browse" kinds={TYPE_FILTERS[route.category.value] ?? []} allowRecursive={false} emptyTitle="Nothing here yet" emptyMessage="Check back after the next scan.">
  {#snippet toolbar()}
    {#if route.category.value === "ANIME"}<Segmented value={page.url.searchParams.get("kind") ?? ""} items={ANIME_KINDS} label="Anime type" onchange={showKind} />{/if}
  {/snippet}
</WorkBrowser>
