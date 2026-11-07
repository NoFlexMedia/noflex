<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { cast } from "$lib/cast.svelte.js";
</script>

<!-- Google's own button and mark: Cast's guidelines have the sender SDK draw it, and only while a device is there. -->
{#if cast.nearby}<google-cast-launcher aria-label="Cast to a device"></google-cast-launcher>{/if}

<style>
  google-cast-launcher { display: inline-block; width: 32px; height: 32px; cursor: pointer; --connected-color: var(--accent); --disconnected-color: var(--text-2); }
  google-cast-launcher:hover { --disconnected-color: var(--text); }
</style>
