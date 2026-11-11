<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowClockwise from "phosphor-svelte/lib/ArrowClockwise";
  import ArrowCounterClockwise from "phosphor-svelte/lib/ArrowCounterClockwise";
  import Check from "phosphor-svelte/lib/Check";
  import Gear from "phosphor-svelte/lib/Gear";
  import Info from "phosphor-svelte/lib/Info";
  import Pause from "phosphor-svelte/lib/Pause";
  import Play from "phosphor-svelte/lib/Play";
  import Stop from "phosphor-svelte/lib/Stop";
  import { megabits } from "$quality";
  import { cast } from "$lib/cast.svelte.js";
  import { connect } from "$lib/connect.svelte.js";
  import { timecode } from "$lib/format.js";
  import { subtitleFor } from "$lib/playback.svelte.js";
  import Artwork from "$components/Artwork.svelte";
  import DeviceIcon from "$components/DeviceIcon.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Tip from "$components/ui/Tip.svelte";

  // Steering what another device is playing: it follows this bar, as the
  // player on this one follows its own.
  const SKIP_BACK_MS = 10_000;
  const SKIP_FORWARD_MS = 30_000;

  let remote = $derived(connect.active);
  let playback = $derived(remote?.playback ?? null);
  let position = $derived(remote ? connect.position(remote) : 0);
  let playing = $derived(playback?.state === "PLAYING");
  let loading = $derived(playback?.state === "BUFFERING");
  let duration = $derived(playback?.durationMs ?? 0);
  let scrubbing = $state(null);

  // A Cast device plays what this browser asks for, so its quality is chosen here.
  let casting = $derived(remote?.via === "cast");

  const seek = (to) => remote && connect.control(remote, "SEEK", Math.round(Math.max(0, duration ? Math.min(to, duration) : to)));
</script>

