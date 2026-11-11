<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { cast } from "$lib/cast.svelte.js";
</script>

<!-- Google's own button and mark: Cast's guidelines have the sender SDK draw it, and only while a device is there. -->
{#if cast.nearby}<google-cast-launcher aria-label="Cast to a device"></google-cast-launcher>{/if}

<style>
  /* The SDK draws its mark across the whole element, so the padding sets the mark's size: as big as ConnectPicker's 19px icon looks, in the same 32px button. */
  google-cast-launcher { display: inline-block; box-sizing: border-box; width: 32px; height: 32px; padding: 7px; border-radius: 999px; cursor: pointer; transition: background var(--t) var(--ease-out), transform var(--t-fast) var(--ease-out); --connected-color: var(--accent); --disconnected-color: var(--text-2); }
  google-cast-launcher:hover { background: var(--surface-2); --disconnected-color: var(--text); }
  google-cast-launcher:active { transform: scale(0.92); }
</style>
