<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Clock from "phosphor-svelte/lib/Clock";
  import Television from "phosphor-svelte/lib/Television";
  import { app } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";
  import ChannelLogo from "./ChannelLogo.svelte";
  import ProgramDialog from "./ProgramDialog.svelte";

  let { channels, programs, now, start, end, scheduled, loading, starting, onplay, onrecord, onsetup } = $props();

  const MINUTE = 60_000;
  const SLOT = 30;
  const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
  const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });

  let span = $derived((end - start) / MINUTE);
  let slots = $derived(Array.from({ length: Math.ceil(span / SLOT) }, (_, index) => {
    const at = new Date(start + index * SLOT * MINUTE);
    return { x: index * SLOT, label: time.format(at), day: at.getHours() === 0 && at.getMinutes() === 0 ? weekday.format(at) : "" };
  }));
  let nowX = $derived((now - start) / MINUTE);

  let rows = $derived.by(() => {
    const byChannel = new Map(channels.map((channel) => [channel.id, []]));
    for (const program of programs) {
      const from = Date.parse(program.startAt);
      const to = Date.parse(program.endAt);
      if (to <= start || from >= end) continue;
      const x = (Math.max(from, start) - start) / MINUTE;
      const w = (Math.min(to, end) - Math.max(from, start)) / MINUTE;
      byChannel.get(program.channelId)?.push({ ...program, from, to, x, w });
    }
    return channels.map((channel) => ({ channel, programs: (byChannel.get(channel.id) ?? []).sort((a, b) => a.from - b.from) }));
  });

  let scroller = $state();
  let selected = $state({ open: false, program: null, channel: null });

  function ppm() {
    return parseFloat(getComputedStyle(scroller).getPropertyValue("--ppm")) || 6;
  }
  function jumpToNow(smooth = true) {
    if (!scroller) return;
    scroller.scrollTo({ left: Math.max(0, (nowX - 20) * ppm()), behavior: smooth ? "smooth" : "auto" });
  }
  let jumped = false;
  $effect(() => {
    if (!scroller || jumped || !rows.length) return;
    jumped = true;
    requestAnimationFrame(() => jumpToNow(false));
  });

  const range = (program) => `${time.format(program.from)} – ${time.format(program.to)}`;
</script>

