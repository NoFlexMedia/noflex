<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CalendarBlank from "phosphor-svelte/lib/CalendarBlank";
  import DownloadSimple from "phosphor-svelte/lib/DownloadSimple";
  import Plus from "phosphor-svelte/lib/Plus";
  import Record from "phosphor-svelte/lib/Record";
  import Trash from "phosphor-svelte/lib/Trash";
  import X from "phosphor-svelte/lib/X";
  import { graphql, ScheduleRecordingStore } from "$houdini";
  import { bytes, date, runtime } from "$lib/format.js";
  import { refetch } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import { ask, attempt, confirmAction, toast } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import Select from "$components/ui/Select.svelte";
  import { saveFile } from "$components/account/save-file.js";
  import ChannelLogo from "./ChannelLogo.svelte";

  /** `live` is the page's LiveTv store. */
  let { live, channels, programs } = $props();

  const cancelSchedule = graphql(`
    mutation CancelRecordingSchedule($id: ID!) { cancelRecordingSchedule(id: $id) { id status } }
  `);
  const deleteRecording = graphql(`
    mutation DeleteRecording($id: ID!, $files: Boolean!) { deleteRecording(id: $id, deleteFile: $files) { id @Recording_delete } }
  `);
  const scheduleRecording = new ScheduleRecordingStore();

  const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
  const SCHEDULE_STATUS = { SCHEDULED: ["Scheduled", "accent"], TRIGGERED: ["Started", ""], CANCELLED: ["Cancelled", ""] };
  const RECORDING_STATUS = { SCHEDULED: ["Waiting", ""], RUNNING: ["Recording", "danger"], COMPLETED: ["Ready", "ready"], FAILED: ["Failed", "danger"], CANCELLED: ["Cancelled", ""] };

  let channelById = $derived(new Map(channels.map((channel) => [channel.id, channel])));
  let programById = $derived(new Map(programs.map((program) => [program.id, program])));
  let schedules = $derived([...($live.data?.recordingSchedules.nodes ?? [])].sort((a, b) => Number(b.status === "SCHEDULED") - Number(a.status === "SCHEDULED") || Date.parse(a.startAt) - Date.parse(b.startAt)));
  let scheduleById = $derived(new Map(schedules.map((schedule) => [schedule.id, schedule])));
  let recordings = $derived(($live.data?.recordings.edges ?? []).map((edge) => edge.node).sort((a, b) => Date.parse(b.startedAt ?? b.createdAt) - Date.parse(a.startedAt ?? a.createdAt)));
  let upcoming = $derived(schedules.filter((item) => item.status === "SCHEDULED"));
  let past = $derived(schedules.filter((item) => item.status !== "SCHEDULED"));

  const channelOf = (id) => channelById.get(id) ?? { id, name: "Unknown channel", logoUrl: null };
  const titleOf = (schedule) => (schedule?.programId && programById.get(schedule.programId)?.title) || null;
  const span = (from, to) => {
    const start = new Date(from);
    const sameDay = new Date(to).toDateString() === start.toDateString();
    return `${date(from, { weekday: "short", month: "short", day: "numeric" })} · ${time.format(start)} – ${sameDay ? time.format(new Date(to)) : date(to, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}`;
  };

  async function cancel(schedule) {
    const label = titleOf(schedule) ?? channelOf(schedule.channelId).name;
    if (!(await confirmAction({ title: `Cancel recording “${label}”?`, message: "Nothing will be recorded for this slot.", confirmLabel: "Cancel recording", danger: true }))) return;
    await attempt(() => cancelSchedule.mutate({ id: schedule.id }), "Recording cancelled");
  }

  let downloading = $state("");
  async function download(recording) {
    downloading = recording.id;
    try {
      if (await saveFile(`/recordings/${recording.id}/file`, `noflex-recording-${recording.id.slice(0, 8)}.mkv`)) toast("Recording saved", "success");
    } catch (cause) { toast(cause instanceof Error ? cause.message : String(cause), "error"); }
    finally { downloading = ""; }
  }

  async function remove(recording) {
    const answer = await ask({ title: "Remove this recording?", message: "It disappears from your DVR list.", option: recording.sizeBytes ? `Also delete the file from disk (${bytes(recording.sizeBytes)})` : "Also delete the file from disk", confirmLabel: "Remove", danger: true });
    if (!answer.confirmed) return;
    await attempt(() => deleteRecording.mutate({ id: recording.id, files: answer.option }), answer.option ? "Recording and file deleted" : "Recording removed");
  }

  // ---- Manual slot ----
  const local = (ms) => { const d = new Date(ms); d.setMinutes(d.getMinutes() - d.getTimezoneOffset()); return d.toISOString().slice(0, 16); };
  let slot = $state({ open: false, channel: "", start: "", end: "", before: "0", after: "0" });
  let saving = $state(false);
  let invalid = $derived(slot.start && slot.end && Date.parse(slot.end) <= Date.parse(slot.start));

  function openSlot() {
    const next = Math.ceil(Date.now() / 1_800_000) * 1_800_000;
    slot = { open: true, channel: slot.channel || channels[0]?.id || "", start: local(next), end: local(next + 3_600_000), before: "0", after: "0" };
  }

  async function schedule(event) {
    event.preventDefault();
    if (invalid) return;
    saving = true;
    const result = await attempt(() => scheduleRecording.mutate({ channel: slot.channel, program: null, start: new Date(slot.start).toISOString(), end: new Date(slot.end).toISOString(), before: Number(slot.before) * 60, after: Number(slot.after) * 60 }), "Time slot scheduled");
    saving = false;
    if (result) { slot.open = false; refetch(live); }
  }
