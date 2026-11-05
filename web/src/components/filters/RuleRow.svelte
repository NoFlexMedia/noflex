<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CaretDown from "phosphor-svelte/lib/CaretDown";
  import Minus from "phosphor-svelte/lib/Minus";
  import { Popover } from "bits-ui";
  import { slide } from "svelte/transition";
  import { FIELDS, fieldOf, operatorOf, valueLabel } from "$lib/filterFields.js";
  import { prefersReducedMotion } from "$lib/ui.svelte.js";
  import Select from "$components/ui/Select.svelte";
  import FacetPicker from "./FacetPicker.svelte";

  /** One condition: field, operator, value. `editor` owns the tree. */
  let { rule, editor, facets, loading } = $props();

  const FIELD_ITEMS = FIELDS.map((field) => ({ value: field.facet, label: field.label, group: field.section }));

  let field = $derived(fieldOf(rule.facet));
  let operator = $derived(operatorOf(field, rule.op));
  let listed = $derived(facets.get(rule.facet));
  let known = $derived(new Map((listed?.values ?? []).map((option) => [option.value, option.label])));
  let names = $derived(rule.inputs.map((value) => valueLabel(field, value, known.get(value))));
  let summary = $derived(names.length > 2 ? `${names.slice(0, 2).join(", ")} +${names.length - 2}` : names.join(", "));
  let hint = $derived(operator.hint && rule.inputs.length > 1 ? operator.hint : field.hint);
  let motion = $derived(prefersReducedMotion() ? 0 : 200);

  const pickItems = (current) => current.operators.map((item) => ({ value: item.id, label: item.label, hint: item.hint }));
</script>

<div class="rule" transition:slide={{ duration: motion }}>
  <div class="cells">
    <span class="cell"><Select value={rule.facet} items={FIELD_ITEMS} label="What to filter by" onchange={(facet) => editor.setField(rule, facet)} /></span>
    <span class="cell"><Select value={rule.op} items={pickItems(field)} label="How to compare" onchange={(op) => editor.setOperator(rule, op)} /></span>
    <span class="cell value">
      {#if operator.input === "pick"}
        <Popover.Root>
          <Popover.Trigger class="nf-select value-trigger" aria-label="Choose {field.label.toLowerCase()} values">
            <span class:placeholder={!names.length}>{summary || "Choose…"}</span><CaretDown size={16} />
          </Popover.Trigger>
          <Popover.Portal>
            <Popover.Content class="nf-menu picker-pop" sideOffset={6} align="start" collisionPadding={12}>
              <FacetPicker {field} options={listed?.values} truncated={listed?.truncated} {loading} selected={rule.inputs} onchange={(values) => editor.setInputs(rule, values)} />
            </Popover.Content>
          </Popover.Portal>
        </Popover.Root>
      {:else if operator.input === "text"}
        <input class="input" value={rule.inputs[0] ?? ""} placeholder="Words in the title" aria-label="Words in the title" onchange={(event) => editor.setInputs(rule, [event.currentTarget.value])} />
      {:else if operator.input === "number"}
        <input class="input" type="number" min="0" max="10" step="0.5" value={rule.inputs[0] ?? ""} placeholder="0–10" aria-label="Rating out of 10" onchange={(event) => editor.setInputs(rule, [event.currentTarget.value])} />
      {:else if operator.input === "days"}
        <span class="with-unit"><input class="input" type="number" min="1" value={rule.inputs[0] ?? ""} placeholder="30" aria-label="Number of days" onchange={(event) => editor.setInputs(rule, [event.currentTarget.value])} /><small>days</small></span>
      {:else if operator.input === "year"}
        <input class="input" type="number" min="1800" max="2200" value={rule.inputs[0] ?? ""} placeholder="Year" aria-label="Year" onchange={(event) => editor.setInputs(rule, [event.currentTarget.value])} />
      {:else if operator.input === "years"}
        <span class="with-unit">
          <input class="input" type="number" min="1800" max="2200" value={rule.inputs[0] ?? ""} placeholder="From" aria-label="From year" onchange={(event) => editor.setInputs(rule, [event.currentTarget.value, rule.inputs[1] ?? ""])} />
          <small>to</small>
          <input class="input" type="number" min="1800" max="2200" value={rule.inputs[1] ?? ""} placeholder="To" aria-label="To year" onchange={(event) => editor.setInputs(rule, [rule.inputs[0] ?? "", event.currentTarget.value])} />
        </span>
      {/if}
    </span>
    <button class="icon-btn sm remove" aria-label="Remove this condition" title="Remove this condition" onclick={() => editor.remove(rule)}><Minus size={16} /></button>
  </div>
  {#if hint}<p class="hint">{hint}</p>{/if}
</div>

<style>
  .rule { display: grid; gap: 3px; min-width: 0; }
  .cells { display: grid; grid-template-columns: minmax(130px, 1fr) minmax(120px, 0.8fr) minmax(150px, 1.5fr) 32px; align-items: center; gap: 8px; }
  .cell { min-width: 0; }
  .cell :global(.nf-select) { width: 100%; }
  .cell.value:empty { min-height: 0; }
  .value .input { width: 100%; }
  .with-unit { display: flex; align-items: center; gap: 8px; color: var(--text-3); font-size: 0.84rem; }
  .with-unit .input { flex: 1; min-width: 0; }
  .value :global(.value-trigger) { display: inline-flex; align-items: center; justify-content: space-between; gap: 8px; min-width: 0; text-align: left; }
  .value :global(.value-trigger span) { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .placeholder { color: var(--text-3); }
  :global(.picker-pop) { width: min(300px, calc(100vw - 24px)); padding: 10px !important; }
  .remove { color: var(--text-3); }
  .remove:hover { color: var(--danger); }
  .hint { padding-left: 2px; font-size: 0.76rem; color: var(--text-3); }
  @media (max-width: 720px) {
    .cells { grid-template-columns: 1fr 1fr 32px; }
    .cell.value { grid-column: 1 / -1; grid-row: 2; }
  }
</style>
