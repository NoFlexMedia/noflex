<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import Books from "phosphor-svelte/lib/Books";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
  import Stack from "phosphor-svelte/lib/Stack";
  import Television from "phosphor-svelte/lib/Television";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { AdminIndexingStore, cache, graphql, TriggerLibraryScanStore } from "$houdini";
  import { timecode, titleCase } from "$lib/format.js";
  import { failure, refetch } from "$lib/graphql.js";
  import { attempt, poll } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import { ago } from "$components/account/relative.js";
  import { stamp } from "./status/fmt.js";

  const indexing = new AdminIndexingStore();
  const triggerScan = new TriggerLibraryScanStore();

  let entries = $derived($indexing.data?.scannerStatus ?? []);
  let search = $derived($indexing.data?.searchIndexStatus);
  let busy = $derived(entries.some((entry) => entry.activeScan || entry.scan.state === "RUNNING"));
  let now = $state(Date.now());
  let starting = $state("");
  let expanded = $state({});

  $effect(() => {
    const ms = busy ? 2500 : 15000;
    const stop = poll(() => refetch(indexing), ms);
    return stop;
  });
  $effect(() => { refetch(indexing); });
  $effect(() => {
    if (!busy) return;
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(tick);
  });

  const endpoint = graphql(`
    query EmbeddingEndpointSettings @cache(policy: NetworkOnly) {
      embeddingEndpoint { url model dimensions requestDimensions queryPrefix documentPrefix hasApiKey }
    }
  `);
  const saveEndpoint = graphql(`
    mutation SaveEmbeddingEndpoint($input: EmbeddingEndpointInput!) {
      setEmbeddingEndpoint(input: $input) { url dimensions }
    }
  `);
  const clearEndpoint = graphql(`
    mutation ClearEmbeddingEndpoint { clearEmbeddingEndpoint }
  `);
  const blank = { url: "", apiKey: "", model: "", requestDimensions: "", queryPrefix: "", documentPrefix: "" };
  let draft = $state({ ...blank });
  let editing = $state(false);
  let saving = $state(false);
  let current = $derived($endpoint.data?.embeddingEndpoint);

  $effect(() => { refetch(endpoint); });

  function edit() {
    draft = current
      ? { ...blank, url: current.url, model: current.model, requestDimensions: current.requestDimensions ?? "", queryPrefix: current.queryPrefix, documentPrefix: current.documentPrefix }
      : { ...blank };
    editing = true;
  }

  async function save(event) {
    event.preventDefault();
    saving = true;
    // An untouched key field keeps the saved key; see the input's note.
    const input = { url: draft.url, model: draft.model, apiKey: draft.apiKey || null, requestDimensions: draft.requestDimensions ? Number(draft.requestDimensions) : null, queryPrefix: draft.queryPrefix, documentPrefix: draft.documentPrefix };
    const saved = await attempt(() => saveEndpoint.mutate({ input }), "Embeddings API saved; reindexing");
    saving = false;
    if (saved) { editing = false; refetch(endpoint); refetch(indexing); }
  }

  async function useLocal() {
    if (await attempt(() => clearEndpoint.mutate(null), "Embedding on this server again")) { editing = false; refetch(endpoint); refetch(indexing); }
  }

  const KIND_ICON = { MOVIES: FilmStrip, SHOWS: Television, MUSIC: MusicNotes, AUDIOBOOKS: BookOpen, PHOTOS: ImageIcon, MIXED: Stack };
  const EVERY = { HOURLY: 3_600_000, DAILY: 86_400_000, WEEKLY: 604_800_000 };
  const SCHEDULE = { DISABLED: "Manual only", HOURLY: "Hourly", DAILY: "Daily", WEEKLY: "Weekly" };

  function lastAt(entry) {
    return entry.lastScan?.finishedAt ?? entry.lastScan?.startedAt ?? entry.scan.finishedAt ?? null;
  }

  function nextScan(entry) {
    const every = EVERY[entry.library.config.scanSchedule];
    if (!every) return { label: "Only when you scan", detail: "" };
    const last = lastAt(entry);
    if (!last) return { label: "Due now", detail: "Never scanned — the scheduler picks it up within 5 minutes." };
    const at = Date.parse(last) + every;
    if (at <= now) return { label: "Due now", detail: "The scheduler checks every 5 minutes." };
    return { label: ago(at, now), detail: stamp(new Date(at).toISOString()) };
  }

  function watcher(entry) {
    if (!entry.library.config.watchEnabled) return { tone: "", label: "Off", detail: "Filesystem watching is disabled for this library." };
    if (entry.watcher.state === "ACTIVE") return { tone: "ok", label: "Watching", detail: entry.watcher.since ? `Since ${stamp(entry.watcher.since)}` : "" };
    if (entry.watcher.state === "FAILED") return { tone: "bad", label: "Failed", detail: entry.watcher.error ?? "The watcher couldn’t start." };
    return { tone: "warn", label: "Not running", detail: "Watching is enabled but no watcher is running." };
  }

  function scanState(entry) {
    if (entry.activeScan || entry.scan.state === "RUNNING") return { tone: "accent", label: "Scanning" };
    if (entry.scan.state === "FAILED") return { tone: "bad", label: "Last scan failed" };
    if (lastAt(entry)) return { tone: "ok", label: "Indexed" };
    return { tone: "", label: "Never scanned" };
  }

  const elapsed = (entry) => {
    const started = entry.activeScan?.startedAt ?? entry.scan.startedAt;
    return started ? timecode(Math.max(0, now - Date.parse(started))) : "";
  };

  async function scan(entry) {
    starting = entry.library.id;
    const result = await attempt(() => triggerScan.mutate({ id: entry.library.id }), `Scanning ${entry.library.name}`);
    starting = "";
    if (result) { now = Date.now(); refetch(indexing); cache.markStale(); }
  }
