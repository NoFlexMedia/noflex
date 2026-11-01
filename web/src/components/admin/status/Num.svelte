<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { Tween } from "svelte/motion";
  import { cubicOut } from "svelte/easing";
  import { prefersReducedMotion } from "$lib/ui.svelte.js";

  /** A number that eases between readings instead of jumping; `format` renders it. */
  let { value, format = (n) => String(Math.round(n)) } = $props();
  const tween = new Tween(0, { duration: 0 });
  let first = true;
  $effect(() => {
    const next = Number(value ?? 0);
    tween.set(next, { duration: first || prefersReducedMotion() ? 0 : 480, easing: cubicOut });
    first = false;
  });
</script>

{#if value == null}—{:else}{format(tween.current)}{/if}
