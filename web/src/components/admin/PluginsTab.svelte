<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowSquareOut from "phosphor-svelte/lib/ArrowSquareOut";
  import ArrowsDownUp from "phosphor-svelte/lib/ArrowsDownUp";
  import Broom from "phosphor-svelte/lib/Broom";
  import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
  import GearSix from "phosphor-svelte/lib/GearSix";
  import PencilSimple from "phosphor-svelte/lib/PencilSimple";
  import Play from "phosphor-svelte/lib/Play";
  import Plug from "phosphor-svelte/lib/Plug";
  import { AdminPluginEventsStore, ClearPluginOverrideStore, graphql } from "$houdini";
  import { webUrl } from "$lib/api.js";
  import { titleCase } from "$lib/format.js";
  import { nextPage, refetch } from "$lib/graphql.js";
  import { attempt, confirmAction, poll, toast } from "$lib/ui.svelte.js";
  import ProviderLogo from "$components/ProviderLogo.svelte";
  import State from "$components/State.svelte";
  import Dialog from "$components/ui/Dialog.svelte";
  import Select from "$components/ui/Select.svelte";
  import { ago, interval } from "$components/account/relative.js";
  import { stamp } from "./status/fmt.js";
  import { liveStatus, useLiveStatus } from "./status/live.svelte.js";
  import PluginSettings from "./PluginSettings.svelte";

  /** `admin` is the page's Admin store. */
  let { admin } = $props();

  const enqueueTask = graphql(`
    mutation EnqueuePluginTask($plugin: String!, $task: String!, $payload: String) {
      enqueuePluginTask(pluginId: $plugin, task: $task, payload: $payload)
    }
  `);
  // The whole table comes back, so it replaces the cached one.
  const setOverride = graphql(`
    mutation SetPluginOverride($plugin: String!, $hook: HookKind!, $kind: WorkKindFilter!, $priority: Int!) {
      setPluginOverride(pluginId: $plugin, hook: $hook, workKind: $kind, priority: $priority) { pluginId hook workKind priority adminSet }
    }
  `);

  const removeData = graphql(`
    mutation RemovePluginData($plugin: String!) {
      removePluginData(pluginId: $plugin) { tagAssignments collections }
    }
  `);
  const WRITES = ["library.tags.write", "library.collections.write"];
  const writesLibrary = (plugin) => plugin.capabilities.some((capability) => WRITES.includes(capability));
  const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;

  async function removePluginData(plugin) {
    const name = plugin.displayName;
    if (!(await confirmAction({ title: `Remove ${name}’s library data?`, message: `Its tags and the collections it syncs are deleted. Collections you detached, and the watch history it imported for people, stay. ${name} writes them again at its next sync while it’s installed.`, confirmLabel: "Remove data", danger: true }))) return;
    const result = await attempt(() => removeData.mutate({ plugin: plugin.id }));
    const removed = result?.data?.removePluginData;
    if (removed) toast(`Removed ${count(removed.tagAssignments, "tag", "tags")} and ${count(removed.collections, "collection", "collections")}`, "success");
  }

  const HOOKS = ["METADATA", "IMAGE", "SUBTITLE", "POST_SCAN", "NOTIFIER", "USER_LINK", "AUTH", "SCROBBLER", "EMBEDDER", "BACKGROUND"];
  const KINDS = ["MOVIE", "SERIES", "SEASON", "EPISODE", "ALBUM", "TRACK", "ARTIST", "AUDIO_BOOK", "PHOTO"];
  const hookLabel = (hook) => (hook === "POST_SCAN" ? "Post-scan" : hook === "USER_LINK" ? "Account link" : titleCase(hook));

  let plugins = $derived([...($admin.data?.plugins ?? [])].sort((a, b) => Number(b.circuitOpen) - Number(a.circuitOpen) || a.displayName.localeCompare(b.displayName)));
  let overrides = $derived([...($admin.data?.pluginOverrides ?? [])].sort((a, b) => a.hook.localeCompare(b.hook) || a.workKind.localeCompare(b.workKind) || b.priority - a.priority));
  let pluginName = $derived(new Map(plugins.map((plugin) => [plugin.id, plugin.displayName])));
  let credits = $derived(new Map(($admin.data?.providerAttributions ?? []).map((credit) => [credit.provider, credit])));

  function health(plugin) {
    if (plugin.circuitOpen) return { dot: "bad", label: "Circuit open", detail: "Disabled after repeated crashes — reload the plugin to recover." };
    if (plugin.tasks.some((task) => task.consecutiveFailures > 0)) return { dot: "warn", label: "Task failing", detail: "A background task failed on its last run." };
    if (plugin.instanceRestarts > 0) return { dot: "warn", label: "Recovered", detail: `Restarted ${plugin.instanceRestarts} ${plugin.instanceRestarts === 1 ? "time" : "times"} after a crash.` };
    return { dot: "on", label: "Healthy", detail: "" };
  }

  // ---- Settings ----
  let settingsOpen = $state(false);
  let settingsId = $state(null);
  let settingsPlugin = $derived(plugins.find((plugin) => plugin.id === settingsId) ?? null);

  // ---- Run task ----
  let run = $state({ open: false, plugin: null, task: null, payload: "" });
  let running = $state(false);

  async function runTask(event) {
    event.preventDefault();
    running = true;
    const { plugin, task, payload } = run;
    const ok = await attempt(() => enqueueTask.mutate({ plugin: plugin.id, task: task.name, payload: payload.trim() || null }), `${task.name} queued on ${plugin.displayName}`);
    running = false;
    if (ok) { run.open = false; setTimeout(() => refetch(admin), 1500); }
  }

  const clearOverride = new ClearPluginOverrideStore();
  const events = new AdminPluginEventsStore();
  let eventEdges = $derived($events.data?.pluginEvents.edges ?? []);
  let olderEvents = $state(false);
  $effect(() => useLiveStatus());
  $effect(() => {
    refetch(events);
    if (olderEvents) return;
    return poll(() => refetch(events), 10_000);
  });
  let pluginHealth = $derived(liveStatus.status?.plugins);

  const EVENT = {
    DISCOVERED: { tone: "info", label: "Discovered" },
    LOADED: { tone: "ok", label: "Loaded" },
    LOAD_FAILED: { tone: "bad", label: "Load failed" },
    UNLOADED: { tone: "", label: "Unloaded" },
    TRAPPED: { tone: "bad", label: "Trapped" },
  };
  const UNLOAD = { FileRemoved: "file removed", ReplacedByHotReload: "replaced by hot reload", TooManyTraps: "too many crashes", AdminRequested: "unloaded by an admin", Shutdown: "server shutdown" };
  const fileName = (path) => path?.split("/").pop() ?? "";

  async function olderPage() {
    olderEvents = true;
    await attempt(() => nextPage(events));
  }

  async function reset(row) {
    const name = pluginName.get(row.pluginId) ?? row.pluginId;
    if (!(await confirmAction({ title: `Reset ${name}’s ${hookLabel(row.hook)} priority?`, message: `The admin-set priority for ${titleCase(row.workKind)} goes away and the manifest or config default applies again.`, confirmLabel: "Reset" }))) return;
    if (await attempt(() => clearOverride.mutate({ plugin: row.pluginId, hook: row.hook, kind: row.workKind }), "Priority reset to default")) refetch(admin);
  }

  // ---- Overrides ----
  let override = $state({ plugin: "", hook: "", kind: "MOVIE", priority: "100" });
  let saving = $state(false);
  let hookChoices = $derived.by(() => {
    const plugin = plugins.find((item) => item.id === override.plugin);
    return (plugin ? plugin.hooks : HOOKS).map((hook) => ({ value: hook, label: hookLabel(hook) }));
  });
  $effect(() => { if (override.hook && !hookChoices.some((item) => item.value === override.hook)) override.hook = ""; });

  async function saveOverride(event) {
    event.preventDefault();
    saving = true;
    const result = await attempt(() => setOverride.mutate({ plugin: override.plugin, hook: override.hook, kind: override.kind, priority: Number(override.priority) }), "Priority override saved");
    saving = false;
    if (result) refetch(admin);
  }

  function edit(row) {
    override = { plugin: row.pluginId, hook: row.hook, kind: row.workKind, priority: String(row.priority) };
    document.getElementById("override-priority")?.focus();
  }
