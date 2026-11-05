<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ProviderLogo from "$components/ProviderLogo.svelte";
  import { webUrl } from "$lib/api.js";

  /** `attributions`: `Work.attributions`, the providers behind what this page shows. */
  let { attributions = [] } = $props();
</script>

{#snippet body(credit)}
  {#if credit.logoUrl}<span class="mark"><ProviderLogo {credit} /></span>{/if}
  <span>{credit.text}</span>
{/snippet}

{#if attributions.length}
  <footer class="attributions" class:aligned={attributions.length > 1}>
    {#each attributions as credit (credit.provider)}
      {#if webUrl(credit.url)}
        <a class="credit" href={webUrl(credit.url)} target="_blank" rel="noopener noreferrer">{@render body(credit)}</a>
      {:else}
        <div class="credit">{@render body(credit)}</div>
      {/if}
    {/each}
  </footer>
{/if}

<style>
  .attributions { display: grid; gap: 12px; margin-top: 56px; padding-top: 20px; border-top: 1px solid var(--line); }
  .credit { display: flex; align-items: center; gap: 14px; max-width: 68ch; color: var(--text-3); font-size: 0.8rem; line-height: 1.4; transition: color var(--t); }
  .mark { flex: none; }
  /* Wide enough for the widest mark at this height, so several credits' text lines up. */
  .aligned .mark { width: 56px; }
  a.credit:hover { color: var(--text-2); }
</style>
