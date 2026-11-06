<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { MySmartMarksStore, PreparedVersionsStore, UnmarkForSmartTranscodingStore } from "$houdini";
  import { bytes } from "$lib/format.js";
  import { fetchQuery, refetch } from "$lib/graphql.js";
  import { subtitleFor } from "$lib/playback.svelte.js";
  import { attempt, poll } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import { ago } from "./relative.js";

  const marks = new MySmartMarksStore();
  const unmark = new UnmarkForSmartTranscodingStore();
  const prepared = new PreparedVersionsStore();
  $effect(() => { fetchQuery(marks); fetchQuery(prepared); });

  // [label, chip tone, what it means when that isn't obvious]
  const STATES = {
    WAITING: ["In the queue", "", "It’s next, once nobody is watching anything."],
    PREPARING: ["Preparing", "accent", ""],
    READY: ["Ready", "ready", ""],
    NOT_NEEDED: ["Plays as it is", "ready", "Nothing to prepare: the original already plays there."],
    TOO_LARGE: ["Too large", "danger", "It’s bigger than this server allows a prepared version to be."],
    NO_DEVICES: ["No devices", "danger", "None of your devices has smart transcoding on. Turn it on under Devices & keys."],
    NO_SPACE: ["Waiting for space", "", "Other marked titles are using all the space this server sets aside."],
    FAILED: ["Failed", "danger", "The server couldn’t prepare this. Unmark it and mark it again to retry."],
    DISABLED: ["Off on this server", "", "Smart transcoding is off in this server’s configuration."],
  };

  let list = $derived($marks.data?.mySmartTranscodeMarks ?? []);
  let active = $derived(list.some((mark) => mark.state === "WAITING" || mark.state === "PREPARING"));
  let ready = $derived($prepared.data?.preparedVersions ?? []);
  $effect(() => {
    if (!active) return;
    const [one, other] = [marks, prepared];
    return poll(() => { refetch(one); refetch(other); }, 5000);
  });

  async function remove(mark) {
    await attempt(() => unmark.mutate({ work: mark.work.id, device: mark.device?.id ?? null }), "Unmarked");
    refetch(marks);
    refetch(prepared);
  }
</script>

<p class="intro">
  Titles you’re going to watch next, in the order the server will prepare them, ahead of anything it guesses at. Each stays until you watch it or unmark it. Mark one from the ••• menu on its page. <a href="/account?tab=devices">Choose your smart transcoding devices</a>.
</p>

