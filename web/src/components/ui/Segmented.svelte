<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { ToggleGroup } from "bits-ui";

  /**
   * Single-choice pill switch with a sliding thumb. `items`:
   * [{ value, label, icon?, hint? }]; an item with `icon` and no `label`
   * renders icon-only and uses `hint` as its accessible name.
   */
  let { value = $bindable(), items = [], label, size = "md", onchange = null } = $props();
  let root = $state();
  let thumb = $state({ x: 0, w: 0, ready: false });

  $effect(() => {
    value; items.length;
    if (!root) return;
    const place = () => {
      const active = root.querySelector('[data-state="on"]');
      if (active) thumb = { x: active.offsetLeft, w: active.offsetWidth, ready: true };
    };
    requestAnimationFrame(place);
    const observer = new ResizeObserver(place);
    observer.observe(root);
    return () => observer.disconnect();
  });

  function pick(next) {
    // A single toggle group deselects on a second click; a switch never empties.
    if (!next || next === value) return;
    value = next;
    onchange?.(next);
  }
</script>

<ToggleGroup.Root type="single" value={value} onValueChange={pick} aria-label={label}>
  {#snippet child({ props })}
    <div {...props} class="seg {size}" bind:this={root}>
      <span class="thumb" class:ready={thumb.ready} style:width="{thumb.w}px" style:transform="translateX({thumb.x}px)" aria-hidden="true"></span>
      {#each items as item (item.value)}
        <ToggleGroup.Item value={item.value} class="seg-item" aria-label={item.label ? undefined : item.hint} title={item.hint ?? undefined}>
          {#if item.icon}<item.icon size={size === "sm" ? 15 : 16} />{/if}
          {#if item.label}<span>{item.label}</span>{/if}
        </ToggleGroup.Item>
      {/each}
    </div>
  {/snippet}
</ToggleGroup.Root>

<style>
  .seg { position: relative; display: inline-flex; padding: 3px; border-radius: 999px; background: var(--surface); border: 1px solid var(--line); max-width: 100%; overflow-x: auto; scrollbar-width: none; flex: none; }
  .thumb { position: absolute; top: 3px; bottom: 3px; left: 0; border-radius: 999px; background: var(--surface-3); box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.07), var(--shadow-1); transition: transform var(--t-slow) var(--ease-spring), width var(--t-slow) var(--ease-spring); }
  .thumb:not(.ready) { opacity: 0; transition: none; }
  .seg :global(.seg-item) { position: relative; z-index: 1; display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 13px; border-radius: 999px; font-size: 0.85rem; font-weight: 600; color: var(--text-3); white-space: nowrap; transition: color var(--t) var(--ease-out), transform var(--t-fast) var(--ease-out); }
  .seg.sm :global(.seg-item) { height: 28px; padding: 0 10px; font-size: 0.8rem; }
  .seg :global(.seg-item:hover) { color: var(--text-2); }
  .seg :global(.seg-item:active) { transform: scale(0.95); }
  .seg :global(.seg-item[data-state="on"]) { color: var(--text); }
</style>
