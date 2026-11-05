<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { graphql } from "$houdini";
  import { failure, refetch } from "$lib/graphql.js";
  import { servers } from "$lib/servers.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { poll } from "$lib/ui.svelte.js";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";

  const POLL_MS = 5000;

  const activity = graphql(`
    query ServerActivity @cache(policy: NetworkOnly) {
      systemStatus { activeScans plugins { circuitOpen } server { database { healthy } } }
      activeStreams { id user { id name } work { id title } transcode { status } }
    }
  `);
  const scans = graphql(`
    query ServerActivityScans @cache(policy: NetworkOnly) {
      scannerStatus { library { id name } scan { state } activeScan { added updated removed } }
    }
  `);
  const versions = graphql(`
    query ServerActivityVersions @cache(policy: NetworkOnly) {
      optimizedVersions { id status progress }
    }
  `);

  let admin = $derived(!!app.viewer?.isAdmin);
  // Requests follow the active linked server, and this row describes our own.
  let watching = $derived(admin && !servers.active);

  $effect(() => {
    if (!watching) return;
    refetch(activity);
    return poll(() => refetch(activity), POLL_MS);
  });
  // The whole table, every row, so it runs at a gentler pace.
  $effect(() => {
    if (!watching) return;
    refetch(versions);
    return poll(() => refetch(versions), POLL_MS * 3);
  });

  let status = $derived(watching ? $activity.data?.systemStatus : null);
  let scanCount = $derived(status?.activeScans ?? 0);
  // Stats every library root, so only ask while there's a scan to name.
  $effect(() => {
    if (!scanCount) return;
    refetch(scans);
    return poll(() => refetch(scans), POLL_MS);
  });

  const count = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
  const watcher = (stream) => [stream.user.name, stream.work?.title ?? "Live TV"].join(" · ");

  let unreachable = $derived(watching && !!failure($activity));
  let streams = $derived(watching ? $activity.data?.activeStreams ?? [] : []);
  // By the job, not the route: a stream outlives ffmpeg once the output is finished.
  let transcoding = $derived(streams.filter((stream) => stream.transcode?.status === "running"));
  let playing = $derived(streams.filter((stream) => stream.transcode?.status !== "running"));
  let scanning = $derived(scanCount ? ($scans.data?.scannerStatus ?? []).filter((row) => row.activeScan || row.scan.state === "RUNNING") : []);
  // Polled more slowly, so it can outlive the moment the server stops answering.
  let versionRows = $derived(watching && !unreachable ? $versions.data?.optimizedVersions ?? [] : []);
  let optimizing = $derived(versionRows.filter((version) => version.status === "RUNNING"));
  let queued = $derived(versionRows.filter((version) => version.status === "QUEUED").length);

  function scanDetail(rows) {
    const run = rows.length === 1 ? rows[0].activeScan : null;
    return run ? `${run.added} added · ${run.updated} updated · ${run.removed} removed` : "Looking for changes";
  }

  // Outermost last: each entry takes the next ring out from the dot.
  let rings = $derived([
    scanCount && {
      kind: "scan",
      short: "Scanning",
      title: scanning.length === 1 ? `Scanning ${scanning[0].library.name}` : `Scanning ${count(scanCount, "library", "libraries")}`,
      detail: scanDetail(scanning),
      href: "/admin?tab=indexing",
    },
    transcoding.length && {
      kind: "transcode",
      short: `${transcoding.length} transcoding`,
      title: `${count(transcoding.length, "stream")} transcoding`,
      detail: transcoding.map(watcher).join(", "),
      href: "/admin?tab=playback",
    },
    playing.length && {
      kind: "playing",
      short: `${playing.length} playing`,
      title: `${count(playing.length, "stream")} playing`,
      detail: playing.map(watcher).join(", "),
      href: "/admin?tab=playback",
    },
    optimizing.length && {
      kind: "optimize",
      short: "Optimizing",
      title: `Optimizing ${count(optimizing.length, "version")}`,
      detail: queued ? `${queued} more queued` : "Nothing else queued",
      progress: optimizing.reduce((total, version) => total + version.progress, 0) / optimizing.length,
      href: "/admin?tab=transcoding",
    },
  ].filter(Boolean));

  let problems = $derived([
    unreachable && { kind: "bad", title: "Can’t reach the server", detail: "Status will return when it answers.", short: "Can’t reach the server" },
    status?.server.database.healthy === false && { kind: "bad", title: "The database is unhealthy", detail: "See the server tab for details.", short: "Database unhealthy", href: "/admin?tab=server" },
    status?.plugins.circuitOpen > 0 && { kind: "warn", title: `${count(status.plugins.circuitOpen, "plugin")} paused after failures`, detail: "See the plugins tab for details.", short: "Plugin trouble", href: "/admin?tab=plugins" },
  ].filter(Boolean));

  let name = $derived(app.server?.name ?? "NoFlex");
  let version = $derived(`v${app.server?.version}`);
  let summary = $derived(problems[0]?.short ?? (rings.map((ring) => ring.short).join(" · ") || `${version} · ${app.server?.backend}`));
  let described = $derived(`${name}: ${[...problems, ...rings].map((entry) => entry.title).join(", ") || "idle"}`);
</script>

