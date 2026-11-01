<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import Books from "phosphor-svelte/lib/Books";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import DotsThree from "phosphor-svelte/lib/DotsThree";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
  import PencilSimple from "phosphor-svelte/lib/PencilSimple";
  import Plus from "phosphor-svelte/lib/Plus";
  import Stack from "phosphor-svelte/lib/Stack";
  import Television from "phosphor-svelte/lib/Television";
  import Trash from "phosphor-svelte/lib/Trash";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { untrack } from "svelte";
  import { cache, graphql, UpdateLibraryConfigStore } from "$houdini";
  import { timecode, titleCase } from "$lib/format.js";
  import { request } from "$lib/graphql.js";
  import { ask, attempt, poll, toast } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Tip from "$components/ui/Tip.svelte";
  import { ago } from "$components/account/relative.js";
  import LibraryForm from "./LibraryForm.svelte";

  /** `admin` is the page's Admin store. */
  let { admin, creating = $bindable(false) } = $props();

  const scanStatus = graphql(`
    query LibraryScanStatus($id: ID!) @cache(policy: NetworkOnly) { libraryScanStatus(id: $id) { ...ScanStatusFields } }
  `);
  const triggerScan = graphql(`
    mutation TriggerLibraryScan($id: ID!) { triggerLibraryScan(id: $id) { ...ScanStatusFields } }
  `);
  const deleteLibrary = graphql(`
    mutation DeleteLibrary($id: ID!) { deleteLibrary(id: $id) }
  `);
  const updateConfig = new UpdateLibraryConfigStore();

  const KIND_ICON = { MOVIES: FilmStrip, SHOWS: Television, MUSIC: MusicNotes, AUDIOBOOKS: BookOpen, PHOTOS: ImageIcon, MIXED: Stack };
  const SCHEDULE = { DISABLED: "Manual only", HOURLY: "Hourly", DAILY: "Daily", WEEKLY: "Weekly" };

  let libraries = $derived($admin.data?.libraries ?? []);
  let providers = $derived(($admin.data?.plugins ?? []).filter((plugin) => plugin.hooks.includes("METADATA")).map((plugin) => plugin.id));
  let scans = $state({});
  let starting = $state("");
  let editing = $state({ open: false, library: null });
  let now = $state(Date.now());

  async function status(library) {
    try {
      const next = (await request(scanStatus, { id: library.id })).libraryScanStatus;
      const previous = scans[library.id];
      scans[library.id] = next;
      if (previous?.state === "RUNNING" && next.state !== "RUNNING") finished(library, next);
    } catch { /* status is decorative; the card still works without it */ }
  }

  function finished(library, scan) {
    // A scan changes what every work list holds.
    cache.markStale();
    if (scan.state === "FAILED") { toast(`Scan of ${library.name} failed: ${scan.error ?? "unknown error"}`, "error"); return; }
    const added = scan.summary?.added ?? 0;
    toast(`${library.name} is up to date${added ? ` — ${added} new ${added === 1 ? "item" : "items"}` : ""}`, "success");
  }

  let ids = $derived(libraries.map((library) => library.id).join());
  $effect(() => {
    ids;
    untrack(() => { for (const library of libraries) if (!(library.id in scans)) status(library); });
  });

  let running = $derived(libraries.filter((library) => scans[library.id]?.state === "RUNNING").map((library) => library.id).join());
  $effect(() => {
    if (!running) return;
    const active = running.split(",");
    const stop = poll(() => { for (const library of untrack(() => libraries)) if (active.includes(library.id)) status(library); }, 2500);
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => { stop(); clearInterval(tick); };
  });

  async function scan(library) {
    starting = library.id;
    const result = (await attempt(() => triggerScan.mutate({ id: library.id })))?.data;
    starting = "";
    if (!result) return;
    scans[library.id] = result.triggerLibraryScan;
    now = Date.now();
    if (result.triggerLibraryScan.state !== "RUNNING") finished(library, result.triggerLibraryScan);
  }

  async function toggleWatch(library, input) {
    const watch = input.checked;
    const c = library.config;
    const result = await attempt(() => updateConfig.mutate({ id: library.id, languages: c.preferredLanguages, providers: c.preferredProviders, excluded: c.excludedFolders, watch, schedule: c.scanSchedule }), watch ? `Watching ${library.name} for changes` : `Stopped watching ${library.name}`);
    if (!result) input.checked = !watch;
  }

  function saved(library, options = {}) {
    if (options.scan) scan(library);
  }

  async function remove(library) {
    const answer = await ask({ title: `Delete “${library.name}”?`, message: "Its titles, artwork and everyone’s watch history for it leave NoFlex. Media files stay on disk.", confirmLabel: "Delete library", danger: true });
    if (!answer.confirmed) return;
    if (!(await attempt(() => deleteLibrary.mutate({ id: library.id }), `${library.name} deleted`))) return;
    cache.get("Library", { id: library.id }).delete();
    cache.markStale();
  }

  const elapsed = (scan) => (scan.startedAt ? timecode(Math.max(0, now - Date.parse(scan.startedAt))) : "");
  const summary = (s) => (s ? `${s.added} added · ${s.updated} updated · ${s.removed} removed${s.errored ? ` · ${s.errored} errors` : ""}` : "");
