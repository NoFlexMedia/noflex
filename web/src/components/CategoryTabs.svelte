<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { goto } from "$app/navigation";
  import Segmented from "$components/ui/Segmented.svelte";

  /** Switches a category between its shelves and its full grid. `current`: "recommended" or "all". */
  let { category, current } = $props();

  const ITEMS = [{ value: "recommended", label: "Recommended" }, { value: "all", label: "Browse all" }];
  const open = (view) => goto(view === "all" ? `/browse/${category.slug}/all` : `/browse/${category.slug}`, { noScroll: true });
</script>

<div class="tabs"><Segmented value={current} items={ITEMS} label="{category.label} views" onchange={open} /></div>

<style>
  .tabs { margin-bottom: 18px; }
</style>