{#snippet row()}
  <span class="pulse" class:busy={rings.length > 0}>
    {#each rings as ring, index (ring.kind)}
      <span class="ring {ring.kind}" style:--slot={index} style:--progress={ring.progress ?? 0}></span>
    {/each}
    <span class="dot" class:on={!problems.length} class:warn={problems[0]?.kind === "warn"} class:bad={problems[0]?.kind === "bad"}></span>
  </span>
  <span class="label"><strong>{name}</strong><small>{summary}</small></span>
{/snippet}

{#snippet entry(item)}
  <span class="entry">
    <span class="swatch {item.kind}"></span>
    <span class="entry-text"><strong>{item.title}</strong><small>{item.detail}</small></span>
  </span>
{/snippet}

{#if admin}
  <Menu side="right" align="end" width={290}>
    {#snippet trigger(props)}
      <button {...props} type="button" class="server" title={described} aria-label="Server activity. {described}">{@render row()}</button>
    {/snippet}
    <div class="nf-menu-label">{name} · {version}</div>
    {#each [...problems, ...rings] as item (item.title)}
      {#if item.href}
        <MenuItem href={item.href}>{@render entry(item)}</MenuItem>
      {:else}
        <div class="static">{@render entry(item)}</div>
      {/if}
    {:else}
      <p class="idle">Nothing is running right now.</p>
    {/each}
    <div class="nf-menu-sep"></div>
    <MenuItem href="/admin">Open administration</MenuItem>
  </Menu>
{:else}
  <div class="server" title="{name} · {version}">{@render row()}</div>
{/if}

<style>
  /* Centered on the rail's axis, with the text in line with the nav labels. */
  .server { display: flex; align-items: center; gap: 4px; width: 100%; min-height: 46px; padding: 4px 8px 4px calc((var(--rail-w) - 24px - 34px) / 2); border-radius: 11px; text-align: left; color: inherit; }
  button.server { transition: background var(--t) var(--ease-out); }
  button.server:hover, button.server[data-state="open"] { background: var(--surface-2); }
  .label { display: grid; flex: 1; min-width: 0; white-space: nowrap; transition: opacity 180ms var(--ease-out) 90ms; }
  :global(:root[data-sidebar="collapsed"]) .label { opacity: 0; transition-duration: 110ms; transition-delay: 0ms; }
  strong, small { display: block; overflow: hidden; text-overflow: ellipsis; }
  .label strong { font-size: 0.84rem; font-weight: 600; }
  .label small { font-size: 0.74rem; color: var(--text-3); }

  .pulse { position: relative; display: grid; place-items: center; flex: none; width: 34px; height: 34px; }
  /* The halo would run into the first ring. */
  .pulse.busy .dot { box-shadow: none; }

  .scan { --tone: var(--focus); }
  .transcode { --tone: var(--accent); }
  .playing { --tone: var(--success); }
  .optimize { --tone: var(--text-2); }
  .warn { --tone: var(--warning); }
  .bad { --tone: var(--danger); }

  /* Each kind differs in shape and motion as well as color, so they stay apart without relying on hue. */
  .ring { --size: calc(16px + var(--slot) * 6px); position: absolute; top: calc(50% - var(--size) / 2); left: calc(50% - var(--size) / 2); width: var(--size); height: var(--size); border-radius: 50%; mask: radial-gradient(farthest-side, transparent calc(100% - 2px), #000 calc(100% - 1.5px)); animation: ring-in 320ms var(--ease-out); }
  .ring.scan { background: conic-gradient(var(--tone) 28%, color-mix(in srgb, var(--tone) 16%, transparent) 0); animation: ring-in 320ms var(--ease-out), orbit 1.8s linear infinite; }
  .ring.transcode { background: repeating-conic-gradient(var(--tone) 0 16%, transparent 16% 25%); animation: ring-in 320ms var(--ease-out), orbit 1.1s linear infinite; }
  .ring.playing { background: var(--tone); animation: ring-in 320ms var(--ease-out), breathe 2.4s var(--ease-in-out) infinite; }
  .ring.optimize { background: conic-gradient(var(--tone) calc(var(--progress) * 100%), color-mix(in srgb, var(--tone) 18%, transparent) 0); }
  @keyframes ring-in { from { opacity: 0; scale: 0.7; } }
  @keyframes orbit { to { rotate: 360deg; } }
  @keyframes breathe { 50% { opacity: 0.45; } }

  .entry { display: flex; align-items: center; gap: 11px; padding-block: 7px; }
  .entry-text { display: grid; min-width: 0; }
  .entry strong { font-weight: 550; color: var(--text); }
  .swatch { flex: none; width: 12px; height: 12px; border: 2px solid var(--tone); border-radius: 50%; }
  .swatch.scan { border-right-color: transparent; border-bottom-color: transparent; }
  .swatch.transcode { border-style: dashed; }
  .static { padding: 0 11px; font-size: 0.9rem; color: var(--text-2); }
  .static small { color: var(--text-3); font-size: 0.78rem; }
  .idle { padding: 6px 11px 8px; font-size: 0.88rem; color: var(--text-3); }

  @media (prefers-reduced-motion: reduce) {
    .label { transition: none; }
    .ring, .ring.scan, .ring.transcode, .ring.playing { animation: none; }
  }
</style>
