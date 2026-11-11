<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { onMount } from "svelte";
  import { prefersReducedMotion } from "$lib/actions";
  import { art, backdrop, poster } from "$lib/site";

  // Stepping through the titles by 3 keeps a column from repeating itself, and a
  // backdrop in each breaks up the rows.
  const columns = Array.from({ length: 9 }, (_, c) => ({
    items: Array.from({ length: 6 }, (_, i) => {
      const item = art[(c * 5 + i * 3) % art.length];
      return i % 3 === c % 3 && item.still !== false ? { src: backdrop(item.slug), wide: true } : { src: poster(item.slug), wide: false };
    }),
    duration: 70 + ((c * 37) % 50),
    reverse: c % 2 === 1,
  }));

  let wall = $state();

  // Pointer and scroll ease toward their targets each frame, so the plane drifts rather than snaps.
  onMount(() => {
    if (prefersReducedMotion()) return;
    const target = { x: 0, y: 0 };
    const now = { x: 0, y: 0, s: 0 };
    let frame = 0;
    const tick = () => {
      const scroll = Math.min(1, scrollY / innerHeight);
      now.x += (target.x - now.x) * 0.06;
      now.y += (target.y - now.y) * 0.06;
      now.s += (scroll - now.s) * 0.18;
      wall.style.setProperty("--mx", now.x.toFixed(4));
      wall.style.setProperty("--my", now.y.toFixed(4));
      wall.style.setProperty("--scroll", now.s.toFixed(4));
      frame = Math.abs(target.x - now.x) + Math.abs(target.y - now.y) + Math.abs(scroll - now.s) > 0.0005 ? requestAnimationFrame(tick) : 0;
    };
    const wake = () => { if (!frame) frame = requestAnimationFrame(tick); };
    const move = (event) => {
      target.x = event.clientX / innerWidth - 0.5;
      target.y = event.clientY / innerHeight - 0.5;
      wake();
    };
    addEventListener("pointermove", move, { passive: true });
    addEventListener("scroll", wake, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      removeEventListener("pointermove", move);
      removeEventListener("scroll", wake);
    };
  });
</script>

<div class="wall" bind:this={wall} aria-hidden="true">
  <div class="plane">
    {#each columns as column, c}
      <div class="column" class:reverse={column.reverse} style:--duration="{column.duration}s" style:--offset="{(c % 3) * -12}%">
        <div class="strip">
          {#each [...column.items, ...column.items] as item, i}
            <img src={item.src} class:wide={item.wide} alt="" width={item.wide ? 1600 : 480} height={item.wide ? 900 : 720} decoding="async" fetchpriority={c > 1 && c < 7 && i < 3 ? "high" : "low"} />
          {/each}
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .wall {
    --mx: 0;
    --my: 0;
    --scroll: 0;
    position: absolute;
    inset: 0;
    overflow: hidden;
    perspective: 1500px;
    perspective-origin: 60% 30%;
    pointer-events: none;
  }
  .plane {
    position: absolute;
    left: 58%;
    top: 46%;
    display: flex;
    gap: clamp(14px, 1.4vw, 22px);
    width: max(1900px, 150vw);
    transform-style: preserve-3d;
    transform:
      translate(-50%, -50%)
      rotateX(calc(42deg + var(--my) * -6deg + var(--scroll) * 16deg))
      rotateZ(calc(-22deg + var(--mx) * 5deg))
      translateZ(calc(var(--scroll) * -260px));
    opacity: calc(1 - var(--scroll) * 0.9);
    animation: wall-in 2.2s var(--ease-out) both;
  }
  @keyframes wall-in { from { opacity: 0; transform: translate(-50%, -42%) rotateX(58deg) rotateZ(-22deg) translateZ(-420px); } }
  .column { flex: 1; min-width: 0; height: 240vh; overflow: hidden; transform: translateY(var(--offset)); }
  .strip { display: grid; gap: clamp(14px, 1.4vw, 22px); animation: drift var(--duration) linear infinite; }
  .reverse .strip { animation-direction: reverse; }
  @keyframes drift { to { transform: translateY(calc(-50% - clamp(7px, 0.7vw, 11px))); } }
  img {
    width: 100%;
    height: auto;
    aspect-ratio: 2 / 3;
    object-fit: cover;
    border-radius: var(--r);
    outline: 1px solid rgb(255 255 255 / 0.06);
    outline-offset: -1px;
    box-shadow: 0 30px 60px -30px rgb(0 0 0 / 0.9);
    background: var(--surface-2);
  }
  img.wide { aspect-ratio: 3 / 4; }
  @media (max-width: 759px) {
    .plane { left: 50%; top: 40%; width: 1300px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .strip { animation: none; }
  }
</style>