{#if $marks.fetching && !$marks.data}
  <div class="marks" aria-busy="true">{#each Array(3) as _}<div class="skeleton mark-skel"></div>{/each}</div>
{:else if !list.length && !ready.length}
  <State icon={Sparkle} title="Nothing marked yet" message="Open a film or show, choose ••• → Mark for smart transcoding, and it’ll be ready before you sit down to watch.">
    <a class="btn sm" href="/">Browse your libraries</a>
  </State>
{:else if list.length}
  <h3 class="heading">Marked to watch next</h3>
  <ul class="marks stagger">
    {#each list as mark, index (`${mark.work.id}-${mark.device?.id ?? "all"}`)}
      {@const [label, tone, detail] = STATES[mark.state] ?? [mark.state, "", ""]}
      {@const done = mark.state === "READY" || mark.state === "NOT_NEEDED"}
      <li class="panel mark" style:--i={index}>
        <span class="icon" class:ready={done} class:failed={tone === "danger"}>{#if tone === "danger"}<WarningCircle size={19} />{:else if done}<Check size={19} />{:else}<Sparkle size={19} />{/if}</span>
        <div class="body">
          <div class="title-row">
            <a class="title" href="/works/{mark.work.id}">{mark.work.title}{#if mark.work.kind === "EPISODE" && subtitleFor(mark.work)}<small> {subtitleFor(mark.work)}</small>{/if}</a>
            <span class="chip {tone}">{#if mark.state === "PREPARING"}<span class="dot on"></span>{/if}{label}</span>
          </div>
          <p>{mark.device ? `For ${mark.device.name} only` : "For your devices with smart transcoding on"} · marked {ago(mark.markedAt)}</p>
          {#if detail}<p class:error={tone === "danger"}>{detail}</p>{/if}
          {#if mark.state === "PREPARING"}
            <div class="progress">
              <div class="progress-bar" role="progressbar" aria-label="Preparing" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(mark.progress * 100)}><span style:width="{mark.progress * 100}%"></span></div>
              <span class="pct tabular">{Math.round(mark.progress * 100)}%</span>
            </div>
          {/if}
        </div>
        <button class="btn ghost sm" onclick={() => remove(mark)} aria-label="Unmark {mark.work.title}">Unmark</button>
      </li>
    {/each}
  </ul>
{/if}

{#if ready.length}
  <h3 class="heading">Ready to play</h3>
  <p class="intro">Already prepared on this server, whoever or whatever asked for it, and used by any device it suits. Unmarking a title doesn’t remove it. The least recently played make room when space runs out.</p>
  <ul class="marks">
    {#each ready as version (version.id)}
      <li class="panel mark">
        <span class="icon ready"><Check size={19} /></span>
        <div class="body">
          <div class="title-row">
            {#if version.work}
              <a class="title" href="/works/{version.work.id}">{version.work.title}{#if version.work.kind === "EPISODE" && subtitleFor(version.work)}<small> {subtitleFor(version.work)}</small>{/if}</a>
            {:else}
              <span class="title">A removed title</span>
            {/if}
            <span class="chip">{bytes(version.sizeBytes)}</span>
          </div>
          <p>{[version.description, version.finishedAt ? `prepared ${ago(version.finishedAt)}` : "", version.lastUsedAt ? `last played ${ago(version.lastUsedAt)}` : "not played yet"].filter(Boolean).join(" · ")}</p>
        </div>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .heading { margin: 26px 0 12px; font-size: 0.98rem; font-weight: 600; }
  .heading:first-of-type { margin-top: 0; }
  .intro { max-width: 720px; margin-bottom: 20px; color: var(--text-2); font-size: 0.92rem; line-height: 1.5; }
  .intro a { color: var(--text); text-decoration: underline; text-underline-offset: 3px; }
  .marks { display: grid; gap: 10px; max-width: 920px; }
  .mark-skel { height: 86px; border-radius: var(--r-l); }
  .mark { display: flex; align-items: center; gap: 16px; padding: 16px 18px; }
  .icon { flex: none; display: grid; place-items: center; width: 42px; height: 42px; border-radius: 12px; background: var(--surface-2); color: var(--text-2); }
  .icon.ready { background: rgb(110 231 168 / 0.12); color: var(--success); }
  .icon.failed { background: var(--danger-soft); color: var(--danger); }
  .body { flex: 1; display: grid; gap: 4px; min-width: 0; }
  .title-row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .title { font-weight: 600; }
  .title:hover { text-decoration: underline; text-underline-offset: 3px; }
  .title small { color: var(--text-3); font-weight: 500; }
  .chip { height: 22px; font-size: 0.72rem; }
  .chip.ready { background: rgb(110 231 168 / 0.12); color: var(--success); }
  .chip.danger { background: var(--danger-soft); color: var(--danger); }
  .chip .dot { width: 6px; height: 6px; }
  .body p { color: var(--text-3); font-size: 0.84rem; }
  .body p.error { color: var(--danger); }
  .progress { display: flex; align-items: center; gap: 12px; margin-top: 6px; }
  .progress .progress-bar { flex: 1; }
  .pct { font-size: 0.8rem; color: var(--text-2); font-weight: 600; min-width: 52px; text-align: right; }
  @media (max-width: 560px) { .mark { flex-wrap: wrap; } }
</style>
