<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";

  let { kind = "empty", icon: Glyph = null, title, message = null, action = null, actionLabel = "Try again", compact = false, children = null } = $props();
</script>

<div class="empty-state fade-up" class:compact role={kind === "error" ? "alert" : "status"}>
  {#if kind === "loading"}
    <span class="spinner"></span>
  {:else}
    <span class="glyph" class:error={kind === "error"}>{#if Glyph}<Glyph size={24} />{:else if kind === "error"}<WarningCircle size={24} />{:else}<Sparkle size={24} />{/if}</span>
  {/if}
  <h2>{title}</h2>
  {#if message}<p>{message}</p>{/if}
  {#if action || children}
    <div class="row">
      {#if action}<button class="btn sm" type="button" onclick={action}>{actionLabel}</button>{/if}
      {@render children?.()}
    </div>
  {/if}
</div>

<style>
  .empty-state { display: grid; justify-items: center; text-align: center; gap: 10px; padding: clamp(40px, 9vh, 96px) 20px; max-width: 460px; margin: 0 auto; }
  .empty-state.compact { padding: 28px 20px; gap: 8px; }
  .glyph { display: grid; place-items: center; width: 56px; height: 56px; border-radius: 18px; background: var(--surface-2); color: var(--text-2); margin-bottom: 6px; box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.06); }
  .empty-state:not(.compact) .glyph { animation: glyph-in 620ms var(--ease-spring) both; }
  @keyframes glyph-in { from { transform: scale(0.6) rotate(-8deg); opacity: 0; } }
  .glyph.error { background: var(--danger-soft); color: var(--danger); }
  h2 { font-size: 1.12rem; font-weight: 650; }
  .compact h2 { font-size: 1rem; }
  p { color: var(--text-2); font-size: 0.92rem; font-weight: 400; max-width: 52ch; }
  .row { margin-top: 8px; justify-content: center; }
</style>
