<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CaretDown from "phosphor-svelte/lib/CaretDown";
  import Check from "phosphor-svelte/lib/Check";
  import { Select } from "bits-ui";

  /**
   * Single-value listbox. `items`: [{ value, label, hint?, disabled? }].
   * `name` renders a hidden input so it participates in <form> submits.
   */
  let { value = $bindable(""), items = [], placeholder = "Select…", label = null, name = undefined, disabled = false, pill = false, onchange = null, id = undefined } = $props();
  let selected = $derived(items.find((item) => item.value === value));
</script>

<Select.Root type="single" bind:value {items} {name} {disabled} onValueChange={(next) => onchange?.(next)}>
  <Select.Trigger class="nf-select {pill ? 'pill' : ''}" aria-label={label ?? placeholder} {id}>
    <span class:placeholder={!selected}>{selected?.label ?? placeholder}</span>
    <CaretDown size={16} />
  </Select.Trigger>
  <Select.Portal>
    <Select.Content class="nf-menu nf-listbox" sideOffset={6} collisionPadding={12}>
      <Select.Viewport>
        {#each items as item (item.value)}
          <Select.Item class="nf-menu-item" value={item.value} label={item.label} disabled={item.disabled}>
            {#snippet children({ selected })}
              <span>{item.label}</span>
              {#if item.hint}<small>{item.hint}</small>{/if}
              <span class="tick" class:on={selected}><Check size={15} weight="bold" /></span>
            {/snippet}
          </Select.Item>
        {/each}
      </Select.Viewport>
    </Select.Content>
  </Select.Portal>
</Select.Root>