</script>

{#if $live.fetching && !$live.data}
  <div class="dvr" aria-busy="true">
    {#each [0, 1] as _}<div class="panel"><div class="skeleton head"></div>{#each Array(3) as _}<div class="skeleton line"></div>{/each}</div>{/each}
  </div>
{:else}
  <div class="dvr stagger">
    <section class="panel" style:--i={0}>
      <header class="panel-head">
        <span class="panel-icon"><CalendarBlank size={19} /></span>
        <div><h2>Scheduled</h2><p>{upcoming.length ? `${upcoming.length} upcoming` : "Nothing queued"}</p></div>
        <span class="spacer"></span>
        <button class="btn sm" onclick={openSlot} disabled={!channels.length}><Plus size={15} />Time slot</button>
      </header>
      {#if schedules.length}
        <ul class="list">
          {#each [...upcoming, ...past.slice(0, 10)] as item (item.id)}
            {@const [label, tone] = SCHEDULE_STATUS[item.status] ?? [item.status, ""]}
            {@const channel = channelOf(item.channelId)}
            <li class:dim={item.status !== "SCHEDULED"}>
              <ChannelLogo {channel} size={36} />
              <span class="what">
                <strong>{titleOf(item) ?? channel.name}</strong>
                <small>{titleOf(item) ? `${channel.name} · ` : ""}{span(item.startAt, item.endAt)}{item.paddingBeforeSecs || item.paddingAfterSecs ? ` · +${Math.round((item.paddingBeforeSecs + item.paddingAfterSecs) / 60)}m padding` : ""}</small>
              </span>
              <span class="chip {tone}">{label}</span>
              {#if item.status === "SCHEDULED"}<button class="icon-btn sm" aria-label="Cancel recording on {channel.name}" onclick={() => cancel(item)}><X size={16} /></button>{/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty-note">Pick a programme in the guide and choose Record, or schedule a time slot on any channel.</p>
      {/if}
    </section>

    <section class="panel" style:--i={1}>
      <header class="panel-head">
        <span class="panel-icon"><Record size={17} weight="fill" /></span>
        <div><h2>Recordings</h2><p>{recordings.length ? `${recordings.length} on this server` : "None yet"}</p></div>
      </header>
      {#if recordings.length}
        <ul class="list">
          {#each recordings as recording (recording.id)}
            {@const [label, tone] = RECORDING_STATUS[recording.status] ?? [recording.status, ""]}
            {@const channel = channelOf(recording.channelId)}
            {@const title = titleOf(scheduleById.get(recording.scheduleId))}
            <li>
              <ChannelLogo {channel} size={36} />
              <span class="what">
                <strong>{title ?? channel.name}</strong>
                {#if recording.status === "FAILED"}
                  <small class="error">{recording.error || "The recorder stopped unexpectedly."}</small>
                {:else}
                  <small>{title ? `${channel.name} · ` : ""}{recording.startedAt ? date(recording.startedAt, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }) : "Not started"}{recording.startedAt && recording.completedAt ? ` · ${runtime(Date.parse(recording.completedAt) - Date.parse(recording.startedAt))}` : ""}{recording.sizeBytes ? ` · ${bytes(recording.sizeBytes)}` : ""}</small>
                {/if}
              </span>
              <span class="chip {tone}">{#if recording.status === "RUNNING"}<span class="dot live"></span>{/if}{label}</span>
              {#if recording.status === "COMPLETED"}
                {#if app.server?.downloadsAllowed}<button class="icon-btn sm" aria-label="Download recording" onclick={() => download(recording)} disabled={downloading === recording.id}>{#if downloading === recording.id}<span class="spinner sm"></span>{:else}<DownloadSimple size={16} />{/if}</button>{/if}
              {/if}
              <button class="icon-btn sm" aria-label="Remove recording" onclick={() => remove(recording)}><Trash size={16} /></button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty-note">Finished recordings land here, ready to download.</p>
      {/if}
    </section>
  </div>
{/if}

<Dialog bind:open={slot.open} title="Schedule a time slot" description="Record a channel between two times, whatever’s on." width={500}>
  <form id="slot-form" class="stack" onsubmit={schedule}>
    <label class="field">Channel<Select bind:value={slot.channel} items={channels.map((channel) => ({ value: channel.id, label: channel.name, hint: channel.number ?? undefined }))} label="Channel" placeholder="Choose a channel" /></label>
    <div class="two">
      <label class="field">Starts<input type="datetime-local" bind:value={slot.start} required /></label>
      <label class="field">Ends<input type="datetime-local" bind:value={slot.end} required min={slot.start} aria-invalid={invalid} /></label>
    </div>
    {#if invalid}<p class="error" role="alert">The end has to be after the start.</p>{/if}
    <div class="two">
      <label class="field">Start early<span class="suffix"><input type="number" min="0" max="60" bind:value={slot.before} /><span>min</span></span></label>
      <label class="field">Run over<span class="suffix"><input type="number" min="0" max="120" bind:value={slot.after} /><span>min</span></span></label>
    </div>
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (slot.open = false)}>Cancel</button>
    <button class="btn primary" form="slot-form" disabled={saving || !slot.channel || invalid}>{#if saving}<span class="spinner sm"></span>{/if}Schedule</button>
  {/snippet}
</Dialog>

<style>
  .dvr { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 420px), 1fr)); gap: 18px; align-items: start; }
  .skeleton.head { height: 38px; width: 50%; margin-bottom: 22px; border-radius: 11px; }
  .skeleton.line { height: 44px; margin-top: 12px; border-radius: 10px; }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what strong { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .what small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .what small.error, .error { color: var(--danger); white-space: normal; font-size: 0.84rem; }
  .dim { opacity: 0.55; }
  .chip { height: 24px; font-size: 0.74rem; }
  .chip.ready { background: rgb(110 231 168 / 0.12); color: var(--success); }
  .chip .dot { width: 6px; height: 6px; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .suffix { position: relative; display: block; }
  .suffix span { position: absolute; right: 13px; top: 50%; translate: 0 -50%; color: var(--text-3); font-size: 0.84rem; pointer-events: none; }
  @media (max-width: 480px) { .two { grid-template-columns: 1fr; } .chip { display: none; } }
</style>
