<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Play from "phosphor-svelte/lib/Play";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { AdminActiveStreamsStore } from "$houdini";
  import { titleCase } from "$lib/format.js";
  import { failure, refetch } from "$lib/graphql.js";
  import { poll } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import Tip from "$components/ui/Tip.svelte";
  import { ago } from "$components/account/relative.js";
  import Meter from "./status/Meter.svelte";
  import Num from "./status/Num.svelte";
  import { bitrate, channels, codec, range, ratio, resolution } from "./status/fmt.js";
  import { timecode } from "$lib/format.js";

  const streams = new AdminActiveStreamsStore();
  $effect(() => {
    refetch(streams);
    return poll(() => refetch(streams), 3000);
  });

  let list = $derived($streams.data?.activeStreams ?? []);
  let loading = $derived($streams.fetching && !$streams.data);
  let counts = $derived({
    direct: list.filter((stream) => stream.decision.route === "DIRECT").length,
    transcode: list.filter((stream) => stream.decision.route === "TRANSCODE").length,
    hardware: list.filter((stream) => stream.decision.hardware).length,
  });

  const PLAN = { DIRECT_PLAY: "Direct play", REMUX: "Remux", TRANSCODE_AUDIO: "Audio transcode", TRANSCODE_VIDEO: "Video transcode", TRANSCODE: "Full transcode" };
  const planTone = (plan) => (plan === "DIRECT_PLAY" ? "ok" : plan === "REMUX" ? "info" : "accent");

  function video(summary) {
    if (!summary.videoCodec) return "—";
    // Profiles are codec-prefixed (`H264_HIGH`, `VP9_PROFILE2`); the codec is already named.
    const profile = summary.videoProfile ? ` ${titleCase(summary.videoProfile.replace(/^(H264|HEVC|AV1|VP9)_/, "")).replace(/^Profile(\d)/, "Profile $1")}` : "";
    const level = summary.videoLevel ? ` @${summary.videoLevel}` : "";
    const depth = summary.bitDepth ? ` · ${summary.bitDepth}-bit` : "";
    return `${codec(summary.videoCodec)}${profile}${level}${depth}`;
  }
  const audio = (summary) => (summary.audioCodec ? `${summary.audioCodec.toUpperCase()}${summary.audioChannels ? ` ${channels(summary.audioChannels)}` : ""}` : "—");
  const changed = (a, b) => a !== b && b !== "—";

  function rows(decision) {
    const { source, target } = decision;
    return [
      ["Container", source.container ?? "—", target.container ?? "—"],
      ["Video", video(source), decision.encoder === "copy" ? `${video(target)} (copy)` : video(target)],
      ["Resolution", resolution(source.width, source.height), resolution(target.width, target.height)],
      ["Range", range(source.videoRange), decision.tonemap ? `${range(target.videoRange)} (tonemapped)` : range(target.videoRange)],
      ["Audio", audio(source), decision.audioEncoder === "copy" ? `${audio(target)} (copy)` : audio(target)],
      ["Bitrate", bitrate(source.bitrateBps), bitrate(target.bitrateBps)],
    ];
  }

  function progress(stream) {
    const runtime = stream.work?.runtimeMs;
    return runtime && stream.positionMs != null ? ratio(stream.positionMs, runtime) : null;
  }
  const idle = (stream) => Date.now() - Date.parse(stream.lastActiveAt) > 60_000;
</script>

