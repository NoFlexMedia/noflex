<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { mediaUrl } from "$lib/tokens.js";

  /** `credit` is a `ProviderAttribution`, with a `logoUrl`. Providers' terms forbid recolouring these, so no filters. */
  let { credit, height = 24 } = $props();
</script>

<img src={mediaUrl(credit.logoUrl)} alt={credit.name} loading="lazy" decoding="async" style:height="{height}px" />

<style>
  img { display: block; flex: none; width: auto; max-width: 100%; }
</style>
