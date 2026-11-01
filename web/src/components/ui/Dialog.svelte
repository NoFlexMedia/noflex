<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script module>
  // Transition factories keyed off the node's variant class.
  function variant_in(node) {
    return node.classList.contains("sheet")
      ? fly(node, { x: 48, duration: 360, easing: cubicOut, opacity: 0 })
      : scale(node, { start: 0.96, duration: 280, easing: cubicOut, opacity: 0 });
  }
  function variant_out(node) {
    return node.classList.contains("sheet")
      ? fly(node, { x: 48, duration: 220, easing: cubicOut, opacity: 0 })
      : scale(node, { start: 0.97, duration: 160, easing: cubicOut, opacity: 0 });
  }
</script>

<script>
  import X from "phosphor-svelte/lib/X";
  import { Dialog } from "bits-ui";
  import { fade, fly, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  /** `variant`: "center" (default) or "sheet" (slides in from the right). */
  let { open = $bindable(false), title, description = null, variant = "center", width = 520, onclose = null, children, footer = null } = $props();

  function change(next) {
    open = next;
    if (!next) onclose?.();
  }
</script>

<Dialog.Root {open} onOpenChange={change}>
  <Dialog.Portal>
    <Dialog.Overlay forceMount>
      {#snippet child({ props, open })}
        {#if open}<div {...props} class="overlay" transition:fade={{ duration: 200 }}></div>{/if}
      {/snippet}
    </Dialog.Overlay>
    <Dialog.Content forceMount>
      {#snippet child({ props, open })}
        {#if open}
          <div {...props} class="dialog {variant}" style:--w="{width}px" in:variant_in out:variant_out>
            <header>
              <div>
                <Dialog.Title class="title">{title}</Dialog.Title>
                {#if description}<Dialog.Description class="description">{description}</Dialog.Description>{/if}
              </div>
              <Dialog.Close class="icon-btn sm" aria-label="Close"><X size={18} /></Dialog.Close>
            </header>
            <div class="body">{@render children?.()}</div>
            {#if footer}<footer>{@render footer()}</footer>{/if}
          </div>
        {/if}
      {/snippet}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>


<style>
  .overlay { position: fixed; inset: 0; z-index: 80; background: rgb(3 4 6 / 0.7); backdrop-filter: blur(8px); }
  .dialog {
    position: fixed;
    z-index: 81;
    display: flex;
    flex-direction: column;
    background: linear-gradient(180deg, rgb(255 255 255 / 0.03), transparent 120px), #111318;
    border: 1px solid var(--line-strong);
    box-shadow: var(--shadow-3);
    outline: none;
  }
  .center { left: 50%; top: 50%; translate: -50% -50%; width: min(var(--w), calc(100vw - 32px)); max-height: min(86dvh, 860px); border-radius: var(--r-xl); }
  .sheet { top: 10px; right: 10px; bottom: 10px; width: min(var(--w), calc(100vw - 20px)); border-radius: var(--r-xl); }
  header { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; padding: 22px 22px 6px 26px; }
  header :global(.title) { font-size: 1.22rem; font-weight: 650; letter-spacing: -0.02em; }
  header :global(.description) { color: var(--text-2); font-size: 0.9rem; margin-top: 6px; }
  .body { padding: 14px 26px 24px; overflow-y: auto; overscroll-behavior: contain; }
  footer { display: flex; justify-content: flex-end; gap: 10px; padding: 16px 22px; border-top: 1px solid var(--line); }
</style>
