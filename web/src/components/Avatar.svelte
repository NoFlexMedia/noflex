<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { initials } from "$lib/picture.js";
  import { mediaUrl } from "$lib/tokens.js";

  /** `user` needs `name` and `avatarUrl`. Decorative: whatever's beside it says who this is. */
  let { user, size = 34 } = $props();
  // A picture that won't load falls back as if there were none.
  let failed = $state(null);
  let url = $derived(user?.avatarUrl ?? null);
</script>

<span class="avatar" aria-hidden="true" style:width="{size}px" style:height="{size}px" style:font-size="{size * 0.36}px">
  {initials(user?.name)}
  {#if url && failed !== url}<img src={mediaUrl(url)} alt="" decoding="async" onerror={() => (failed = url)} />{/if}
</span>

<style>
  .avatar { position: relative; overflow: hidden; }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
</style>
