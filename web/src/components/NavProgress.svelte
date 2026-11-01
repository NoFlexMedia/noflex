<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { untrack } from "svelte";
  import { navigating } from "$app/state";

  // Navigations that paint from the cache finish within a frame or two;
  // the bar only appears for ones that actually wait on the network.
  const DELAY = 140;
  let phase = $state("idle");
  let progress = $state(0);

  $effect(() => {
    if (!navigating.to) {
      if (untrack(() => phase) === "running") {
        phase = "done";
        progress = 1;
        const timer = setTimeout(() => { phase = "idle"; progress = 0; }, 360);
        return () => clearTimeout(timer);
      }
      return;
    }
    let trickle;
    const start = setTimeout(() => {
      phase = "running";
      progress = 0.12;
      // Ease toward, but never reach, the end while waiting.
      trickle = setInterval(() => { progress += (0.9 - progress) * 0.12; }, 200);
    }, DELAY);
    return () => { clearTimeout(start); clearInterval(trickle); };
  });
</script>

<div class="nav-progress {phase}" aria-hidden="true"><span style:transform="scaleX({progress})"></span></div>

<style>
  .nav-progress { position: fixed; inset: 0 0 auto 0; height: 2px; z-index: 100; pointer-events: none; opacity: 0; transition: opacity 280ms var(--ease-out) 80ms; }
  .nav-progress.running { opacity: 1; transition-delay: 0s; }
  .nav-progress.done { opacity: 0; }
  .nav-progress span { display: block; height: 100%; transform-origin: left; background: linear-gradient(90deg, rgb(212 255 90 / 0.4), var(--accent)); box-shadow: 0 0 12px rgb(212 255 90 / 0.7); transition: transform 240ms var(--ease-out); }
  .nav-progress.idle span { transition: none; }
</style>
