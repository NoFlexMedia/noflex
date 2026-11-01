<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Play from "phosphor-svelte/lib/Play";
  import Record from "phosphor-svelte/lib/Record";
  import { runtime } from "$lib/format.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import ChannelLogo from "./ChannelLogo.svelte";

  let { open = $bindable(false), program, channel, now, scheduled, onplay, onrecord } = $props();

  const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
  const day = new Intl.DateTimeFormat(undefined, { weekday: "long", month: "short", day: "numeric" });

  let from = $derived(program ? Date.parse(program.startAt) : 0);
  let to = $derived(program ? Date.parse(program.endAt) : 0);
  let live = $derived(from <= now && to > now);
  let ended = $derived(to <= now);
  let progress = $derived(live ? (now - from) / (to - from) : 0);
  let recording = $state(false);

  async function record() {
    recording = true;
    const ok = await onrecord(program);
    recording = false;
    if (ok) open = false;
  }

  function watch() {
    open = false;
    onplay(channel);
  }
</script>

<Dialog bind:open title={program?.title ?? ""} description={program?.subtitle ?? null} width={520}>
  {#if program && channel}
    <div class="body">
      <div class="meta">
        <ChannelLogo {channel} size={34} />
        <span><strong>{channel.name}</strong><small>{day.format(from)} · {time.format(from)} – {time.format(to)} · {runtime(to - from)}</small></span>
      </div>
      {#if live}
        <div class="live">
          <span class="chip danger"><span class="dot live"></span>On now</span>
          <div class="progress-bar"><span style:width="{progress * 100}%"></span></div>
          <small class="tabular">{runtime(to - now) || "<1m"} left</small>
        </div>
      {/if}
      {#if program.category}<div class="row">{#each program.category.split(/\s*[,;/]\s*/).filter(Boolean) as category}<span class="chip">{category}</span>{/each}</div>{/if}
      <p class="description">{program.description || "No description in the guide for this programme."}</p>
    </div>
  {/if}
  {#snippet footer()}
    {#if scheduled}
      <span class="scheduled"><span class="dot rec"></span>Recording scheduled</span>
    {:else if !ended}
      <button class="btn ghost" onclick={record} disabled={recording}>{#if recording}<span class="spinner sm"></span>{:else}<Record size={14} weight="fill" />{/if}Record</button>
    {/if}
    <button class="btn primary" onclick={watch}><Play size={15} weight="fill" />{live ? "Watch live" : "Watch channel"}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: grid; gap: 16px; }
  .meta { display: flex; align-items: center; gap: 12px; }
  .meta span { display: grid; }
  .meta strong { font-weight: 600; font-size: 0.94rem; }
  .meta small { color: var(--text-3); font-size: 0.84rem; }
  .live { display: flex; align-items: center; gap: 12px; }
  .live .progress-bar { flex: 1; }
  .live small { color: var(--text-2); font-size: 0.82rem; white-space: nowrap; }
  .live .dot { width: 6px; height: 6px; }
  .description { color: var(--text-2); line-height: 1.6; }
  .scheduled { display: inline-flex; align-items: center; gap: 8px; margin-right: auto; color: var(--text-2); font-size: 0.88rem; font-weight: 550; }
  .dot.rec { background: #ff4d5e; }
</style>