</script>

{#if $indexing.fetching && !$indexing.data}
  <div class="diag">{#each Array(2) as _}<div class="panel skeleton tall"></div>{/each}</div>
{:else if failure($indexing) && !$indexing.data}
  <State kind="error" title="Couldn’t read indexing state" message={failure($indexing)} action={() => refetch(indexing)} />
{:else}
  <div class="diag">
    {#if !entries.length}
      <State icon={Books} title="No libraries" message="Create a library on the Libraries tab; its scans, watcher and schedule show up here." />
    {/if}
    {#each entries as entry, index (entry.library.id)}
      {@const Glyph = KIND_ICON[lib.kind] ?? Books}
      {@const lib = entry.library}
      {@const st = scanState(entry)}
      {@const w = watcher(entry)}
      {@const next = nextScan(entry)}
      {@const running = st.label === "Scanning"}
      <article class="panel dense lib fade-up" class:running style:animation-delay="{index * 40}ms">
        {#if running}<div class="scanbar" aria-hidden="true"><span></span></div>{/if}
        <header class="panel-head">
          <span class="panel-icon"><Glyph size={18} /></span>
          <div class="title"><h2><a href="/libraries/{lib.id}">{lib.name}</a></h2><p>{titleCase(lib.kind)} · {lib.paths.length} {lib.paths.length === 1 ? "folder" : "folders"}</p></div>
          <span class="spacer"></span>
          <span class="state {st.tone}" aria-live="polite">{#if running}<span class="spinner sm"></span>{/if}{st.label}{#if running} <span class="tabular">{elapsed(entry)}</span>{/if}</span>
          <button class="btn sm" onclick={() => scan(entry)} disabled={running || starting === lib.id}>
            {#if starting === lib.id}<span class="spinner sm"></span>{:else}<ArrowsClockwise size={14} />{/if}Scan now
          </button>
        </header>

        <dl class="kv">
          <div><dt>Schedule</dt><dd>{SCHEDULE[lib.config.scanSchedule] ?? lib.config.scanSchedule}</dd></div>
          <div><dt>Next scan</dt><dd title={next.detail || undefined}>{next.label}</dd></div>
          <div><dt>Last scan</dt><dd title={lastAt(entry) ? stamp(lastAt(entry)) : undefined}>{lastAt(entry) ? ago(lastAt(entry), now) : "Never"}</dd></div>
          <div><dt>Watcher</dt><dd><span class="state {w.tone}" title={w.detail || undefined}>{w.label}</span></dd></div>
        </dl>
        {#if entry.watcher.state === "FAILED" && entry.watcher.error}<p class="error"><WarningCircle size={14} />{entry.watcher.error}</p>{/if}
        {#if entry.scan.state === "FAILED" && entry.scan.error}<p class="error"><WarningCircle size={14} />Scan failed {ago(entry.scan.finishedAt)}: {entry.scan.error}</p>{/if}

        <div class="runs">
          {#if entry.activeScan}
            <div class="run live">
              <p class="eyebrow">Running now</p>
              <p class="counts tabular"><strong>{entry.activeScan.added}</strong> added so far · <strong class:warn-text={entry.activeScan.failures}>{entry.activeScan.failures}</strong> failed</p>
            </div>
          {/if}
          {#if entry.lastScan}
            {@const r = entry.lastScan}
            <div class="run">
              <p class="eyebrow">Last completed run</p>
              <p class="counts tabular"><strong>{r.added}</strong> added · <strong>{r.updated}</strong> updated · <strong>{r.removed}</strong> removed · <strong class:warn-text={r.failures}>{r.failures}</strong> failed</p>
              <p class="faint small">{stamp(r.finishedAt)}{#if r.startedAt && r.finishedAt}{` · took ${timecode(Date.parse(r.finishedAt) - Date.parse(r.startedAt))}`}{/if}</p>
            </div>
          {/if}
        </div>

        <div class="sub-head"><h3>Folders</h3></div>
        <div class="dtable-wrap">
          <table class="dtable">
            <tbody>
              {#each entry.roots as root (root.path)}
                <tr>
                  <td class="mono wrap">{root.path}</td>
                  <td class="num">
                    {#if root.readable}<span class="state ok">Readable</span>
                    {:else if root.exists}<span class="state bad" title={root.error ?? undefined}>{root.isDir ? "Unreadable" : "Not a folder"}</span>
                    {:else}<span class="state bad" title={root.error ?? undefined}>Missing</span>{/if}
                  </td>
                </tr>
                {#if root.error && !root.readable}<tr class="note"><td colspan="2" class="error-cell">{root.error}</td></tr>{/if}
              {/each}
            </tbody>
          </table>
        </div>

        {#if entry.failureCount}
          <div class="sub-head">
            <h3>Files that failed</h3><small>{entry.failureCount} since the server started</small>
            <span class="spacer"></span>
            <button class="btn ghost sm" aria-expanded={!!expanded[lib.id]} onclick={() => (expanded[lib.id] = !expanded[lib.id])}>{expanded[lib.id] ? "Hide" : "Show"}</button>
          </div>
          {#if expanded[lib.id]}
            <ul class="failures">
              {#each entry.recentFailures as failed (failed.at + failed.path + failed.stage)}
                <li>
                  <span class="mono path">{failed.path}</span>
                  <small><strong>{failed.stage.replace(/^scan: /, "")}</strong>{failed.error ? ` — ${failed.error}` : ""} · {ago(failed.at)}</small>
                </li>
              {/each}
            </ul>
          {/if}
        {/if}
      </article>
    {/each}

    {#if search}
      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><MagnifyingGlass size={18} /></span>
          <div><h2>Search index</h2><p>{#if search.remoteHost}Vector embeddings for hybrid search, computed by the API at {search.remoteHost}.{:else}Vector embeddings for hybrid search, computed on this server while it's idle.{/if}</p></div>
          <span class="spacer"></span>
          {#if !search.namespace}<span class="state warn">No embedder</span>
          {:else if search.pendingWorks}<span class="state accent">Backfilling</span>
          {:else}<span class="state ok">Up to date</span>{/if}
        </header>
        {#if search.namespace}
          <dl class="kv">
            {#if search.remoteHost}
              <div><dt>Endpoint</dt><dd class="mono">{search.remoteHost}</dd></div>
            {:else}
              <div><dt>Plugin</dt><dd class="mono">{search.embedderPlugins[0] ?? "—"}</dd></div>
              <div><dt>Running on</dt><dd class="mono">{search.inferenceDevice ?? "Not loaded yet"}</dd></div>
            {/if}
            <div><dt>Model</dt><dd class="mono">{search.modelId ?? "—"}</dd></div>
            <div><dt>Namespace</dt><dd class="mono">{search.namespace}</dd></div>
            <div><dt>Dimensions</dt><dd>{search.dimensions ?? "—"}</dd></div>
            <div><dt>Waiting for vectors</dt><dd class:warn-text={search.pendingWorks}>{search.pendingWorks}{search.pendingIsLowerBound ? "+" : ""} titles</dd></div>
          </dl>
        {:else}
          <p class="empty-note">Search falls back to lexical matching. Install an embedder plugin or use an embeddings API to enable vector and hybrid search.</p>
        {/if}
        {#if editing}
          <form class="endpoint" onsubmit={save}>
            <p class="privacy"><WarningCircle size={18} /><span>An embeddings API receives every title, overview and credit in your libraries, and everything searched for. Saving sends a test, and reindexes the library if the model changes.</span></p>
            <label class="field wide">Base URL<input class="mono" bind:value={draft.url} placeholder="https://api.openai.com/v1" spellcheck="false" autocomplete="off" required /></label>
            <label class="field">Model<input class="mono" bind:value={draft.model} placeholder="text-embedding-3-small" spellcheck="false" autocomplete="off" required /></label>
            <label class="field">API key<input class="mono" type="password" bind:value={draft.apiKey} placeholder={current?.hasApiKey ? "Saved; leave blank to keep" : "None"} autocomplete="off" /><small>A saved key is dropped if the URL changes, so it can't reach another host.</small></label>
            <label class="field">Dimensions<input type="number" min="1" step="1" inputmode="numeric" bind:value={draft.requestDimensions} placeholder="Model's default" /><small>Only for models that can shorten their vectors.</small></label>
            <label class="field">Query prefix<input class="mono" bind:value={draft.queryPrefix} placeholder="None" spellcheck="false" autocomplete="off" /><small>Some models expect one, such as <code>search_query: </code>.</small></label>
            <label class="field">Document prefix<input class="mono" bind:value={draft.documentPrefix} placeholder="None" spellcheck="false" autocomplete="off" /></label>
            <div class="endpoint-actions">
              <button class="btn sm primary" disabled={saving}>{#if saving}<span class="spinner sm"></span>{/if}Test and save</button>
              <button type="button" class="btn sm ghost" onclick={() => (editing = false)}>Cancel</button>
            </div>
          </form>
        {:else}
          <div class="endpoint-actions">
            <button class="btn sm" onclick={edit}>{current ? "Edit embeddings API" : "Use an embeddings API"}</button>
            {#if current}<button class="btn sm ghost" onclick={useLocal}>Embed on this server instead</button>{/if}
          </div>
        {/if}
      </section>
    {/if}
  </div>
{/if}

<style>
  .tall { height: 260px; }
  .endpoint { display: grid; gap: 14px; grid-template-columns: repeat(auto-fit, minmax(min(100%, 240px), 1fr)); margin-top: 16px; }
  .endpoint .wide, .endpoint .privacy { grid-column: 1 / -1; }
  .privacy { display: flex; gap: 10px; padding: 10px 12px; border-radius: var(--r-s); background: rgb(255 255 255 / 0.03); color: var(--warning); font-size: 0.86rem; }
  .privacy :global(svg) { flex: none; margin-top: 2px; }
  .endpoint-actions { grid-column: 1 / -1; display: flex; flex-wrap: wrap; gap: 8px; margin-top: 14px; }
  .lib { position: relative; overflow: hidden; display: grid; gap: 4px; }
  .lib.running { border-color: rgb(212 255 90 / 0.25); }
  .scanbar { position: absolute; inset: 0 0 auto; height: 2px; overflow: hidden; }
  .scanbar span { position: absolute; inset: 0; width: 40%; background: linear-gradient(90deg, transparent, var(--accent), transparent); animation: sweep 1.4s var(--ease-in-out) infinite; }
  @keyframes sweep { from { transform: translateX(-100%); } to { transform: translateX(250%); } }
  .title { min-width: 0; }
  .title h2 a:hover { text-decoration: underline; text-underline-offset: 3px; }
  .panel-head { flex-wrap: wrap; }
  .error { display: flex; align-items: flex-start; gap: 8px; color: var(--danger); font-size: 0.84rem; margin-top: 10px; overflow-wrap: anywhere; }
  .error :global(svg) { margin-top: 3px; flex: none; }
  .runs { display: grid; gap: 12px; grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr)); margin-top: 16px; }
  .run { padding: 12px 14px; border-radius: var(--r); background: rgb(255 255 255 / 0.025); border: 1px solid var(--line); }
  .run.live { border-color: rgb(212 255 90 / 0.25); background: var(--accent-soft); }
  .counts { margin-top: 4px; font-size: 0.88rem; color: var(--text-2); }
  .counts strong { color: var(--text); }
  .small { font-size: 0.78rem; margin-top: 2px; }
  .warn-text { color: var(--warning) !important; }
  .note td { border-top: 0; padding-top: 0; }
  .error-cell { color: var(--danger); font-size: 0.8rem; }
  .failures { display: grid; gap: 8px; max-height: 320px; overflow-y: auto; }
  .failures li { display: grid; gap: 2px; padding: 8px 10px; border-radius: var(--r-s); background: rgb(255 255 255 / 0.025); }
  .failures .path { font-size: 0.78rem; overflow-wrap: anywhere; }
  .failures small { color: var(--text-3); font-size: 0.78rem; }
  .failures small strong { color: var(--warning); font-weight: 600; }
</style>
