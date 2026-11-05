<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import Copy from "phosphor-svelte/lib/Copy";
  import Devices from "phosphor-svelte/lib/Devices";
  import Gear from "phosphor-svelte/lib/Gear";
  import HardDrives from "phosphor-svelte/lib/HardDrives";
  import Lock from "phosphor-svelte/lib/Lock";
  import Stack from "phosphor-svelte/lib/Stack";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import DownloadSimple from "phosphor-svelte/lib/DownloadSimple";
  import { graphql } from "$houdini";
  import { app } from "$lib/session.svelte.js";
  import { attempt } from "$lib/ui.svelte.js";
  import { ago } from "$components/account/relative.js";
  import { size, stamp, uptime } from "./status/fmt.js";
  import { liveStatus, useLiveStatus } from "./status/live.svelte.js";

  $effect(() => useLiveStatus());

  const setDownloads = graphql(`
    mutation SetDownloadsAllowed($allowed: Boolean!) { setDownloadsAllowed(allowed: $allowed) }
  `);
  async function changeDownloads(event) {
    const allowed = event.currentTarget.checked;
    const result = await attempt(() => setDownloads.mutate({ allowed }), allowed ? "Downloads allowed" : "Downloads turned off");
    // A failure leaves the switch showing what the server still says.
    if (result) app.server = { ...app.server, downloadsAllowed: allowed };
    else event.target.checked = !allowed;
  }

  let s = $derived(liveStatus.status);
  let filter = $state("");
  let config = $derived.by(() => {
    const needle = filter.trim().toLowerCase();
    const rows = s?.server.config ?? [];
    return needle ? rows.filter((row) => row.key.toLowerCase().includes(needle) || (row.value ?? "").toLowerCase().includes(needle)) : rows;
  });
  let redacted = $derived((s?.server.config ?? []).filter((row) => row.redacted).length);
  let copied = $state(false);

  async function copyConfig() {
    const text = (s?.server.config ?? []).map((row) => `${row.key} = ${row.value ?? "(unset)"}`).join("\n");
    try { await navigator.clipboard.writeText(text); copied = true; setTimeout(() => (copied = false), 1600); } catch { /* clipboard denied */ }
  }
</script>

{#if !s}
  <div class="diag"><div class="diag-cols">{#each Array(2) as _}<div class="panel skeleton tall"></div>{/each}</div><div class="panel skeleton tall"></div></div>
{:else}
  {@const db = s.server.database}
  <div class="diag">
    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><DownloadSimple size={18} /></span>
        <div class="grow"><h2>Downloads</h2><p>Whether people signed in here, or linked from another server, can keep copies on their devices. Streaming isn’t affected, and copies already on a device stay there.</p></div>
        <label class="toggle"><input type="checkbox" class="switch" aria-label="Allow downloads" checked={app.server?.downloadsAllowed ?? true} onchange={changeDownloads} /><span><strong>Allow downloads</strong></span></label>
      </header>
    </section>

    <div class="diag-cols">
      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><HardDrives size={18} /></span>
          <div><h2>NoFlex {s.server.version}</h2><p>Up {uptime(s.server.uptimeSeconds)} · started {stamp(s.server.startedAt)}</p></div>
        </header>
        <dl class="kv wide">
          <div><dt>Listening on</dt><dd class="mono">{s.server.bind}</dd></div>
          <div><dt>Public URL</dt><dd class="mono">{s.server.publicUrl ?? "not set"}</dd></div>
          <div class="full"><dt>Data folder</dt><dd class="mono">{s.server.dataDir}</dd></div>
          <div class="full"><dt>Cache folder</dt><dd class="mono">{s.server.cacheDir}</dd></div>
          {#if app.server}<div><dt>Server name</dt><dd>{app.server.name}</dd></div>{/if}
        </dl>
      </section>

      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><Stack size={18} /></span>
          <div><h2>Database</h2><p>{db.backend || "Storage"} engine {db.engineVersion}</p></div>
          <span class="spacer"></span>
          <span class="state" class:ok={db.healthy && !db.error} class:bad={!db.healthy || db.error}>{db.error ? "Unreachable" : db.healthy ? "Healthy" : "Unhealthy"}</span>
        </header>
        {#if db.error}<p class="error"><WarningCircle size={14} />{db.error}</p>{/if}
        <dl class="kv">
          <div><dt>Schema version</dt><dd>{db.schemaVersion}</dd></div>
          <div><dt>Database file</dt><dd>{size(db.sizeBytes)}</dd></div>
          <div><dt>Write-ahead log</dt><dd>{size(db.walSizeBytes)}</dd></div>
          <div class="full"><dt>Path</dt><dd class="mono">{db.path ?? "—"}</dd></div>
          {#if db.notes.length}<div class="full"><dt>Notes</dt><dd>{db.notes.join(" · ")}</dd></div>{/if}
        </dl>
      </section>

      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><Devices size={18} /></span>
          <div><h2>Host</h2><p>{s.host.hostname ?? "Unknown host"}</p></div>
        </header>
        <dl class="kv">
          <div><dt>OS</dt><dd>{s.host.osVersion ?? s.host.osName ?? "—"}</dd></div>
          <div><dt>Kernel</dt><dd class="mono">{s.host.kernelVersion ?? "—"}</dd></div>
          <div><dt>Architecture</dt><dd class="mono">{s.host.arch}</dd></div>
          <div><dt>Booted</dt><dd title={stamp(s.host.bootedAt)}>{ago(s.host.bootedAt)}</dd></div>
        </dl>
      </section>
    </div>

    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><Gear size={18} /></span>
        <div class="grow"><h2>Configuration</h2><p>The resolved config — files plus <code>NOFLEX_*</code> overrides. {redacted ? `${redacted} secret-looking ${redacted === 1 ? "value is" : "values are"} hidden.` : "Secret-looking values are hidden."}</p></div>
        <input class="input search-input" type="search" placeholder="Filter keys or values" bind:value={filter} aria-label="Filter configuration" />
        <button class="btn ghost sm" onclick={copyConfig}>{#if copied}<Check size={14} />{:else}<Copy size={14} />{/if}{copied ? "Copied" : "Copy"}</button>
      </header>
      <div class="dtable-wrap">
        <table class="dtable config">
          <thead><tr><th scope="col">Key</th><th scope="col">Value</th></tr></thead>
          <tbody>
            {#each config as row (row.key)}
              <tr>
                <td class="mono key">{row.key}</td>
                <td class="mono wrap">{#if row.redacted}<span class="state warn"><Lock size={11} />hidden</span>{:else if row.value == null}<span class="faint">unset</span>{:else if row.value === ""}<span class="faint">empty</span>{:else}{row.value}{/if}</td>
              </tr>
            {:else}
              <tr><td colspan="2" class="faint">No keys match.</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  </div>
{/if}

<style>
  .tall { height: 220px; }
  .grow { flex: 1; min-width: 0; }
  .panel-head { flex-wrap: wrap; }
  .error { display: flex; align-items: center; gap: 8px; color: var(--danger); font-size: 0.86rem; margin-bottom: 12px; }
  .config .key { color: var(--text); white-space: nowrap; width: 1%; padding-right: 24px; }
  .config td { padding-block: 6px; }
</style>