{#if remote && playback}
  <section class="remote" class:casting aria-label="Playing on {remote.device.name}">
    <div class="art"><Artwork work={playback.work} width={44} /></div>
    <div class="what">
      <strong>{playback.work?.title ?? "Playing"}</strong>
      <span><DeviceIcon device={remote.device.name} client={remote.device.client} size={14} />{remote.device.name}{#if playback.work && subtitleFor(playback.work)}{" · "}{subtitleFor(playback.work)}{/if}</span>
    </div>
    <div class="controls">
      <Tip text="Back 10 seconds">{#snippet children(tip)}<button {...tip} type="button" class="icon-btn sm skip" aria-label="Back 10 seconds" onclick={() => seek(position - SKIP_BACK_MS)}><ArrowCounterClockwise size={19} /></button>{/snippet}</Tip>
      <button type="button" class="icon-btn solid toggle" aria-label={loading ? "Loading" : playing ? "Pause" : "Play"} aria-busy={loading} onclick={() => connect.control(remote, playing || loading ? "PAUSE" : "PLAY")}>
        {#if loading}<span class="spinner sm"></span>{:else if playing}<Pause size={20} weight="fill" />{:else}<Play size={20} weight="fill" />{/if}
      </button>
      <Tip text="Forward 30 seconds">{#snippet children(tip)}<button {...tip} type="button" class="icon-btn sm skip" aria-label="Forward 30 seconds" onclick={() => seek(position + SKIP_FORWARD_MS)}><ArrowClockwise size={19} /></button>{/snippet}</Tip>
    </div>
    <div class="scrub">
      <span class="time">{timecode(scrubbing ?? position)}</span>
      <input
        type="range" min="0" max={duration || 1} step="1000" disabled={!duration}
        value={scrubbing ?? position}
        style:--played="{duration ? ((scrubbing ?? position) / duration) * 100 : 0}%"
        aria-label="Position"
        oninput={(event) => (scrubbing = Number(event.currentTarget.value))}
        onchange={(event) => { seek(Number(event.currentTarget.value)); scrubbing = null; }}
      />
      <span class="time">{duration ? timecode(duration) : ""}</span>
    </div>
    {#if casting}
      <Menu side="top" width={240}>
        {#snippet trigger(props)}<button {...props} type="button" class="icon-btn sm" aria-label="Playback settings on {remote.device.name}"><Gear size={18} /></button>{/snippet}
        <MenuItem icon={cast.statsOnTv ? Check : Info} onSelect={() => cast.showStats(!cast.statsOnTv)}>Stream info on the TV</MenuItem>
        {#if cast.qualities.length}
          <div class="nf-menu-sep"></div>
          <div class="nf-menu-label">Quality</div>
          <MenuItem icon={cast.choice.kind === "original" ? Check : null} onSelect={() => cast.setQuality({ kind: "original" })}>Original</MenuItem>
          <MenuItem icon={cast.choice.kind === "auto" ? Check : null} onSelect={() => cast.setQuality({ kind: "auto" })}><span class="quality">Auto<small>adapts to the connection</small></span></MenuItem>
          {#each cast.qualities as quality (quality.bitrateBps)}
            <MenuItem icon={cast.choice.kind === "pinned" && cast.choice.bitrateBps === quality.bitrateBps ? Check : null} onSelect={() => cast.setQuality({ kind: "pinned", bitrateBps: quality.bitrateBps })}><span class="quality">{quality.label}<small>{megabits(quality.bitrateBps)}</small></span></MenuItem>
          {/each}
        {/if}
      </Menu>
    {/if}
    <Tip text="Stop playing on {remote.device.name}">{#snippet children(tip)}<button {...tip} type="button" class="icon-btn sm" aria-label="Stop playing on {remote.device.name}" onclick={() => connect.control(remote, "STOP")}><Stop size={18} weight="fill" /></button>{/snippet}</Tip>
  </section>
{/if}

<style>
  .remote { position: fixed; z-index: 45; left: calc(var(--nav-w) + 12px); right: 12px; bottom: 12px; display: grid; grid-template-columns: auto minmax(0, 1fr) auto minmax(160px, 1.4fr) auto; align-items: center; gap: 14px; padding: 8px 12px 8px 8px; border: 1px solid var(--line-strong); border-radius: var(--r-l); background: var(--bg-overlay); backdrop-filter: blur(22px) saturate(1.4); box-shadow: var(--shadow-2); animation: rise var(--t) var(--ease-out); }
  .remote.casting { grid-template-columns: auto minmax(0, 1fr) auto minmax(160px, 1.4fr) auto auto; }
  @keyframes rise { from { transform: translateY(12px); opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .remote { animation: none; } }
  .quality { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; width: 100%; }
  .quality small { color: var(--text-3); font-size: 0.78rem; font-variant-numeric: tabular-nums; }
  .art { width: 44px; border-radius: var(--r-xs); overflow: hidden; }
  .what { display: grid; min-width: 0; line-height: 1.3; }
  .what strong, .what span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .what strong { font-weight: 650; }
  .what span { display: inline-flex; align-items: center; gap: 6px; color: var(--text-2); font-size: 0.82rem; }
  .controls { display: flex; align-items: center; gap: 4px; }
  .toggle { width: 40px; height: 40px; }
  .scrub { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 10px; }
  .time { min-width: 3.4ch; color: var(--text-3); font-size: 0.78rem; font-variant-numeric: tabular-nums; }
  input[type="range"] { appearance: none; height: 20px; margin: 0; background: transparent; cursor: pointer; }
  input[type="range"]:disabled { cursor: default; }
  input[type="range"]::-webkit-slider-runnable-track { height: 4px; border-radius: 2px; background: linear-gradient(to right, var(--accent) var(--played), var(--surface-3) var(--played)); }
  input[type="range"]::-moz-range-track { height: 4px; border-radius: 2px; background: var(--surface-3); }
  input[type="range"]::-moz-range-progress { height: 4px; border-radius: 2px; background: var(--accent); }
  input[type="range"]::-webkit-slider-thumb { appearance: none; width: 12px; height: 12px; margin-top: -4px; border-radius: 50%; background: var(--text); transition: transform var(--t-fast) var(--ease-out); }
  input[type="range"]::-moz-range-thumb { width: 12px; height: 12px; border: 0; border-radius: 50%; background: var(--text); }
  input[type="range"]:active::-webkit-slider-thumb { transform: scale(1.3); }
  input[type="range"]:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; border-radius: 4px; }
  @media (max-width: 759px) {
    .remote { left: 8px; right: 8px; bottom: calc(68px + env(safe-area-inset-bottom)); grid-template-columns: auto minmax(0, 1fr) auto auto; gap: 10px; }
    .skip, .time { display: none; }
    .remote.casting { grid-template-columns: auto minmax(0, 1fr) auto auto auto; }
    .scrub { grid-column: 1 / -1; grid-row: 2; grid-template-columns: minmax(0, 1fr); }
  }
</style>
