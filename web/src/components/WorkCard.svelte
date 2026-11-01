<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import Play from "phosphor-svelte/lib/Play";
  import { tilt } from "$lib/actions/index.js";
  import { titleCase } from "$lib/format.js";
  import { player, playWork, subtitleFor } from "$lib/playback.svelte.js";
  import { markMorph, morph } from "$lib/transition.svelte.js";
  import Artwork from "$components/Artwork.svelte";

  /** `key` must be unique on the page; it scopes the poster morph. */
  let { work, key = work.id, wide = false, width = wide ? 340 : 190, meta = null, eager = false } = $props();

  let card = $state();
  let progress = $derived.by(() => {
    const data = work.userData;
    if (!data || data.played || !data.positionMs || !work.runtimeMs) return null;
    return Math.min(1, data.positionMs / work.runtimeMs);
  });
  let line = $derived(meta ?? ([subtitleFor(work), work.kind !== "EPISODE" ? titleCase(work.kind) : ""].filter(Boolean).join(" · ")));
  let playing = $derived(player.preparing === work.id);
  let playable = $derived(!["SERIES", "SEASON", "ARTIST", "ALBUM"].includes(work.kind));
</script>

<div class="card" class:wide bind:this={card}>
  <a class="hit" href="/works/{work.id}" onclick={() => markMorph(key, card)} aria-label={work.title}>
    <div class="frame" use:tilt>
      <Artwork {work} kind={wide ? "thumb" : "poster"} {width} {eager} vt={!wide && morph.key === key ? "poster" : null} />
      <span class="glare" aria-hidden="true"></span>
      {#if work.userData?.played}<span class="watched" title="Watched"><Check size={13} weight="bold" /></span>{/if}
      {#if wide}<span class="wide-title"><strong>{work.title}</strong>{#if line}<small>{line}</small>{/if}</span>{/if}
      {#if progress != null}<span class="progress" aria-label="{Math.round(progress * 100)}% watched"><span style:width="{progress * 100}%"></span></span>{/if}
    </div>
    {#if !wide}
      <span class="caption"><strong>{work.title}</strong>{#if line}<small>{line}</small>{/if}</span>
    {/if}
  </a>
  {#if playable}
    <button class="quick-play" class:busy={playing} aria-label="Play {work.title}" onclick={() => playWork(work)} disabled={playing}>
      {#if playing}<span class="spinner sm"></span>{:else}<Play size={16} weight="fill" />{/if}
    </button>
  {/if}
</div>

<style>
  .card { position: relative; min-width: 0; }
  .hit { display: grid; gap: 10px; outline: none; }
  .frame {
    position: relative;
    border-radius: var(--r);
    transform: perspective(900px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg)) translateZ(0);
    transition: transform 500ms var(--ease-out), box-shadow 400ms var(--ease-out), scale 220ms var(--ease-spring);
    box-shadow: 0 1px 0 rgb(255 255 255 / 0.05) inset, 0 8px 22px -14px rgb(0 0 0 / 0.9);
    will-change: transform;
  }
  .frame :global(.art) { outline: 1px solid rgb(255 255 255 / 0.06); outline-offset: -1px; }
  .frame:global([data-tilting]) { transition: transform 120ms linear, box-shadow 400ms var(--ease-out), scale 220ms var(--ease-spring); }
  /* A press sinks the poster; `scale` composes with the tilt transform. */
  .hit:active .frame { scale: 0.965; transition-duration: 500ms, 400ms, 90ms; }
  .hit:hover .frame, .hit:focus-visible .frame { box-shadow: 0 1px 0 rgb(255 255 255 / 0.08) inset, 0 26px 44px -18px rgb(0 0 0 / 0.95); }
  .hit:hover .frame:not(:global([data-tilting])) { transform: translateY(-4px); }
  .hit:focus-visible .frame { outline: 2px solid var(--focus); outline-offset: 3px; }
  .glare { position: absolute; inset: 0; border-radius: inherit; pointer-events: none; opacity: 0; background: radial-gradient(circle at var(--gx, 50%) var(--gy, 0%), rgb(255 255 255 / 0.22), transparent 55%); mix-blend-mode: soft-light; transition: opacity 300ms var(--ease-out); }
  .frame:global([data-tilting]) .glare { opacity: 1; }
  .caption { display: grid; gap: 2px; padding: 0 2px; min-width: 0; }
  .caption strong, .wide-title strong { font-weight: 600; font-size: 0.92rem; line-height: 1.3; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; transition: color var(--t); }
  .caption small, .wide-title small { color: var(--text-3); font-size: 0.8rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .hit:hover .caption strong { color: #fff; }
  .wide-title { position: absolute; inset: auto 0 0 0; display: grid; gap: 1px; padding: 40px 14px 14px; border-radius: 0 0 var(--r) var(--r); background: linear-gradient(to top, rgb(0 0 0 / 0.85), transparent); pointer-events: none; }
  .watched { position: absolute; top: 8px; left: 8px; display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; background: var(--accent); color: var(--accent-ink); box-shadow: var(--shadow-1); }
  .progress { position: absolute; left: 10px; right: 10px; bottom: 9px; height: 3px; border-radius: 9px; background: rgb(255 255 255 / 0.22); overflow: hidden; backdrop-filter: blur(4px); }
  .wide .progress { bottom: 7px; }
  .progress span { display: block; height: 100%; border-radius: inherit; background: var(--accent); }
  .quick-play {
    position: absolute;
    right: 10px;
    top: 10px;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: rgb(255 255 255 / 0.92);
    color: #0b0c0f;
    box-shadow: 0 10px 24px -8px rgb(0 0 0 / 0.7);
    opacity: 0;
    transform: scale(0.7);
    transition: opacity 260ms var(--ease-out), transform 360ms var(--ease-spring), background var(--t);
  }
  .quick-play :global(svg) { margin-left: 2px; }
  .card:hover .quick-play, .quick-play:focus-visible, .quick-play.busy { opacity: 1; transform: none; }
  .quick-play:hover { background: var(--accent); }
  .quick-play:active { transform: scale(0.9); }
  .wide .quick-play { top: auto; bottom: 18px; right: 14px; }
  @media (hover: none) { .quick-play { display: none; } }
</style>
