<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Avatar from "$components/Avatar.svelte";
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import Books from "phosphor-svelte/lib/Books";
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import Eye from "phosphor-svelte/lib/Eye";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import HardDrives from "phosphor-svelte/lib/HardDrives";
  import Play from "phosphor-svelte/lib/Play";
  import Plug from "phosphor-svelte/lib/Plug";
  import SquaresFour from "phosphor-svelte/lib/SquaresFour";
  import Stack from "phosphor-svelte/lib/Stack";
  import Users from "phosphor-svelte/lib/Users";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { AdminActiveStreamsStore, AdminIndexingStore, AdminTranscoderStore } from "$houdini";
  import { fetchQuery, refetch } from "$lib/graphql.js";
  import { poll } from "$lib/ui.svelte.js";
  import { ago } from "$components/account/relative.js";
  import LiveBadge from "./status/LiveBadge.svelte";
  import Meter from "./status/Meter.svelte";
  import Num from "./status/Num.svelte";
  import Spark from "./status/Spark.svelte";
  import { percent, rate, ratio, size, uptime } from "./status/fmt.js";
  import { liveStatus, useLiveStatus } from "./status/live.svelte.js";

  /** `admin` is the page's Admin store; `navigate(tab)` switches tabs. */
  let { admin, navigate } = $props();

  const streams = new AdminActiveStreamsStore();
  const transcoder = new AdminTranscoderStore();
  const indexing = new AdminIndexingStore();

  $effect(() => useLiveStatus());
  $effect(() => {
    fetchQuery(transcoder);
    fetchQuery(indexing);
    const stopStreams = poll(() => refetch(streams), 5000);
    const stopIndexing = poll(() => refetch(indexing), 20000);
    refetch(streams);
    return () => { stopStreams(); stopIndexing(); };
  });

  let s = $derived(liveStatus.status);
  let history = $derived(liveStatus.history);
  let active = $derived($streams.data?.activeStreams ?? []);
  let transcoding = $derived(active.filter((stream) => stream.decision.route === "TRANSCODE"));
  let info = $derived($transcoder.data?.transcoderInfo);
  let libraries = $derived($indexing.data?.scannerStatus ?? []);
  let adminData = $derived($admin.data);

  let memPct = $derived(s ? ratio(s.memory.usedBytes, s.memory.cgroupLimitBytes ?? s.memory.totalBytes) : 0);

  let issues = $derived.by(() => {
    const out = [];
    if (!s) return out;
    if (s.server.database.error || !s.server.database.healthy) out.push({ tone: "bad", icon: HardDrives, title: "Database unhealthy", detail: s.server.database.error ?? s.server.database.notes.join("; "), tab: "server" });
    if (s.logs.errors) out.push({ tone: "bad", icon: WarningCircle, title: `${s.logs.errors} ${s.logs.errors === 1 ? "error" : "errors"} logged since start`, detail: s.logs.lastError ? `${ago(s.logs.lastError.at)}: ${s.logs.lastError.message}` : "", tab: "logs" });
    if (s.plugins.circuitOpen) out.push({ tone: "bad", icon: Plug, title: `${s.plugins.circuitOpen} plugin${s.plugins.circuitOpen === 1 ? "" : "s"} disabled after crashing`, detail: "The circuit breaker opened; reload the plugin to recover.", tab: "plugins" });
    if (s.plugins.loadFailures) out.push({ tone: "warn", icon: Plug, title: `${s.plugins.loadFailures} plugin load failure${s.plugins.loadFailures === 1 ? "" : "s"}`, detail: "A .wasm in the search path couldn’t be loaded.", tab: "plugins" });
    if (s.plugins.traps) out.push({ tone: "warn", icon: Plug, title: `${s.plugins.traps} plugin trap${s.plugins.traps === 1 ? "" : "s"} since start`, detail: "A plugin crashed mid-call and was restarted.", tab: "plugins" });
    if (memPct >= 90) out.push({ tone: memPct >= 96 ? "bad" : "warn", icon: Stack, title: `Memory ${percent(memPct)} used`, detail: `${size(s.memory.availableBytes)} available`, tab: "resources" });
    for (const location of s.storage) {
      const free = ratio(location.availableBytes, location.totalBytes);
      // Missing library roots surface through indexing; other folders are created on first use.
      if (location.error && location.exists) out.push({ tone: "bad", icon: HardDrives, title: `${place(location)} unavailable`, detail: `${location.path}: ${location.error}`, tab: "resources" });
      else if (location.totalBytes && free < 10) out.push({ tone: free < 3 ? "bad" : "warn", icon: HardDrives, title: `${place(location)} nearly full`, detail: `${size(location.availableBytes)} free of ${size(location.totalBytes)} on ${location.mountPoint ?? location.path}`, tab: "resources" });
    }
    if (info) {
      if (info.ffmpeg.error) out.push({ tone: "bad", icon: FilmStrip, title: "ffmpeg not usable", detail: info.ffmpeg.error, tab: "transcoding" });
      if (info.ffprobe.error) out.push({ tone: "bad", icon: FilmStrip, title: "ffprobe not usable", detail: info.ffprobe.error, tab: "transcoding" });
      if (info.fellBack) out.push({ tone: "warn", icon: FilmStrip, title: `Configured ${info.configuredHwaccel} acceleration unavailable`, detail: `Transcoding on ${info.resolvedBackend} instead.`, tab: "transcoding" });
      // A missing encoder costs its codec's rungs, not playback: H.264 always remains.
      for (const encoder of new Set(info.ladder.filter((item) => !item.encoderAvailable).map((item) => item.encoder))) out.push({ tone: "warn", icon: FilmStrip, title: `${encoder} encoder missing`, detail: `It isn’t in this ffmpeg build, so the ladder skips its rungs.`, tab: "transcoding" });
    }
    for (const entry of libraries) {
      const name = entry.library.name;
      if (entry.watcher.state === "FAILED") out.push({ tone: "bad", icon: Eye, title: `${name}: watcher failed`, detail: entry.watcher.error ?? "", tab: "indexing" });
      for (const root of entry.roots.filter((item) => !item.readable)) out.push({ tone: "bad", icon: Books, title: `${name}: folder unreadable`, detail: `${root.path}${root.error ? ` — ${root.error}` : ""}`, tab: "indexing" });
      if (entry.scan.state === "FAILED") out.push({ tone: "warn", icon: ArrowsClockwise, title: `${name}: last scan failed`, detail: entry.scan.error ?? "", tab: "indexing" });
      if (entry.failureCount) out.push({ tone: "warn", icon: WarningCircle, title: `${name}: ${entry.failureCount} file${entry.failureCount === 1 ? "" : "s"} failed to index`, detail: entry.recentFailures[0] ? `${entry.recentFailures[0].stage} — ${entry.recentFailures[0].path}` : "", tab: "indexing" });
    }
    for (const stream of transcoding) {
      if (stream.transcode?.error) out.push({ tone: "bad", icon: Play, title: `Transcode failed for ${stream.user.name}`, detail: stream.transcode.error, tab: "playback" });
      else if (stream.transcode?.speed != null && stream.transcode.speed < 1 && stream.transcode.status === "running") out.push({ tone: "warn", icon: Play, title: `Transcode below realtime (${stream.transcode.speed.toFixed(2)}×)`, detail: `${stream.user.name} · ${stream.work?.title ?? "Live TV"} — playback will buffer.`, tab: "playback" });
    }
    return out.sort((a, b) => (a.tone === b.tone ? 0 : a.tone === "bad" ? -1 : 1));
  });

  // Data, cache and library folders usually share a few filesystems;
  // one row per filesystem reads better than one per folder.
  let filesystems = $derived.by(() => {
    const groups = new Map();
    for (const location of s?.storage ?? []) {
      if (!location.totalBytes) continue;
      const key = location.mountPoint ?? location.path;
      const group = groups.get(key) ?? { key, fileSystem: location.fileSystem, total: location.totalBytes, available: location.availableBytes, used: location.usedBytes, places: [] };
      const label = place(location);
      if (!group.places.includes(label)) group.places.push(label);
      groups.set(key, group);
    }
    return [...groups.values()].sort((a, b) => ratio(b.used, b.total) - ratio(a.used, a.total));
  });

  const BACKEND = { software: "Software", vaapi: "VAAPI", nvenc: "NVENC", qsv: "Quick Sync", videotoolbox: "VideoToolbox", mf: "Media Foundation" };

  function place(location) {
    return ({ DATA: "Data folder", CACHE: "Cache folder", TRANSCODE_CACHE: "Transcode cache", IMAGE_CACHE: "Image cache", EMBED_CACHE: "Embedding cache", RECORDINGS: "Recordings folder" })[location.kind] ?? `${location.libraryName ?? "Library"} folder`;
  }