</script>

{#if ($admin.fetching && !$admin.data)}
  <div class="libs" aria-busy="true">
    {#each Array(2) as _}
      <div class="panel"><div class="skeleton head"></div><div class="skeleton line"></div><div class="skeleton line short"></div><div class="skeleton line"></div></div>
    {/each}
  </div>
{:else if !libraries.length}
  <State icon={Books} title="No libraries yet" message="A library is a set of folders holding one kind of media — films, shows, music, audiobooks or photos.">
    <button class="btn primary sm" onclick={() => (creating = true)}><Plus size={15} />Add your first library</button>
  </State>
{:else}
  <div class="libs stagger">
    {#each libraries as library, index (library.id)}
      {@const Glyph = KIND_ICON[library.kind] ?? Books}
      {@const scanState = scans[library.id]}
      {@const isRunning = scanState?.state === "RUNNING"}
      <article class="panel lib" class:running={isRunning} style:--i={index}>
        {#if isRunning}<div class="scanbar" aria-hidden="true"><span></span></div>{/if}
        <header class="panel-head">
          <span class="panel-icon"><Glyph size={19} /></span>
          <div class="title">
            <h2><a href="/libraries/{library.id}">{library.name}</a></h2>
            <p>{titleCase(library.kind)} · {library.paths.length} {library.paths.length === 1 ? "folder" : "folders"}</p>
          </div>
          <span class="spacer"></span>
          <span class="status" aria-live="polite">
            {#if isRunning}
              <span class="chip accent"><span class="spinner sm"></span>Scanning <span class="tabular">{elapsed(scanState)}</span></span>
            {:else if scanState?.state === "FAILED"}
              <Tip text={scanState.error ?? "Unknown error"}>{#snippet children(props)}<button {...props} class="chip danger" onclick={() => status(library)}><WarningCircle size={13} />Scan failed</button>{/snippet}</Tip>
            {:else if scanState?.state === "COMPLETED"}
              <Tip text={summary(scanState.summary)}>{#snippet children(props)}<button {...props} class="chip" onclick={() => status(library)}><span class="dot on"></span>Scanned {ago(scanState.finishedAt)}</button>{/snippet}</Tip>
            {/if}
          </span>
          <Menu width={220}>
            {#snippet trigger(props)}<button {...props} class="icon-btn sm" aria-label="Manage {library.name}"><DotsThree size={18} weight="bold" /></button>{/snippet}
            <MenuItem icon={PencilSimple} onSelect={() => (editing = { open: true, library })}>Edit library…</MenuItem>
            <MenuItem icon={ArrowRight} href="/libraries/{library.id}">Browse</MenuItem>
            <MenuItem icon={ArrowsClockwise} disabled={isRunning} onSelect={() => scan(library)}>Scan now</MenuItem>
            <div class="nf-menu-sep"></div>
            <MenuItem icon={Trash} danger onSelect={() => remove(library)}>Delete library</MenuItem>
          </Menu>
        </header>

        <dl class="facts">
          <div class="full">
            <dt>Folders</dt>
            <dd class="paths">{#each library.paths as path}<code>{path}</code>{/each}</dd>
          </div>
          <div>
            <dt>Metadata</dt>
            <dd class="chain">
              {#each library.config.preferredProviders as provider, position}{#if position}<CaretRight size={13} />{/if}<span class="chip">{provider}</span>{:else}<span class="faint">Default order</span>{/each}
            </dd>
          </div>
          <div><dt>Rescans</dt><dd>{SCHEDULE[library.config.scanSchedule] ?? titleCase(library.config.scanSchedule)}</dd></div>
          {#if library.config.preferredLanguages.length}<div><dt>Languages</dt><dd>{library.config.preferredLanguages.join(", ")}</dd></div>{/if}
          {#if library.config.excludedFolders.length}<div><dt>Excluded</dt><dd class="mono small">{library.config.excludedFolders.join(", ")}</dd></div>{/if}
        </dl>

        {#if scanState?.state === "COMPLETED" && scanState.summary}
          <p class="summary tabular"><CheckCircle size={14} />{summary(scanState.summary)}</p>
        {/if}

        <footer>
          <label class="watch"><input type="checkbox" class="switch" checked={library.config.watchEnabled} onchange={(event) => toggleWatch(library, event.currentTarget)} />Watch for changes</label>
          <span class="spacer"></span>
          <button class="btn sm" onclick={() => (editing = { open: true, library })}>Edit</button>
          <button class="btn sm primary" onclick={() => scan(library)} disabled={isRunning || starting === library.id}>
            {#if isRunning || starting === library.id}<span class="spinner sm"></span>{:else}<ArrowsClockwise size={15} />{/if}{isRunning ? "Scanning…" : "Scan now"}
          </button>
        </footer>
      </article>
    {/each}
  </div>
{/if}

<LibraryForm bind:open={editing.open} library={editing.library} {providers} onsaved={(library) => saved(library)} />
<LibraryForm bind:open={creating} {providers} onsaved={(library, options) => saved(library, options)} />

<style>
  .libs { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 460px), 1fr)); gap: 18px; }
  .skeleton.head { height: 38px; width: 55%; margin-bottom: 22px; border-radius: 11px; }
  .skeleton.line { height: 16px; margin-top: 14px; border-radius: 6px; }
  .skeleton.short { width: 60%; }
  .lib { position: relative; overflow: hidden; display: flex; flex-direction: column; transition: border-color var(--t); }
  .lib.running { border-color: rgb(212 255 90 / 0.25); }
  .scanbar { position: absolute; inset: 0 0 auto; height: 2px; overflow: hidden; background: var(--accent-soft); }
  .scanbar span { position: absolute; inset: 0 auto 0 0; width: 35%; background: var(--accent); animation: scan 1.5s var(--ease-in-out) infinite; }
  @keyframes scan { from { transform: translateX(-100%); } to { transform: translateX(300%); } }
  .panel-head { align-items: center; }
  .title { min-width: 0; }
  .title h2 { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .title a:hover { text-decoration: underline; text-underline-offset: 3px; text-decoration-color: var(--text-3); }
  .status { display: contents; }
  .status .chip { height: 28px; }
  .status button.chip { cursor: pointer; }
  .chip .spinner { width: 12px; height: 12px; border-width: 1.6px; }
  .chip .dot { width: 6px; height: 6px; box-shadow: none; }
  .facts { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 16px 22px; flex: 1; }
  .facts .full { grid-column: 1 / -1; }
  dt { font-size: 0.72rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-3); margin-bottom: 5px; }
  dd { font-size: 0.9rem; font-weight: 550; overflow-wrap: anywhere; }
  dd.small { font-size: 0.8rem; }
  .paths { display: grid; gap: 4px; }
  .paths code { display: block; padding: 6px 10px; border-radius: 8px; background: rgb(0 0 0 / 0.25); color: var(--text-2); overflow-wrap: anywhere; }
  .chain { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; color: var(--text-3); }
  .chain .chip { height: 24px; font-family: ui-monospace, "SF Mono", Menlo, monospace; }
  .summary { display: flex; align-items: center; gap: 7px; margin-top: 16px; color: var(--text-3); font-size: 0.82rem; }
  .summary :global(svg) { color: var(--success); }
  footer { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-top: 18px; padding-top: 16px; border-top: 1px solid var(--line); }
  .watch { display: inline-flex; align-items: center; gap: 10px; font-size: 0.88rem; color: var(--text-2); cursor: pointer; }
</style>
