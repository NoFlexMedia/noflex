<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { DropdownMenu } from "bits-ui";

  let { icon: Glyph = null, danger = false, disabled = false, href = null, onSelect = () => {}, hint = null, children } = $props();
</script>

{#if href}
  <DropdownMenu.Item class="nf-menu-item" {disabled}>
    {#snippet child({ props })}
      <a {...props} {href} class:danger>{#if Glyph}<Glyph size={17} />{/if}<span>{@render children()}</span>{#if hint}<kbd>{hint}</kbd>{/if}</a>
    {/snippet}
  </DropdownMenu.Item>
{:else}
  <DropdownMenu.Item class="nf-menu-item {danger ? 'danger' : ''}" {disabled} {onSelect}>
    {#if Glyph}<Glyph size={17} />{/if}<span>{@render children()}</span>{#if hint}<kbd>{hint}</kbd>{/if}
  </DropdownMenu.Item>
{/if}
