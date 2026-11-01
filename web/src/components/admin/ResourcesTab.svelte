<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import HardDrives from "phosphor-svelte/lib/HardDrives";
  import SquaresFour from "phosphor-svelte/lib/SquaresFour";
  import Stack from "phosphor-svelte/lib/Stack";
  import { ago } from "$components/account/relative.js";
  import LiveBadge from "./status/LiveBadge.svelte";
  import Meter from "./status/Meter.svelte";
  import Num from "./status/Num.svelte";
  import Spark from "./status/Spark.svelte";
  import { percent, rate, ratio, size, stamp, uptime } from "./status/fmt.js";
  import { liveStatus, useLiveStatus } from "./status/live.svelte.js";

  $effect(() => useLiveStatus());

  let s = $derived(liveStatus.status);
  let history = $derived(liveStatus.history);
  let showAllNics = $state(false);
  let nics = $derived((s?.network ?? []).filter((nic) => showAllNics || (!nic.loopback && (nic.state === "up" || nic.rxTotalBytes > 0 || nic.txTotalBytes > 0))));
  let hiddenNics = $derived((s?.network.length ?? 0) - nics.length);

  const KIND = { DATA: "Data", CACHE: "Cache", TRANSCODE_CACHE: "Transcode cache", IMAGE_CACHE: "Image cache", EMBED_CACHE: "Embedding cache", RECORDINGS: "Recordings", LIBRARY_ROOT: "Library" };
  const memLimit = (m) => m.cgroupLimitBytes ?? m.totalBytes;
</script>