<div class="diag">
  <div class="diag-bar">
    <span class="state">{list.length} active</span>
    <span class="state ok">{counts.direct} direct</span>
    <span class="state accent">{counts.transcode} transcoding</span>
    <span class="state info">{counts.hardware} on hardware</span>
    <span class="spacer"></span>
    <span class="meta">Refreshes every 3 s</span>
  </div>

  {#if loading}
    <div class="streams">{#each Array(2) as _}<div class="panel skeleton stream-skel"></div>{/each}</div>
  {:else if failure($streams) && !$streams.data}
    <State kind="error" title="Couldn’t read active streams" message={failure($streams)} action={() => refetch(streams)} />
  {:else if !list.length}
    <State icon={Play} title="Nothing is playing" message="Streams appear here the moment someone presses play — with the direct-play decision, the reasons behind a transcode, and ffmpeg’s live progress." />
  {:else}
    <div class="streams stagger">
      {#each list as stream, index (stream.id)}
        {@const d = stream.decision}
        {@const t = stream.transcode}
        {@const pct = progress(stream)}
        <article class="panel dense stream" style:--i={index}>
          <header class="head">
            <span class="avatar">{stream.user.name.slice(0, 2).toUpperCase()}</span>
            <div class="who">
              <h3>{#if stream.work}<a href="/works/{stream.work.parentId ?? stream.work.id}">{stream.work.title}</a>{:else}{stream.channelId ? "Live TV channel" : "Unknown title"}{/if}{#if stream.work?.year}<span class="faint year">({stream.work.year})</span>{/if}</h3>
              <p>{stream.user.name}{stream.device ? ` · ${stream.device}` : ""} · started {ago(stream.startedAt)}{#if idle(stream)} · <span class="warn-text">idle {ago(stream.lastActiveAt)}</span>{/if}</p>
            </div>
            <span class="spacer"></span>
            <span class="state {planTone(d.plan)}">{PLAN[d.plan] ?? titleCase(d.plan)}</span>
            {#if d.route === "TRANSCODE"}<span class="state" class:accent={d.hardware}>{d.hardware ? "Hardware" : "Software"}</span>{/if}
            {#if d.tonemap}<span class="state warn">Tonemap</span>{/if}
          </header>

          <div class="position">
            <span class="tabular">{stream.positionMs != null ? timecode(stream.positionMs) : "—"}</span>
            <Meter pct={pct ?? 0} thin label="Playback position" />
            <span class="tabular faint">{stream.work?.runtimeMs ? timecode(stream.work.runtimeMs) : ""}</span>
          </div>

          {#if d.reasons.length}
            <div class="reasons">
              {#each d.reasons as reason (reason.code)}
                <Tip text={reason.code}>{#snippet children(props)}<span {...props} class="chip">{reason.description}</span>{/snippet}</Tip>
              {/each}
            </div>
          {/if}

          <div class="dtable-wrap">
            <table class="dtable">
              <thead><tr><th scope="col"></th><th scope="col">Source</th><th scope="col" class="arrow"><span class="sr-only">to</span></th><th scope="col">Delivered</th></tr></thead>
              <tbody>
                {#each rows(d) as [label, from, to] (label)}
                  <tr>
                    <th scope="row" class="row-label">{label}</th>
                    <td class="nowrap">{from}</td>
                    <td class="arrow" aria-hidden="true">{changed(from, to) ? "→" : ""}</td>
                    <td class="nowrap" class:strong={changed(from, to)}>{to}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>

          {#if d.route === "TRANSCODE"}
            <dl class="kv ffmpeg">
              <div><dt>Video encoder</dt><dd class="mono">{d.encoder ?? "—"}</dd></div>
              <div><dt>Audio encoder</dt><dd class="mono">{d.audioEncoder ?? "—"}</dd></div>
              {#if t}
                <div><dt>Job</dt><dd><span class="state" class:ok={t.status === "running"} class:bad={t.status === "failed"}>{t.status}</span></dd></div>
                <div><dt>Speed</dt><dd class:warn-text={t.speed != null && t.speed < 1}>{#if t.speed != null}<Num value={t.speed} format={(n) => `${n.toFixed(2)}×`} />{:else}—{/if}</dd></div>
                <div><dt>Frame rate</dt><dd>{#if t.fps != null}<Num value={t.fps} format={(n) => `${n.toFixed(0)} fps`} />{:else}—{/if}</dd></div>
                <div><dt>Encoded to</dt><dd>{t.outTimeMs != null ? timecode(t.outTimeMs) : "—"}</dd></div>
                <div class="full"><dt>Job id</dt><dd class="mono">{t.jobId}</dd></div>
              {/if}
            </dl>
            {#if t?.error}<p class="error"><WarningCircle size={14} />{t.error}</p>{/if}
          {/if}
        </article>
      {/each}
    </div>
  {/if}
</div>

<style>
  .streams { display: grid; gap: 16px; grid-template-columns: repeat(auto-fill, minmax(min(100%, 520px), 1fr)); align-items: start; }
  .year { margin-left: 0.35em; font-weight: 500; }
  .stream-skel { height: 300px; }
  .stream { display: grid; gap: 14px; align-content: start; }
  .head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .who { min-width: 0; flex: 1 1 200px; }
  .who h3 { font-size: 1rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who h3 a:hover { text-decoration: underline; text-underline-offset: 3px; }
  .who p { color: var(--text-3); font-size: 0.82rem; margin-top: 2px; }
  .head .spacer { flex: 0 0 0; }
  .position { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 10px; font-size: 0.8rem; color: var(--text-2); }
  .reasons { display: flex; flex-wrap: wrap; gap: 6px; }
  .reasons .chip { height: auto; min-height: 26px; padding-block: 4px; white-space: normal; }
  .row-label { font-size: 0.7rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--text-3); font-weight: 600; padding-left: 0 !important; width: 1%; white-space: nowrap; border-top: 1px solid var(--line); }
  .arrow { color: var(--accent); width: 1%; padding-inline: 2px !important; }
  .ffmpeg { padding-top: 12px; border-top: 1px solid var(--line); grid-template-columns: repeat(auto-fill, minmax(min(100%, 120px), 1fr)); }
  .warn-text { color: var(--warning); }
  .error { display: flex; align-items: center; gap: 8px; color: var(--danger); font-size: 0.86rem; }
  @media (max-width: 560px) {
    .stream .dtable td { white-space: normal; overflow-wrap: anywhere; padding-inline: 5px; }
    .stream .dtable .arrow { display: none; }
    .stream .dtable .row-label { width: auto; }
    .stream .dtable td.strong { color: var(--accent); }
  }
</style>
