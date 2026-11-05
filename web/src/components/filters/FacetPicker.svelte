<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import { valueLabel } from "$lib/filterFields.js";

  /**
   * The values one field can take, as a checklist. `options` is what the server
   * listed for it: [{ value, label, workCount }]. Values already chosen stay
   * listed even when this server doesn't have them, so they can be unchecked.
   */
  let { field, options = [], truncated = false, loading = false, selected = [], onchange } = $props();

  let search = $state("");
  let rows = $derived.by(() => {
    const listed = field.fixed ? field.fixed.map((option) => ({ value: option.value, label: option.label })) : options.map((option) => ({ ...option, label: valueLabel(field, option.value, option.label) }));
    const missing = selected.filter((value) => !listed.some((row) => row.value === value)).map((value) => ({ value, label: valueLabel(field, value) }));
    return [...missing, ...listed];
  });
  let shown = $derived.by(() => {
    const words = search.trim().toLowerCase();
    return words ? rows.filter((row) => row.label.toLowerCase().includes(words)) : rows;
  });

  const toggle = (value) => onchange(selected.includes(value) ? selected.filter((item) => item !== value) : [...selected, value]);
</script>

<div class="picker">
  {#if rows.length > 8}
    <label class="search">
      <MagnifyingGlass size={15} />
      <!-- svelte-ignore a11y_autofocus -->
      <input type="search" bind:value={search} placeholder="Find a {field.label.toLowerCase()}" aria-label="Find a {field.label.toLowerCase()}" autofocus />
    </label>
  {/if}
  {#if loading && !rows.length}
    <div class="rows" aria-busy="true">{#each Array(4) as _}<div class="skeleton row-skel"></div>{/each}</div>
  {:else if !rows.length}
    <p class="none">Nothing here has one yet.</p>
  {:else}
    <div class="rows">
      {#each shown as row (row.value)}
        <label class="row check">
          <input type="checkbox" checked={selected.includes(row.value)} onchange={() => toggle(row.value)} />
          <span class="name">{row.label}</span>
          {#if row.workCount != null}<small class="tabular">{row.workCount}</small>{/if}
        </label>
      {:else}
        <p class="none">Nothing matches “{search.trim()}”.</p>
      {/each}
    </div>
    {#if truncated && !search}<p class="note">The most common are listed. Search narrows them down.</p>{/if}
  {/if}
</div>

<style>
  .picker { display: grid; gap: 6px; }
  .search { display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 10px; border: 1px solid var(--line); border-radius: 10px; color: var(--text-3); transition: border-color var(--t-fast); }
  .search:focus-within { border-color: var(--line-strong); }
  .search input { flex: 1; min-width: 0; border: 0; outline: 0; background: none; color: var(--text); font: inherit; font-size: 0.86rem; }
  .rows { display: grid; gap: 1px; max-height: min(320px, 50vh); overflow-y: auto; overscroll-behavior: contain; margin-inline: -4px; padding-inline: 4px; }
  .row { display: flex; align-items: center; width: 100%; min-height: 34px; padding: 4px 8px; border-radius: 9px; transition: background var(--t-fast); }
  .row:hover { background: var(--surface-2); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  small { color: var(--text-3); font-size: 0.76rem; }
  .row-skel { height: 28px; border-radius: 8px; margin-block: 3px; }
  .none, .note { padding: 8px; font-size: 0.84rem; color: var(--text-3); }
  .note { padding-block: 4px 0; font-size: 0.76rem; }
</style>
