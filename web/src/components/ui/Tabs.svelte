<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { Tabs } from "bits-ui";

  /** `items`: [{ value, label, count? }]. Panels go in `children` as <Tabs.Content>. */
  let { value = $bindable(), items = [], onchange = null, label, children = null } = $props();
  let list = $state();
  let indicator = $state({ x: 0, w: 0, ready: false });
  // Which edges have tabs scrolled out past them, to fade those edges.
  let edges = $state({ start: false, end: false });
  const measure = () => {
    if (!list) return;
    const max = list.scrollWidth - list.clientWidth;
    edges = { start: list.scrollLeft > 2, end: max - list.scrollLeft > 2 };
  };

  $effect(() => {
    value; items.length;
    if (!list) return;
    const place = () => {
      const active = list.querySelector('[data-state="active"]');
      if (!active) return;
      const first = !indicator.ready;
      indicator = { x: active.offsetLeft, w: active.offsetWidth, ready: true };
      // On narrow screens the strip scrolls; keep the chosen tab in view.
      if (list.scrollWidth > list.clientWidth) list.scrollTo({ left: active.offsetLeft - (list.clientWidth - active.offsetWidth) / 2, behavior: first ? "instant" : "smooth" });
      measure();
    };
    requestAnimationFrame(place);
    const observer = new ResizeObserver(place);
    observer.observe(list);
    return () => observer.disconnect();
  });
</script>

<Tabs.Root bind:value onValueChange={(next) => onchange?.(next)}>
  <Tabs.List class="tabs" aria-label={label}>
    {#snippet child({ props })}
      <div {...props} bind:this={list} class="{props.class} {edges.start ? 'fade-start' : ''} {edges.end ? 'fade-end' : ''}" onscroll={measure}>
        <span class="indicator" class:ready={indicator.ready} style:width="{indicator.w}px" style:transform="translateX({indicator.x}px)"></span>
        {#each items as item (item.value)}
          <Tabs.Trigger value={item.value}>{item.label}{#if item.count != null}<span class="count">{item.count}</span>{/if}</Tabs.Trigger>
        {/each}
      </div>
    {/snippet}
  </Tabs.List>
  {@render children?.()}
</Tabs.Root>
