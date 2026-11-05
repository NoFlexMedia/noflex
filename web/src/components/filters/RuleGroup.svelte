<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Plus from "phosphor-svelte/lib/Plus";
  import Stack from "phosphor-svelte/lib/Stack";
  import Trash from "phosphor-svelte/lib/Trash";
  import { flip } from "svelte/animate";
  import { fade, slide } from "svelte/transition";
  import { prefersReducedMotion } from "$lib/ui.svelte.js";
  import Select from "$components/ui/Select.svelte";
  import RuleGroup from "./RuleGroup.svelte";
  import RuleRow from "./RuleRow.svelte";

  /**
   * One group of conditions. `editor` owns the tree: every change goes through it.
   * `depth` is 0 for the outermost; `facets` is what `RuleRow` offers.
   */
  let { group, editor, depth = 0, facets, loading } = $props();

  const MAX_DEPTH = 3;
  const MODES = [
    { value: "ALL", label: "Match all of these", hint: "Every condition has to be true" },
    { value: "ANY", label: "Match any of these", hint: "One true condition is enough" },
    { value: "NONE", label: "Match none of these", hint: "Leaves out anything that fits" },
  ];
  const WORD = { ALL: "and", ANY: "or", NONE: "nor" };
  const CONNECTOR_HINT = { ALL: "Every condition has to match. Select to need just one.", ANY: "One matching condition is enough. Select to need them all." };

  let motion = $derived(prefersReducedMotion() ? 0 : 220);
</script>

<div class="group" class:nested={depth > 0} transition:slide={{ duration: motion }}>
  <header>
    <span class="mode"><Select value={group.match} items={MODES} label="How conditions combine" onchange={(value) => editor.setMatch(group, value)} /></span>
    <button class="btn ghost sm" onclick={() => editor.addRule(group)}><Plus size={15} />Condition</button>
    {#if depth < MAX_DEPTH}<button class="btn ghost sm" title="A group lets one part follow its own rule, like “any of” inside “all of”." onclick={() => editor.addGroup(group)}><Stack size={15} />Group</button>{/if}
    {#if depth > 0}
      <span class="spacer"></span>
      <button class="icon-btn sm" aria-label="Remove this group" title="Remove this group" onclick={() => editor.remove(group)}><Trash size={15} /></button>
    {/if}
  </header>

  <div class="items">
    {#each group.items as item, index (item.key)}
      <div class="item" animate:flip={{ duration: motion }}>
        {#if index > 0}
          {#if group.match === "NONE"}
            <span class="connector still" in:fade={{ duration: motion }}>{WORD.NONE}</span>
          {:else}
            <button class="connector" class:any={group.match === "ANY"} title={CONNECTOR_HINT[group.match]} aria-label="Conditions combine with {WORD[group.match]}. Switch." onclick={() => editor.setMatch(group, group.match === "ALL" ? "ANY" : "ALL")}>
              {#key group.match}<span class="word" in:fade={{ duration: motion / 2 }}>{WORD[group.match]}</span>{/key}
            </button>
          {/if}
        {/if}
        {#if item.kind === "group"}
          <RuleGroup group={item} {editor} depth={depth + 1} {facets} {loading} />
        {:else}
          <RuleRow rule={item} {editor} {facets} {loading} />
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .group { display: grid; gap: 10px; min-width: 0; }
  .group.nested { padding: 12px 12px 14px; border: 1px solid var(--line); border-radius: var(--r); background: var(--surface); }
  header { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .mode { flex: 0 1 230px; min-width: 190px; margin-right: 4px; }
  .items { display: grid; gap: 8px; padding-left: 14px; border-left: 2px solid var(--line); }
  .items:empty { display: none; }
  .item { position: relative; display: grid; gap: 8px; min-width: 0; }
  .connector { justify-self: start; display: inline-grid; place-items: center; min-width: 38px; height: 22px; margin-left: -28px; padding: 0 8px; border: 1px solid var(--line-strong); border-radius: 99px; background: var(--bg, #0b0c10); color: var(--text-2); font-size: 0.7rem; font-weight: 700; letter-spacing: 0.04em; transition: transform var(--t) var(--ease-spring), color var(--t-fast), border-color var(--t-fast), background var(--t-fast); }
  button.connector:hover { transform: scale(1.1); border-color: var(--text-3); color: var(--text); }
  button.connector:active { transform: scale(0.94); }
  .connector.any { border-color: rgb(212 255 90 / 0.4); color: var(--accent); }
  .connector.still { color: var(--text-3); }
  .word { grid-area: 1 / 1; }
  @media (prefers-reduced-motion: reduce) { .connector { transition: none; } }
  @media (max-width: 560px) { .mode { flex-basis: 100%; margin-right: 0; } .items { padding-left: 10px; } .connector { margin-left: -24px; } }
</style>
