<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CategoryGrid from "$components/CategoryGrid.svelte";
  import CategoryTabs from "$components/CategoryTabs.svelte";

  let { data: route } = $props();
</script>

<svelte:head><title>All {route.category.label} · NoFlex</title></svelte:head>

<CategoryGrid category={route.category} works={route.works}>
  {#snippet above()}
    {#if route.category.shelves}<CategoryTabs category={route.category} current="all" />{/if}
  {/snippet}
</CategoryGrid>