{#if loading}
  <div class="guide skeleton-guide" aria-busy="true">
    <div class="skel-head"><div class="skeleton"></div></div>
    {#each Array(7) as _, row}
      <div class="skel-row">
        <div class="skeleton ch-skel"></div>
        {#each Array(4) as _, index}<div class="skeleton prog-skel" style:flex-grow={((row + index) % 3) + 1}></div>{/each}
      </div>
    {/each}
    <span class="sr-only" role="status">Loading the guide</span>
  </div>
{:else if !channels.length}
  <State icon={Television} title="No channels yet" message={app.viewer.isAdmin ? "Add an M3U tuner source — with an XMLTV guide if you have one — and its channels appear here." : "Live TV hasn’t been set up on this server yet. Ask an administrator to add a source."}>
    {#if app.viewer.isAdmin}<button class="btn primary sm" onclick={onsetup}>Add a source</button>{/if}
  </State>
{:else}
  <div class="guide" bind:this={scroller} role="region" aria-label="Programme guide" tabindex="-1">
    <div class="canvas" style:--span={span}>
      <div class="head">
        <div class="corner">
          <button class="btn sm glass" onclick={() => jumpToNow()}><Clock size={14} />Now</button>
        </div>
        <div class="times">
          {#each slots as slot (slot.x)}
            <span class="slot" style:--x={slot.x}>{#if slot.day}<b>{slot.day}</b>{/if}{slot.label}</span>
          {/each}
          {#if nowX >= 0 && nowX <= span}<span class="now-pill" style:--x={nowX}>{time.format(now)}</span>{/if}
        </div>
      </div>

      {#each rows as { channel, programs: list } (channel.id)}
        <div class="row">
          <button class="ch" onclick={() => onplay(channel)} disabled={starting === channel.id} aria-label="Watch {channel.name} live">
            <ChannelLogo {channel} size={36} />
            <span class="ch-text"><strong>{channel.name}</strong>{#if channel.number}<small class="tabular">{channel.number}</small>{/if}</span>
            {#if starting === channel.id}<span class="spinner sm"></span>{/if}
          </button>
          <div class="lane">
            {#each list as program (program.id)}
              {@const live = program.from <= now && program.to > now}
              <button
                class="prog"
                class:live
                class:past={program.to <= now}
                class:narrow={program.w < 20}
                style:--x={program.x}
                style:--w={program.w}
                style:--p={live ? (now - program.from) / (program.to - program.from) : 0}
                aria-label="{program.title}, {channel.name}, {range(program)}{live ? ', on now' : ''}{scheduled.has(program.id) ? ', recording scheduled' : ''}"
                onclick={() => (selected = { open: true, program, channel })}
              >
                <strong>{#if scheduled.has(program.id)}<span class="rec" aria-hidden="true"></span>{/if}{program.title}</strong>
                <small class="tabular">{range(program)}</small>
              </button>
            {:else}
              <span class="no-data" style:--x={Math.max(0, nowX)}>No guide data</span>
            {/each}
          </div>
        </div>
      {/each}

      {#if nowX >= 0 && nowX <= span}<div class="now-line" style:--x={nowX} aria-hidden="true"></div>{/if}
    </div>
  </div>
{/if}

<ProgramDialog bind:open={selected.open} program={selected.program} channel={selected.channel} {now} scheduled={selected.program ? scheduled.has(selected.program.id) : false} {onplay} {onrecord} />

<style>
  .guide {
    --ppm: 6px;
    --ch: 208px;
    --row: 70px;
    --head: 46px;
    position: relative;
    overflow: auto;
    max-height: max(440px, calc(100dvh - var(--topbar-h) - 250px));
    border-radius: var(--r-l);
    border: 1px solid var(--line);
    background: var(--bg-raised);
    overscroll-behavior: contain;
    outline: none;
  }
  .canvas { position: relative; width: calc(var(--ch) + var(--span) * var(--ppm)); }
  .head { position: sticky; top: 0; z-index: 4; display: flex; height: var(--head); background: rgb(12 14 18 / 0.94); backdrop-filter: blur(12px); border-bottom: 1px solid var(--line); }
  .corner { position: sticky; left: 0; z-index: 5; display: flex; align-items: center; padding-left: 12px; width: var(--ch); flex: none; background: inherit; border-right: 1px solid var(--line); }
  .times { position: relative; flex: none; width: calc(var(--span) * var(--ppm)); }
  .slot { position: absolute; top: 0; bottom: 0; left: calc(var(--x) * var(--ppm)); display: flex; align-items: center; gap: 6px; padding-left: 10px; font-size: 0.78rem; font-weight: 600; color: var(--text-3); white-space: nowrap; border-left: 1px solid var(--line); }
  .slot b { color: var(--accent); font-weight: 700; }
  .now-pill { position: absolute; bottom: 6px; left: calc(var(--x) * var(--ppm)); translate: -50% 0; padding: 2px 7px; border-radius: 99px; background: #ff4d5e; color: #fff; font-size: 0.7rem; font-weight: 700; white-space: nowrap; font-variant-numeric: tabular-nums; transition: left var(--t-slow) var(--ease-out); }
  .row { display: flex; height: var(--row); border-bottom: 1px solid var(--line); }
  .row:last-of-type { border-bottom: 0; }
  .ch { position: sticky; left: 0; z-index: 2; display: flex; align-items: center; gap: 11px; width: var(--ch); flex: none; padding: 0 12px; background: var(--bg-raised); border-right: 1px solid var(--line); text-align: left; transition: background var(--t); }
  .ch:hover { background: #15181d; }
  .ch-text { display: grid; min-width: 0; flex: 1; }
  .ch-text strong { font-size: 0.88rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ch-text small { color: var(--text-3); font-size: 0.76rem; }
  .lane {
    position: relative;
    flex: none;
    width: calc(var(--span) * var(--ppm));
    background-image: linear-gradient(90deg, var(--line) 1px, transparent 1px);
    background-size: calc(var(--ppm) * 30) 100%;
  }
  .prog {
    position: absolute;
    top: 6px;
    bottom: 6px;
    left: calc(var(--x) * var(--ppm) + 2px);
    width: calc(var(--w) * var(--ppm) - 4px);
    display: grid;
    align-content: center;
    gap: 2px;
    padding: 0 12px;
    overflow: hidden;
    isolation: isolate;
    border-radius: 10px;
    background: var(--surface-2);
    border: 1px solid transparent;
    text-align: left;
    transition: background var(--t), border-color var(--t), transform var(--t-fast) var(--ease-out);
  }
  .prog::before { content: ""; position: absolute; inset: 0 auto 0 0; z-index: -1; width: calc(var(--p) * 100%); background: rgb(212 255 90 / 0.1); transition: width var(--t-slow) var(--ease-out); }
  .prog:hover { background: var(--surface-3); border-color: var(--line-strong); }
  .prog:active { transform: scale(0.985); }
  .prog strong { display: flex; align-items: center; gap: 6px; font-size: 0.86rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .prog small { font-size: 0.74rem; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .prog.narrow { padding: 0 6px; }
  .prog.narrow small { display: none; }
  .prog.live { background: rgb(212 255 90 / 0.08); border-color: rgb(212 255 90 / 0.32); }
  .prog.live strong { color: var(--text); }
  .prog.live small { color: var(--accent); }
  .prog.past { opacity: 0.45; }
  .prog.past:hover { opacity: 0.8; }
  .rec { width: 7px; height: 7px; border-radius: 50%; background: #ff4d5e; flex: none; }
  .no-data { position: absolute; top: 50%; translate: 0 -50%; left: calc(var(--x) * var(--ppm) + 14px); color: var(--text-3); font-size: 0.82rem; }
  .now-line { position: absolute; z-index: 1; top: 0; bottom: 0; left: calc(var(--ch) + var(--x) * var(--ppm)); width: 2px; margin-left: -1px; background: #ff4d5e; box-shadow: 0 0 12px rgb(255 77 94 / 0.6); pointer-events: none; transition: left var(--t-slow) var(--ease-out); }
  .now-line::before { content: ""; position: absolute; top: calc(var(--head) - 5px); left: -4px; width: 10px; height: 10px; border-radius: 50%; background: #ff4d5e; }

  .skeleton-guide { padding: 0; overflow: hidden; }
  .skel-head { height: 46px; padding: 12px; border-bottom: 1px solid var(--line); }
  .skel-head .skeleton { height: 22px; width: 40%; border-radius: 6px; }
  .skel-row { display: flex; gap: 6px; height: 70px; padding: 6px; border-bottom: 1px solid var(--line); }
  .ch-skel { width: 196px; flex: none; border-radius: 10px; }
  .prog-skel { flex-basis: 0; border-radius: 10px; }

  @media (max-width: 759px) {
    .guide { --ppm: 4.4px; --ch: 112px; --row: 62px; max-height: calc(100dvh - var(--topbar-h) - 210px); }
    .ch { gap: 8px; padding: 0 8px; }
    .ch :global(.logo) { width: 28px; height: 28px; }
    .ch-text strong { font-size: 0.8rem; }
    .ch-skel { width: 100px; }
    .corner { padding-left: 8px; }
  }
</style>
