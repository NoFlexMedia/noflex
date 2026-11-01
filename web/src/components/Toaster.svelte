<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import Info from "phosphor-svelte/lib/Info";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import X from "phosphor-svelte/lib/X";
  import { flip } from "svelte/animate";
  import { fly } from "svelte/transition";
  import { backOut, cubicOut } from "svelte/easing";
  import { dismissToast, toasts } from "$lib/ui.svelte.js";

  const ICON = { info: Info, success: CheckCircle, error: WarningCircle };
</script>

<section class="toaster" aria-live="polite" aria-label="Notifications">
  {#each toasts as item (item.id)}
    {@const Glyph = ICON[item.kind]}
    <div class="toast {item.kind}" role={item.kind === "error" ? "alert" : "status"} animate:flip={{ duration: 260, easing: cubicOut }} in:fly={{ y: 24, duration: 380, easing: backOut }} out:fly={{ y: 10, duration: 180, opacity: 0 }}>
      <Glyph size={18} />
      <p>{item.message}</p>
      {#if item.action}<button class="btn sm ghost" onclick={() => { item.action.run(); dismissToast(item.id); }}>{item.action.label}</button>{/if}
      <button class="icon-btn sm" aria-label="Dismiss" onclick={() => dismissToast(item.id)}><X size={15} /></button>
      <span class="timer"></span>
    </div>
  {/each}
</section>

<style>
  .toaster { position: fixed; z-index: 100; left: 50%; bottom: max(20px, env(safe-area-inset-bottom)); translate: -50% 0; display: grid; gap: 8px; width: min(440px, calc(100vw - 24px)); pointer-events: none; }
  .toast { position: relative; overflow: hidden; display: flex; align-items: center; gap: 12px; padding: 10px 8px 10px 16px; border-radius: 16px; background: rgb(26 28 34 / 0.9); backdrop-filter: blur(20px) saturate(1.6); border: 1px solid var(--line-strong); box-shadow: var(--shadow-3); pointer-events: auto; }
  .toast > :global(svg) { flex: none; color: var(--text-2); }
  .success > :global(svg) { color: var(--success); }
  .error > :global(svg) { color: var(--danger); }
  p { flex: 1; font-size: 0.9rem; line-height: 1.35; }
  .timer { position: absolute; left: 0; bottom: 0; height: 2px; width: 100%; background: currentColor; opacity: 0.18; transform-origin: left; animation: drain 4.2s linear forwards; }
  .error .timer { animation-duration: 7s; }
  .toast:hover .timer { animation-play-state: paused; }
  @keyframes drain { to { transform: scaleX(0); } }
  @media (max-width: 760px) { .toaster { bottom: calc(76px + env(safe-area-inset-bottom)); } }
</style>