</script>

<div class="diag">
  <div class="diag-bar">
    <LiveBadge />
    {#if s}
      <span class="meta"><strong>{s.host.hostname ?? "Server"}</strong> · {s.host.osName ?? ""} · up <span class="tabular">{uptime(s.server.uptimeSeconds)}</span></span>
    {/if}
  </div>

  <div class="tiles">
    {#if !s}
      {#each Array(4) as _}<div class="tile skeleton"></div>{/each}
    {:else}
      <button class="tile" class:warn={s.cpu.usagePercent >= 85} onclick={() => navigate("resources")}>
        <span class="tile-label"><SquaresFour size={14} />CPU</span>
        <span class="tile-value"><Num value={s.cpu.usagePercent} format={(n) => percent(n)} /></span>
        <span class="tile-sub">{#if s.cpu.loadAverage}load <Num value={s.cpu.loadAverage.one} format={(n) => n.toFixed(2)} />{" · "}{/if}{s.cpu.logicalCores} threads</span>
        <span class="spark-slot"><Spark values={history.map((h) => h.cpuPercent)} max={100} label="CPU usage, last minutes" /></span>
      </button>
      <button class="tile" class:warn={memPct >= 90} onclick={() => navigate("resources")}>
        <span class="tile-label"><Stack size={14} />Memory</span>
        <span class="tile-value"><Num value={s.memory.usedBytes} format={(n) => size(n)} /><small>/ {size(s.memory.cgroupLimitBytes ?? s.memory.totalBytes)}</small></span>
        <span class="tile-sub"><Num value={memPct} format={(n) => percent(n)} /> used{#if s.memory.swapUsedBytes}{` · swap ${size(s.memory.swapUsedBytes)}`}{/if}</span>
        <span class="spark-slot"><Spark values={history.map((h) => h.memoryUsedBytes)} max={s.memory.totalBytes} tone="blue" label="Memory used, last minutes" /></span>
      </button>
      <button class="tile" onclick={() => navigate("resources")}>
        <span class="tile-label"><Broadcast size={14} />Network</span>
        <span class="tile-value"><span class="arrow">↓</span><Num value={s.networkTotal.rxBytesPerSec} format={rate} /></span>
        <span class="tile-sub">↑ <Num value={s.networkTotal.txBytesPerSec} format={rate} /> · all interfaces</span>
        <span class="spark-slot"><Spark values={history.map((h) => h.rxBytesPerSec)} alt={history.map((h) => h.txBytesPerSec)} tone="green" label="Network receive and transmit, last minutes" /></span>
      </button>
      <button class="tile" onclick={() => navigate("resources")}>
        <span class="tile-label"><HardDrives size={14} />NoFlex process</span>
        <span class="tile-value"><Num value={s.process.cpuPercent} format={(n) => percent(n)} /><small><Num value={s.process.rssBytes} format={(n) => size(n)} /></small></span>
        <span class="tile-sub">pid {s.process.pid} · {s.process.threads ?? "?"} threads · v{s.server.version}</span>
        <span class="spark-slot"><Spark values={history.map((h) => h.processCpuPercent)} tone="warn" label="Server process CPU, last minutes" /></span>
      </button>
    {/if}
  </div>

  <div class="tiles small">
    <button class="tile small" onclick={() => navigate("playback")}>
      <span class="tile-label"><Play size={13} weight="fill" />Streams</span>
      <span class="tile-value">{active.length}<small>{transcoding.length} transcoding</small></span>
    </button>
    <button class="tile small" onclick={() => navigate("indexing")}>
      <span class="tile-label"><ArrowsClockwise size={13} />Scans running</span>
      <span class="tile-value">{s?.activeScans ?? "—"}<small>{libraries.length} libraries</small></span>
    </button>
    <button class="tile small" class:bad={(s?.logs.errors ?? 0) > 0} class:warn={!s?.logs.errors && (s?.logs.warnings ?? 0) > 0} onclick={() => navigate("logs")}>
      <span class="tile-label"><WarningCircle size={13} />Errors</span>
      <span class="tile-value">{s?.logs.errors ?? "—"}<small>{s?.logs.warnings ?? 0} warn</small></span>
    </button>
    <button class="tile small" class:bad={(s?.plugins.circuitOpen ?? 0) > 0} onclick={() => navigate("plugins")}>
      <span class="tile-label"><Plug size={13} />Plugins</span>
      <span class="tile-value">{s ? s.plugins.loaded - s.plugins.circuitOpen : "—"}<small>of {s?.plugins.loaded ?? 0} healthy</small></span>
    </button>
    <button class="tile small" onclick={() => navigate("users")}>
      <span class="tile-label"><Users size={13} />Accounts</span>
      <span class="tile-value">{adminData?.users.length ?? "—"}<small>{adminData?.sessions.length ?? 0} sessions</small></span>
    </button>
    <button class="tile small" onclick={() => navigate("transcoding")}>
      <span class="tile-label"><FilmStrip size={13} />Transcoding</span>
      <span class="tile-value">{info ? BACKEND[info.resolvedBackend] ?? info.resolvedBackend : "—"}<small>{info ? (info.hardwareAccelerated ? "hardware" : "software") : ""}</small></span>
    </button>
  </div>

  <div class="diag-cols">
    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon">{#if issues.length}<WarningCircle size={18} />{:else}<CheckCircle size={18} />{/if}</span>
        <div><h2>{issues.length ? "Needs attention" : "All clear"}</h2><p>{issues.length ? "Problems the server can see right now, worst first." : "No errors, failing plugins, unreadable folders or full disks."}</p></div>
      </header>
      {#if !s}
        <div class="skeleton line"></div><div class="skeleton line"></div>
      {:else if issues.length}
        <ul class="issues">
          {#each issues.slice(0, 8) as issue (issue.title + issue.detail)}
            <li>
              <button onclick={() => navigate(issue.tab)}>
                <span class="glyph {issue.tone}"><issue.icon size={15} /></span>
                <span class="what"><strong>{issue.title}</strong>{#if issue.detail}<small>{issue.detail}</small>{/if}</span>
                <CaretRight size={16} />
              </button>
            </li>
          {/each}
        </ul>
        {#if issues.length > 8}<p class="empty-note">+ {issues.length - 8} more</p>{/if}
      {/if}
    </section>

    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><Play size={18} weight="fill" /></span>
        <div><h2>Now playing</h2><p>{active.length ? `${active.length} active · ${transcoding.length} transcoding · ${active.filter((x) => x.decision.hardware).length} on hardware` : "Nobody is streaming right now."}</p></div>
        <span class="spacer"></span>
        <button class="btn ghost sm" onclick={() => navigate("playback")}>Details<CaretRight size={14} /></button>
      </header>
      {#if active.length}
        <ul class="list now">
          {#each active.slice(0, 5) as stream (stream.id)}
            <li>
              <Avatar user={stream.user} />
              <span class="what"><strong>{stream.work?.title ?? (stream.channelId ? "Live TV" : "Unknown title")}</strong><small>{stream.user.name}{stream.device ? ` · ${stream.device}` : ""} · started {ago(stream.startedAt)}</small></span>
              {#if stream.decision.route === "DIRECT"}
                <span class="state ok">Direct</span>
              {:else}
                <span class="state" class:accent={stream.decision.hardware} class:info={!stream.decision.hardware}>{stream.decision.hardware ? "HW" : "SW"} transcode{#if stream.transcode?.speed != null}{` · ${stream.transcode.speed.toFixed(1)}×`}{/if}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>

  {#if s}
    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><HardDrives size={18} /></span>
        <div><h2>Filesystems</h2><p>Free space where NoFlex keeps its data and reads your media.</p></div>
      </header>
      <div class="disks">
        {#each filesystems as fs (fs.key)}
          <div class="disk">
            <div class="disk-head"><strong class="mono">{fs.key}</strong><span class="tabular">{size(fs.available)} free of {size(fs.total)}</span></div>
            <Meter pct={ratio(fs.used, fs.total)} label="{fs.key} used" thin />
            <small>{fs.fileSystem ?? ""}{fs.fileSystem ? " · " : ""}{fs.places.join(", ")}</small>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .arrow { color: var(--text-3); margin-right: 3px; font-weight: 500; }
  .cap { text-transform: capitalize; }
  .skeleton.line { height: 18px; margin-top: 12px; border-radius: 6px; }
  .issues { display: grid; gap: 2px; }
  .issues button { display: flex; align-items: center; gap: 12px; width: 100%; padding: 9px 8px; border-radius: var(--r-s); text-align: left; color: var(--text-3); transition: background var(--t-fast); }
  .issues button:hover { background: var(--surface-2); color: var(--text-2); }
  .glyph { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 9px; flex: none; }
  .glyph.bad { background: var(--danger-soft); color: var(--danger); }
  .glyph.warn { background: rgb(255 193 100 / 0.12); color: var(--warning); }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what strong { color: var(--text); font-size: 0.9rem; font-weight: 600; }
  .what small { color: var(--text-3); font-size: 0.8rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .now li { padding-block: 10px; }
  .disks { display: grid; gap: 14px 22px; grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr)); }
  .disk { display: grid; gap: 6px; min-width: 0; }
  .disk-head { display: flex; justify-content: space-between; gap: 8px; font-size: 0.86rem; }
  .disk-head span { color: var(--text-3); }
  .disk small { color: var(--text-3); font-size: 0.74rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
