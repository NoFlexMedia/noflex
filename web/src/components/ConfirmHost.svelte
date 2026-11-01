<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { confirmState } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";

  let option = $state(false);
  let open = $state(false);
  // Keep the last request rendered while the dialog animates out.
  let shown = $state(null);
  $effect(() => {
    const request = confirmState.current;
    if (request) { shown = request; option = false; open = true; }
  });

  function settle(confirmed) {
    const current = confirmState.current;
    confirmState.current = null;
    open = false;
    current?.resolve({ confirmed, option });
  }
</script>

<Dialog bind:open title={shown?.title ?? ""} description={shown?.message} width={440} onclose={() => settle(false)}>
  {#if shown?.option}<label class="check"><input type="checkbox" bind:checked={option} /> {shown.option}</label>{/if}
  {#snippet footer()}
    <button class="btn ghost" onclick={() => settle(false)}>Cancel</button>
    <button class="btn {shown?.danger ? 'danger' : 'primary'}" onclick={() => settle(true)}>{shown?.confirmLabel ?? "Confirm"}</button>
  {/snippet}
</Dialog>
