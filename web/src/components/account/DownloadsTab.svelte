<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import DownloadSimple from "phosphor-svelte/lib/DownloadSimple";
  import { graphql } from "$houdini";
  import { date } from "$lib/format.js";
  import { refetch } from "$lib/graphql.js";
  import { attempt, confirmAction, poll, toast } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import { ago } from "./relative.js";
  import { saveFile } from "./save-file.js";

  let { account } = $props();

  const cancelExport = graphql(`
    mutation CancelExportJob($id: ID!) { cancelExportJob(id: $id) { ...ExportJobFields } }
  `);

  const ACTIVE = ["QUEUED", "RUNNING", "CANCEL_REQUESTED"];
  const STATUS = {
    QUEUED: ["Queued", ""],
    RUNNING: ["Encoding", "accent"],
    CANCEL_REQUESTED: ["Cancelling", ""],
    COMPLETED: ["Ready", "ready"],
    FAILED: ["Failed", "danger"],
    CANCELLED: ["Cancelled", ""],
  };

  let jobs = $derived([...($account.data?.exportJobs.nodes ?? [])].sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt)));
  let anyActive = $derived(jobs.some((job) => ACTIVE.includes(job.status)));
  let downloading = $state("");

  $effect(() => { if (anyActive) { const store = account; return poll(() => refetch(store), 3000); } });

  const extension = (job) => (job.format === "MKV" ? "mkv" : "mp4");
  const formatLabel = (job) => (job.format === "MKV" ? "MKV" : "MP4");

  async function download(job) {
    downloading = job.id;
    try {
      if (await saveFile(`/download/${job.id}/file`, `noflex-${job.id.slice(0, 8)}.${extension(job)}`)) toast("Download complete", "success");
    } catch (cause) {
      toast(cause instanceof Error ? cause.message : String(cause), "error");
    } finally { downloading = ""; }
  }

  async function cancel(job) {
    if (!(await confirmAction({ title: "Cancel this export?", message: "Any partial output is discarded.", confirmLabel: "Cancel export", danger: true }))) return;
    await attempt(() => cancelExport.mutate({ id: job.id }), "Export cancelled");
  }
</script>

{#if ($account.fetching && !$account.data)}
  <div class="jobs" aria-busy="true">{#each Array(3) as _}<div class="skeleton job-skel"></div>{/each}</div>
{:else if !jobs.length}
  <State icon={DownloadSimple} title="No downloads yet" message="Open any film or episode, choose ••• → Export for offline, and it’ll be prepared here as an MP4 or MKV you can keep.">
    <a class="btn sm" href="/">Browse your libraries</a>
  </State>
{:else}
  <ul class="jobs stagger">
    {#each jobs as job, index (job.id)}
      {@const [label, tone] = STATUS[job.status] ?? [job.status, ""]}
      <li class="panel job" style:--i={index}>
        <span class="icon" class:ready={job.status === "COMPLETED"} class:failed={job.status === "FAILED"}>{#if job.status === "FAILED"}<WarningCircle size={19} />{:else if job.status === "COMPLETED"}<Check size={19} />{:else}<DownloadSimple size={19} />{/if}</span>
        <div class="body">
          <div class="title-row">
            <strong>{formatLabel(job)} export</strong>
            <span class="chip {tone}">{#if job.status === "RUNNING" || job.status === "QUEUED"}<span class="dot" class:on={job.status === "RUNNING"}></span>{/if}{label}</span>
            <code class="faint">edition {job.editionId.slice(0, 8)}</code>
          </div>
          {#if job.status === "FAILED"}
            <p class="error">{job.error || "The encoder stopped unexpectedly."}</p>
          {:else}
            <p>Requested {ago(job.createdAt)}{#if job.finishedAt} · finished {date(job.finishedAt, { dateStyle: "medium", timeStyle: "short" })}{/if}</p>
          {/if}
          {#if ACTIVE.includes(job.status)}
            <div class="progress">
              <div class="progress-bar" class:indeterminate={job.status === "QUEUED"} role="progressbar" aria-label="Export progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(job.progress * 100)}><span style:width="{job.status === 'QUEUED' ? 30 : Math.max(2, job.progress * 100)}%"></span></div>
              <span class="pct tabular">{job.status === "QUEUED" ? "Waiting" : `${Math.round(job.progress * 100)}%`}</span>
            </div>
          {/if}
        </div>
        <div class="actions">
          {#if job.status === "COMPLETED"}
            <button class="btn primary sm" onclick={() => download(job)} disabled={downloading === job.id}>{#if downloading === job.id}<span class="spinner sm"></span>{:else}<DownloadSimple size={15} />{/if}Download</button>
          {:else if job.status === "QUEUED" || job.status === "RUNNING"}
            <button class="btn ghost sm" onclick={() => cancel(job)}>Cancel</button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .jobs { display: grid; gap: 10px; max-width: 920px; }
  .job-skel { height: 86px; border-radius: var(--r-l); }
  .job { display: flex; align-items: center; gap: 16px; padding: 16px 18px; }
  .icon { display: grid; place-items: center; width: 42px; height: 42px; border-radius: 12px; background: var(--surface-2); color: var(--text-2); flex: none; }
  .icon.ready { background: rgb(110 231 168 / 0.12); color: var(--success); }
  .icon.failed { background: var(--danger-soft); color: var(--danger); }
  .body { flex: 1; display: grid; gap: 4px; min-width: 0; }
  .title-row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .title-row strong { font-weight: 600; }
  .title-row code { font-size: 0.76rem; }
  .chip { height: 22px; font-size: 0.72rem; }
  .chip.ready { background: rgb(110 231 168 / 0.12); color: var(--success); }
  .chip .dot { width: 6px; height: 6px; }
  .body p { color: var(--text-3); font-size: 0.84rem; }
  .body p.error { color: var(--danger); overflow-wrap: anywhere; }
  .progress { display: flex; align-items: center; gap: 12px; margin-top: 6px; }
  .progress .progress-bar { flex: 1; }
  .pct { font-size: 0.8rem; color: var(--text-2); font-weight: 600; min-width: 52px; text-align: right; }
  .indeterminate > span { animation: slide 1.4s var(--ease-in-out) infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(340%); } }
  .actions { flex: none; }
  @media (max-width: 560px) {
    .job { flex-wrap: wrap; }
    .actions { flex-basis: 100%; display: flex; justify-content: flex-end; }
  }
</style>