<div class="diag">
  <div class="diag-bar">
    <LiveBadge />
    {#if s}<span class="meta">{s.host.hostname} · {s.host.osVersion ?? s.host.osName} · kernel {s.host.kernelVersion ?? "?"} · {s.host.arch} · host up {uptime(s.host.uptimeSeconds)}</span>{/if}
  </div>

  {#if !s}
    <div class="diag-cols">{#each Array(4) as _}<div class="panel skeleton tall"></div>{/each}</div>
  {:else}
    <div class="diag-cols">
      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><SquaresFour size={18} /></span>
          <div class="grow"><h2>CPU</h2><p class="ellipsis">{s.cpu.brand || "Processor"} · {s.cpu.physicalCores ?? "?"} cores / {s.cpu.logicalCores} threads</p></div>
          <span class="big tabular"><Num value={s.cpu.usagePercent} format={(n) => percent(n)} /></span>
        </header>
        <div class="spark-lg"><Spark values={history.map((h) => h.cpuPercent)} max={100} label="CPU usage" /></div>
        {#if s.cpu.loadAverage}
          <dl class="kv load">
            <div><dt>Load 1 min</dt><dd><Num value={s.cpu.loadAverage.one} format={(n) => n.toFixed(2)} /></dd></div>
            <div><dt>5 min</dt><dd><Num value={s.cpu.loadAverage.five} format={(n) => n.toFixed(2)} /></dd></div>
            <div><dt>15 min</dt><dd><Num value={s.cpu.loadAverage.fifteen} format={(n) => n.toFixed(2)} /></dd></div>
          </dl>
        {/if}
        <div class="cores" style:--cols={Math.min(16, Math.max(1, s.cpu.cores.length))}>
          {#each s.cpu.cores as core (core.index)}
            <div class="core" title="Thread {core.index}: {percent(core.usagePercent)} at {core.frequencyMhz} MHz">
              <span class="bar"><span style:height="{Math.max(2, core.usagePercent)}%" class:hot={core.usagePercent >= 85}></span></span>
              <small class="tabular">{core.index}</small>
            </div>
          {/each}
        </div>
      </section>

      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><Stack size={18} /></span>
          <div class="grow"><h2>Memory</h2><p>{s.memory.cgroupLimitBytes ? `Container limit ${size(s.memory.cgroupLimitBytes)} (host ${size(s.memory.totalBytes)})` : `${size(s.memory.totalBytes)} installed`}</p></div>
          <span class="big tabular"><Num value={ratio(s.memory.usedBytes, memLimit(s.memory))} format={(n) => percent(n)} /></span>
        </header>
        <div class="spark-lg"><Spark values={history.map((h) => h.memoryUsedBytes)} max={memLimit(s.memory)} tone="blue" label="Memory used" /></div>
        <div class="bars">
          <div class="bar-row"><span>RAM</span><Meter pct={ratio(s.memory.usedBytes, memLimit(s.memory))} warn={85} bad={95} label="RAM used" /><span class="tabular"><Num value={s.memory.usedBytes} format={(n) => size(n)} /> / {size(memLimit(s.memory))}</span></div>
          <div class="bar-row"><span>Swap</span><Meter pct={ratio(s.memory.swapUsedBytes, s.memory.swapTotalBytes)} warn={50} bad={80} label="Swap used" /><span class="tabular">{s.memory.swapTotalBytes ? `${size(s.memory.swapUsedBytes)} / ${size(s.memory.swapTotalBytes)}` : "none"}</span></div>
        </div>
        <dl class="kv">
          <div><dt>Available</dt><dd><Num value={s.memory.availableBytes} format={(n) => size(n)} /></dd></div>
          <div><dt>NoFlex resident</dt><dd><Num value={s.process.rssBytes} format={(n) => size(n)} /></dd></div>
        </dl>
      </section>

      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><HardDrives size={18} /></span>
          <div class="grow"><h2>NoFlex process</h2><p>pid {s.process.pid} · up {uptime(s.server.uptimeSeconds)} · started {ago(s.server.startedAt)}</p></div>
          <span class="big tabular"><Num value={s.process.cpuPercent} format={(n) => percent(n)} /></span>
        </header>
        <div class="spark-lg"><Spark values={history.map((h) => h.processCpuPercent)} tone="warn" label="Server process CPU" /></div>
        <dl class="kv">
          <div><dt>CPU of host</dt><dd><Num value={s.process.cpuPercentOfHost} format={(n) => percent(n, 1)} /></dd></div>
          <div><dt>Resident</dt><dd><Num value={s.process.rssBytes} format={(n) => size(n)} /></dd></div>
          <div><dt>Virtual</dt><dd>{size(s.process.virtualBytes)}</dd></div>
          <div><dt>Threads</dt><dd>{s.process.threads ?? "—"}</dd></div>
          <div><dt>Open files</dt><dd>{s.process.openFiles ?? "—"}{s.process.openFilesLimit ? ` / ${s.process.openFilesLimit}` : ""}</dd></div>
          <div><dt>Started</dt><dd>{stamp(s.server.startedAt)}</dd></div>
        </dl>
      </section>

      <section class="panel dense">
        <header class="panel-head">
          <span class="panel-icon"><Broadcast size={18} /></span>
          <div class="grow"><h2>Network</h2><p>Non-loopback interfaces combined.</p></div>
          <span class="big tabular small-big">↓ <Num value={s.networkTotal.rxBytesPerSec} format={rate} /><br />↑ <Num value={s.networkTotal.txBytesPerSec} format={rate} /></span>
        </header>
        <div class="spark-lg"><Spark values={history.map((h) => h.rxBytesPerSec)} alt={history.map((h) => h.txBytesPerSec)} tone="green" label="Network receive (solid) and transmit (dashed)" /></div>
        <p class="legend"><span class="key rx"></span>receive <span class="key tx"></span>transmit</p>
      </section>
    </div>

    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><Broadcast size={18} /></span>
        <div class="grow"><h2>Interfaces</h2><p>Rates over the last sample interval; totals since the interface came up.</p></div>
        <label class="check"><input type="checkbox" bind:checked={showAllNics} />Show all{hiddenNics && !showAllNics ? ` (+${hiddenNics})` : ""}</label>
      </header>
      <div class="dtable-wrap">
        <table class="dtable">
          <thead><tr><th scope="col">Interface</th><th scope="col">State</th><th scope="col">Addresses</th><th scope="col" class="num">↓ Receive</th><th scope="col" class="num">↑ Transmit</th><th scope="col" class="num">Total ↓ / ↑</th><th scope="col" class="num">Errors</th></tr></thead>
          <tbody>
            {#each nics as nic (nic.name)}
              <tr class:dim={nic.state === "down"}>
                <td class="strong mono">{nic.name}{#if nic.loopback} <span class="faint">(loopback)</span>{/if}</td>
                <td><span class="state" class:ok={nic.state === "up"}>{nic.state}</span></td>
                <td class="mono addr">{nic.addresses.slice(0, 2).join(", ") || "—"}{nic.addresses.length > 2 ? ` +${nic.addresses.length - 2}` : ""}</td>
                <td class="num"><Num value={nic.rxBytesPerSec} format={rate} /></td>
                <td class="num"><Num value={nic.txBytesPerSec} format={rate} /></td>
                <td class="num">{size(nic.rxTotalBytes)} / {size(nic.txTotalBytes)}</td>
                <td class="num" class:warn-text={nic.rxErrorsTotal + nic.txErrorsTotal > 0}>{nic.rxErrorsTotal + nic.txErrorsTotal}</td>
              </tr>
            {:else}
              <tr><td colspan="7" class="faint">No active interfaces.</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>

    <section class="panel dense">
      <header class="panel-head">
        <span class="panel-icon"><HardDrives size={18} /></span>
        <div class="grow"><h2>Disks & folders</h2><p>Each folder NoFlex depends on and the filesystem that holds it. Cache sizes refresh every minute.</p></div>
      </header>
      <div class="dtable-wrap">
        <table class="dtable">
          <thead><tr><th scope="col">Folder</th><th scope="col">Path</th><th scope="col">Filesystem</th><th scope="col" class="usage-col">Used</th><th scope="col" class="num">Free</th><th scope="col" class="num">Contents</th></tr></thead>
          <tbody>
            {#each s.storage as location (location.kind + location.path)}
              {@const used = ratio(location.usedBytes, location.totalBytes)}
              <tr>
                <td class="strong nowrap">{KIND[location.kind] ?? location.kind}{#if location.libraryName}<span class="faint">{` · ${location.libraryName}`}</span>{/if}</td>
                <td class="mono wrap">{location.path}</td>
                <td class="nowrap">{#if location.error}<span class="state" class:bad={location.exists || location.kind === "LIBRARY_ROOT"} title={location.error}>{location.exists ? "Error" : location.kind === "LIBRARY_ROOT" ? "Missing" : "Not created"}</span>{:else}<span class="mono">{location.fileSystem ?? "?"}</span> <span class="faint mono">{location.mountPoint ?? ""}</span>{/if}</td>
                <td class="usage-col">{#if location.totalBytes}<div class="usage"><Meter pct={used} thin label="{location.path} used" /><span class="tabular">{percent(used)}</span></div>{:else}<span class="faint">{location.exists ? location.error ?? "—" : "—"}</span>{/if}</td>
                <td class="num">{location.totalBytes ? `${size(location.availableBytes)} of ${size(location.totalBytes)}` : "—"}</td>
                <td class="num">{location.contentBytes != null ? size(location.contentBytes) : "—"}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  {/if}
</div>

<style>
  .tall { height: 240px; }
  .grow { flex: 1; min-width: 0; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .big { font-size: 1.5rem; font-weight: 700; letter-spacing: -0.02em; white-space: nowrap; min-width: 4.5ch; text-align: right; }
  .small-big { font-size: 0.95rem; line-height: 1.35; min-width: 11ch; }
  .spark-lg { height: 54px; margin: -4px 0 14px; }
  .spark-lg :global(.spark) { height: 54px; }
  .load { margin-bottom: 14px; grid-template-columns: repeat(3, minmax(0, 1fr)); }
  /* Even rows: up to 16 bars per row, so 32 threads read as two clean rows. */
  .cores { display: grid; grid-template-columns: repeat(var(--cols, 16), minmax(0, 1fr)); gap: 4px 3px; }
  .core { display: grid; gap: 3px; justify-items: center; }
  .core .bar { position: relative; width: 100%; height: 44px; border-radius: 4px; background: rgb(255 255 255 / 0.06); overflow: hidden; }
  .core .bar span { position: absolute; inset: auto 0 0; background: var(--accent); border-radius: 3px; transition: height 600ms var(--ease-out); }
  .core .bar span.hot { background: var(--warning); }
  .core small { font-size: 0.6rem; color: var(--text-3); }
  .bars { display: grid; gap: 12px; margin-bottom: 16px; }
  .bar-row { display: grid; grid-template-columns: 44px 1fr auto; align-items: center; gap: 12px; font-size: 0.84rem; color: var(--text-2); }
  .bar-row > span:last-child { min-width: 15ch; text-align: right; color: var(--text-3); }
  .legend { display: flex; align-items: center; gap: 8px; font-size: 0.78rem; color: var(--text-3); }
  .key { width: 16px; height: 0; border-top: 2px solid var(--success); }
  .key.tx { border-top: 2px dashed var(--focus); margin-left: 8px; }
  .addr { max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .usage-col { min-width: 140px; }
  .usage { display: grid; grid-template-columns: 1fr 4ch; align-items: center; gap: 8px; }
  .usage span { text-align: right; font-size: 0.8rem; }
  .warn-text { color: var(--warning) !important; }
  @media (max-width: 560px) { .bar-row { grid-template-columns: 40px 1fr; } .bar-row > span:last-child { grid-column: 2; text-align: left; } }
</style>