</script>

{#if ($admin.fetching && !$admin.data)}
  <div class="plugins" aria-busy="true">{#each Array(3) as _}<div class="panel"><div class="skeleton head"></div><div class="skeleton line"></div><div class="skeleton line short"></div></div>{/each}</div>
{:else if !plugins.length}
  <div class="panel empty-panel"><State compact icon={Plug} title="No plugins loaded" message="Plugins are WASM components that add metadata providers, sign-in methods, scrobblers and automation. Drop a .wasm into the server’s plugin folder and it loads automatically." /></div>
{:else}
  {#if pluginHealth}
    <div class="summary">
      <span class="state ok">{pluginHealth.loaded - pluginHealth.circuitOpen} healthy</span>
      {#if pluginHealth.circuitOpen}<span class="state bad">{pluginHealth.circuitOpen} disabled</span>{/if}
      <span class="state" class:warn={pluginHealth.instanceRestarts}>{pluginHealth.instanceRestarts} restarts</span>
      <span class="state" class:bad={pluginHealth.traps}>{pluginHealth.traps} traps since start</span>
      <span class="state" class:bad={pluginHealth.loadFailures}>{pluginHealth.loadFailures} load failures</span>
    </div>
  {/if}
  <div class="plugins stagger">
    {#each plugins as plugin, index (plugin.id)}
      {@const vital = health(plugin)}
      {@const credit = credits.get(plugin.id)}
      <article class="panel plugin" class:broken={plugin.circuitOpen} style:--i={index}>
        <header class="panel-head">
          <span class="panel-icon" class:mark={credit?.logoUrl}>{#if credit?.logoUrl}<ProviderLogo {credit} height={22} />{:else}<Plug size={19} />{/if}</span>
          <div class="title">
            <h3>{plugin.displayName}</h3>
            <p><code>{plugin.id}</code> · v{plugin.version}</p>
          </div>
          <span class="spacer"></span>
          <span class="health" title={vital.detail || undefined}><span class="dot {vital.dot}"></span>{vital.label}</span>
        </header>
        {#if vital.detail}<p class="detail" class:bad={plugin.circuitOpen}>{vital.detail}</p>{/if}

        <div class="hooks">{#each plugin.hooks as hook}<span class="chip">{hookLabel(hook)}</span>{/each}</div>

        <dl class="facts">
          <div><dt>Plugin API</dt><dd class="mono">{plugin.apiVersion}</dd></div>
          <div><dt>Restarts</dt><dd class="tabular" class:warn={plugin.instanceRestarts > 0}>{plugin.instanceRestarts}</dd></div>
          {#if plugin.author}<div><dt>Author</dt><dd>{plugin.author}</dd></div>{/if}
          {#if plugin.license}<div><dt>License</dt><dd>{plugin.license}</dd></div>{/if}
          <div class="full"><dt>Capabilities</dt><dd class="caps">{#each plugin.capabilities as capability}<code>{capability}</code>{:else}<span class="faint">None requested</span>{/each}</dd></div>
        </dl>

        {#if plugin.tasks.length}
          <div class="tasks">
            <p class="eyebrow">Background tasks</p>
            <ul class="list">
              {#each plugin.tasks as task (task.name)}
                <li>
                  <span class="what">
                    <strong>{task.name}</strong>
                    <small>{interval(task.intervalMs)} · last run {ago(task.lastRunAt)} · {task.totalRuns} {task.totalRuns === 1 ? "run" : "runs"}{#if task.consecutiveFailures}<span class="fail"> · {task.consecutiveFailures} failing</span>{/if}</small>
                  </span>
                  <button class="btn ghost sm" onclick={() => (run = { open: true, plugin, task, payload: "" })} disabled={plugin.circuitOpen}><Play size={12} weight="fill" />Run now</button>
                </li>
              {/each}
            </ul>
          </div>
        {/if}

        {#if plugin.settings.length || webUrl(plugin.homepage) || writesLibrary(plugin)}
          <div class="foot">
            {#if plugin.settings.length}<button class="btn sm" onclick={() => { settingsId = plugin.id; settingsOpen = true; }}><GearSix size={15} />Settings</button>{/if}
            {#if writesLibrary(plugin)}<button class="btn danger sm" onclick={() => removePluginData(plugin)}><Broom size={15} />Remove its data</button>{/if}
            {#if webUrl(plugin.homepage)}<a class="home" href={webUrl(plugin.homepage)} target="_blank" rel="noopener noreferrer"><ArrowSquareOut size={14} />Homepage</a>{/if}
          </div>
        {/if}
      </article>
    {/each}
  </div>
{/if}

{#if !($admin.fetching && !$admin.data) && plugins.length}
  <section class="section">
    <div class="panel">
      <header class="panel-head">
        <span class="panel-icon"><ArrowsDownUp size={19} /></span>
        <div><h2>Priority overrides</h2><p>Decide which plugin runs first for a hook and type of title. Higher priority wins; admin-set rows can be reset to the manifest or config default.</p></div>
      </header>
      {#if overrides.length}
        <div class="table-wrap">
          <table>
            <thead><tr><th scope="col">Plugin</th><th scope="col">Hook</th><th scope="col">Applies to</th><th scope="col">Set by</th><th scope="col" class="num">Priority</th><th><span class="sr-only">Actions</span></th></tr></thead>
            <tbody>
              {#each overrides as row (`${row.pluginId}:${row.hook}:${row.workKind}`)}
                <tr>
                  <td><strong>{pluginName.get(row.pluginId) ?? row.pluginId}</strong></td>
                  <td>{hookLabel(row.hook)}</td>
                  <td>{titleCase(row.workKind)}</td>
                  <td>{#if row.adminSet}<span class="state accent">Admin</span>{:else}<span class="faint">Default</span>{/if}</td>
                  <td class="num tabular">{row.priority}</td>
                  <td class="num actions">
                    <button class="icon-btn sm" aria-label="Edit override for {pluginName.get(row.pluginId) ?? row.pluginId}" onclick={() => edit(row)}><PencilSimple size={15} /></button>
                    {#if row.adminSet}<button class="icon-btn sm" aria-label="Reset {pluginName.get(row.pluginId) ?? row.pluginId} to its default priority" title="Reset to default" onclick={() => reset(row)}><ClockCounterClockwise size={15} /></button>{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="empty-note">No overrides — plugins run in each library’s provider order.</p>
      {/if}
      <form class="override-form" onsubmit={saveOverride}>
        <label class="field">Plugin<Select bind:value={override.plugin} items={plugins.map((plugin) => ({ value: plugin.id, label: plugin.displayName, hint: plugin.id }))} placeholder="Choose…" label="Plugin" /></label>
        <label class="field">Hook<Select bind:value={override.hook} items={hookChoices} placeholder="Choose…" label="Hook" /></label>
        <label class="field">Applies to<Select bind:value={override.kind} items={KINDS.map((kind) => ({ value: kind, label: titleCase(kind) }))} label="Work kind" /></label>
        <label class="field">Priority<input id="override-priority" type="number" step="1" bind:value={override.priority} required /></label>
        <button class="btn primary" disabled={saving || !override.plugin || !override.hook || override.priority === ""}>{#if saving}<span class="spinner sm"></span>{/if}Save</button>
      </form>
    </div>
  </section>
{/if}

{#if !($admin.fetching && !$admin.data)}
  <section class="section">
    <div class="panel">
      <header class="panel-head">
        <span class="panel-icon"><ClockCounterClockwise size={19} /></span>
        <div><h2>Lifecycle events</h2><p>Discovery, loads, load failures, unloads and crashes since the server started, newest first.</p></div>
      </header>
      {#if eventEdges.length}
        <ol class="events">
          {#each eventEdges as { node } (node.id)}
            {@const kind = EVENT[node.event.kind] ?? { tone: "", label: node.event.kind }}
            <li>
              <time class="tabular" datetime={node.at} title={stamp(node.at)}>{ago(node.at)}</time>
              <span class="state {kind.tone}">{kind.label}</span>
              <span class="ev">
                <strong>{node.event.pluginId ? (pluginName.get(node.event.pluginId) ?? node.event.pluginId) : fileName(node.event.path)}</strong>
                <small>
                  {#if node.event.kind === "TRAPPED"}in {hookLabel(node.event.hooks[0] ?? "")} hook: <span class="mono">{node.event.trap}</span>
                  {:else if node.event.kind === "LOAD_FAILED"}<span class="mono">{node.event.path}</span> — {node.event.reason}
                  {:else if node.event.kind === "UNLOADED"}{UNLOAD[node.event.reason] ?? node.event.reason}
                  {:else if node.event.hooks.length}{node.event.hooks.map(hookLabel).join(", ")}{#if node.event.sha256}{" · "}<span class="mono">{node.event.sha256.slice(0, 12)}</span>{/if}
                  {/if}
                </small>
              </span>
            </li>
          {/each}
        </ol>
        {#if $events.pageInfo?.hasNextPage}<div class="more-row"><button class="btn sm" onclick={olderPage}>Load older</button></div>{/if}
      {:else}
        <p class="empty-note">{$events.fetching ? "Loading…" : "No lifecycle events recorded since the server started."}</p>
      {/if}
    </div>
  </section>
{/if}

<PluginSettings bind:open={settingsOpen} plugin={settingsPlugin} />

<Dialog bind:open={run.open} title="Run {run.task?.name ?? 'task'} now" description="Queues one extra run of {run.plugin?.displayName ?? 'this plugin'}’s task, outside its regular schedule." width={480}>
  <form id="task-form" class="stack" onsubmit={runTask}>
    <label class="field">Payload<textarea class="mono" bind:value={run.payload} rows="4" placeholder="Optional — a string or JSON the task understands" spellcheck="false"></textarea><small>Leave empty for a normal run.</small></label>
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (run.open = false)}>Cancel</button>
    <button class="btn primary" form="task-form" disabled={running}>{#if running}<span class="spinner sm"></span>{:else}<Play size={14} weight="fill" />{/if}Run now</button>
  {/snippet}
</Dialog>

<style>
  .plugins { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 380px), 1fr)); gap: 18px; }
  .skeleton.head { height: 38px; width: 55%; margin-bottom: 22px; border-radius: 11px; }
  .skeleton.line { height: 16px; margin-top: 14px; border-radius: 6px; }
  .skeleton.short { width: 60%; }
  .plugin { display: flex; flex-direction: column; gap: 14px; }
  .plugin.broken { border-color: rgb(255 107 107 / 0.3); background: linear-gradient(180deg, rgb(255 107 107 / 0.05), transparent 140px), var(--surface); }
  .plugin .panel-head { margin-bottom: 0; align-items: center; }
  .panel-icon.mark { width: auto; padding-inline: 9px; }
  .title { min-width: 0; }
  .title p code { font-size: 0.8rem; }
  .health { display: inline-flex; align-items: center; gap: 8px; font-size: 0.82rem; font-weight: 600; color: var(--text-2); white-space: nowrap; }
  .detail { font-size: 0.84rem; color: var(--warning); margin-top: -4px; }
  .detail.bad { color: var(--danger); }
  .hooks { display: flex; flex-wrap: wrap; gap: 6px; }
  .facts { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 14px 18px; }
  .facts .full { grid-column: 1 / -1; }
  dt { font-size: 0.72rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-3); margin-bottom: 4px; }
  dd { font-size: 0.9rem; font-weight: 550; overflow-wrap: anywhere; }
  dd.warn { color: var(--warning); }
  .caps { display: flex; flex-wrap: wrap; gap: 5px; }
  .caps code { padding: 3px 8px; border-radius: 6px; background: rgb(0 0 0 / 0.25); color: var(--text-2); font-size: 0.76rem; }
  .tasks { border-top: 1px solid var(--line); padding-top: 12px; }
  .tasks .list li { padding-block: 10px; }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what strong { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 0.86rem; }
  .fail { color: var(--warning); }
  .foot { display: flex; flex-wrap: wrap; align-items: center; gap: 14px; margin-top: auto; }
  .home { display: inline-flex; align-items: center; gap: 6px; font-size: 0.84rem; color: var(--text-2); }
  .home:hover { color: var(--text); }
  .table-wrap { overflow-x: auto; margin: 0 -6px 18px; }
  table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
  th { text-align: left; font-size: 0.72rem; font-weight: 600; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-3); padding: 8px 6px; }
  td { padding: 8px 6px; border-top: 1px solid var(--line); color: var(--text-2); white-space: nowrap; }
  td strong { color: var(--text); font-weight: 600; }
  .num { text-align: right; }
  .summary { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 16px; }
  .actions { white-space: nowrap; }
  .events { display: grid; }
  .events li { display: grid; grid-template-columns: 9rem 7.5rem minmax(0, 1fr); align-items: baseline; gap: 12px; padding: 9px 2px; border-top: 1px solid var(--line); font-size: 0.86rem; }
  .events li:first-child { border-top: 0; }
  .events time { color: var(--text-3); font-size: 0.8rem; }
  .events .state { justify-self: start; }
  .ev { min-width: 0; }
  .events small { display: block; color: var(--text-3); font-size: 0.8rem; overflow-wrap: anywhere; }
  .events strong { font-weight: 600; }
  .more-row { display: flex; justify-content: center; margin-top: 12px; }
  @media (max-width: 640px) { .events li { grid-template-columns: auto 1fr; } .events .ev { grid-column: 1 / -1; } }
  .override-form { display: grid; grid-template-columns: 1.3fr 1.1fr 1fr 110px auto; gap: 10px; align-items: end; padding-top: 16px; border-top: 1px solid var(--line); }
  .override-form .btn { height: 42px; }
  @media (max-width: 900px) { .override-form { grid-template-columns: 1fr 1fr; } .override-form .btn { grid-column: 1 / -1; } }
  @media (max-width: 480px) { .override-form { grid-template-columns: 1fr; } }
</style>
