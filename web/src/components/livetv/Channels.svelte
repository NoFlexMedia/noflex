<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import Play from "phosphor-svelte/lib/Play";
  import Television from "phosphor-svelte/lib/Television";
  import { app } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";
  import ChannelLogo from "./ChannelLogo.svelte";

  let { channels, onNow, now, loading, starting, onplay, onsetup } = $props();

  let filter = $state("");
  let visible = $derived(filter.trim() ? channels.filter((channel) => `${channel.number ?? ""} ${channel.name} ${channel.group ?? ""}`.toLowerCase().includes(filter.trim().toLowerCase())) : channels);
  const progress = (program) => Math.min(1, (now - Date.parse(program.startAt)) / (Date.parse(program.endAt) - Date.parse(program.startAt)));
</script>

{#if loading}
  <div class="grid" aria-busy="true">{#each Array(12) as _}<div class="skeleton card-skel"></div>{/each}</div>
{:else if !channels.length}
  <State icon={Television} title="No channels yet" message={app.viewer.isAdmin ? "Add an M3U tuner source and its channels appear here." : "Live TV hasn’t been set up on this server yet."}>
    {#if app.viewer.isAdmin}<button class="btn primary sm" onclick={onsetup}>Add a source</button>{/if}
  </State>
{:else}
  {#if channels.length > 12}
    <label class="filter">
      <MagnifyingGlass size={16} />
      <span class="sr-only">Filter channels</span>
      <input class="input" type="search" bind:value={filter} placeholder="Filter by name, number or group" autocomplete="off" />
    </label>
  {/if}
  {#if visible.length}
    <div class="grid stagger">
      {#each visible as channel, index (channel.id)}
        {@const program = onNow.get(channel.id)}
        <button class="card" style:--i={index % 24} onclick={() => onplay(channel)} disabled={starting === channel.id} aria-label="Watch {channel.name}{program ? `, now showing ${program.title}` : ''}">
          <div class="top">
            <ChannelLogo {channel} size={46} />
            <span class="id">
              {#if channel.number}<span class="number tabular">{channel.number}</span>{/if}
              <strong>{channel.name}</strong>
              <small>{channel.group || "Live channel"}</small>
            </span>
            <span class="play">{#if starting === channel.id}<span class="spinner sm"></span>{:else}<Play size={15} weight="fill" />{/if}</span>
          </div>
          <div class="now">
            {#if program}
              <span class="eyebrow">Now</span>
              <span class="title">{program.title}</span>
              <span class="progress-bar"><span style:width="{progress(program) * 100}%"></span></span>
            {:else}
              <span class="faint">No guide information</span>
            {/if}
          </div>
        </button>
      {/each}
    </div>
  {:else}
    <p class="empty-note">No channels match “{filter}”.</p>
  {/if}
{/if}

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr)); gap: 14px; }
  .card-skel { height: 132px; border-radius: var(--r-l); }
  .filter { position: relative; display: block; max-width: 360px; margin-bottom: 18px; }
  .filter :global(svg) { position: absolute; left: 14px; top: 50%; translate: 0 -50%; color: var(--text-3); pointer-events: none; }
  .filter input { padding-left: 40px; border-radius: 999px; }
  .card { display: grid; gap: 14px; padding: 16px; border-radius: var(--r-l); background: var(--surface); border: 1px solid var(--line); text-align: left; transition: background var(--t), border-color var(--t), transform var(--t) var(--ease-out), box-shadow var(--t); }
  .card:hover { background: var(--surface-2); border-color: var(--line-strong); transform: translateY(-2px); box-shadow: var(--shadow-2); }
  .card:active { transform: scale(0.985); }
  .top { display: flex; align-items: center; gap: 12px; }
  .id { flex: 1; display: grid; min-width: 0; }
  .number { font-size: 0.72rem; font-weight: 700; color: var(--accent); letter-spacing: 0.04em; }
  .id strong { font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .id small { color: var(--text-3); font-size: 0.8rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .play { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 50%; background: var(--surface-3); color: var(--text); flex: none; transition: background var(--t), color var(--t), transform var(--t) var(--ease-spring); }
  .card:hover .play { background: var(--accent); color: var(--accent-ink); transform: scale(1.08); }
  .now { display: grid; gap: 6px; min-height: 48px; align-content: end; padding-top: 12px; border-top: 1px solid var(--line); font-size: 0.86rem; }
  .now .eyebrow { font-size: 0.66rem; color: #ff6b78; }
  .now .title { font-weight: 550; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .now .progress-bar { height: 3px; }
  .now .faint { align-self: center; font-size: 0.84rem; }
</style>
