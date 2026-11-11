<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import Cpu from "phosphor-svelte/lib/Cpu";
  import Info from "phosphor-svelte/lib/Info";
  import { prefersReducedMotion, reveal } from "$lib/actions";

  // Reasons are the server's own wording (noflex-transcoder/src/reasons.rs).
  const cases = [
    { device: "Pixel 9", source: ["Sintel", "AV1 10-bit", "Opus 5.1", "MKV"], plan: "Direct play", reason: null, detail: "Decodes AV1 in hardware, so the file plays as stored." },
    { device: "Chrome on a laptop", source: ["Big Buck Bunny", "H.264", "AAC", "MKV"], plan: "Remux", reason: "container not supported for direct play", detail: "Both streams copied into HLS. Nothing is re-encoded." },
    { device: "Living room TV", source: ["Tears of Steel", "HEVC HDR10", "TrueHD 7.1", "MKV"], plan: "Transcode", reason: "HDR / Dolby Vision range not supported", detail: "Tone-mapped to SDR on the GPU, encoded to AV1 with NVENC." },
  ];
  const ladder = [
    { codec: "AV1", note: "Tried first, for any device that decodes it" },
    { codec: "HEVC", note: "Next, where AV1 won't play" },
    { codec: "H.264", note: "Plays everywhere else" },
  ];
  const backends = ["VAAPI", "NVENC", "Quick Sync", "AMF", "VideoToolbox", "Media Foundation"];

  let index = $state(2);
  let current = $derived(cases[index]);
  let timer = 0;
  function start() {
    if (prefersReducedMotion()) return;
    index = 0;
    timer = setInterval(() => (index = (index + 1) % cases.length), 3800);
  }
  $effect(() => () => clearInterval(timer));
</script>

<div class="grid" use:reveal={start}>
  <div class="panel decision">
    <div class="top">
      <span class="eyebrow">Playback info</span>
      <div class="tabs" role="tablist" aria-label="Example devices">
        <span class="indicator" style:width="calc((100% - 8px) / 3)" style:transform="translateX(calc({index} * 100%))"></span>
        {#each cases as item, i}
          <button role="tab" aria-selected={i === index} onclick={() => { clearInterval(timer); index = i; }}>{item.plan}</button>
        {/each}
      </div>
    </div>
    {#key index}
      <div class="body">
        <div class="flow">
          <div class="side">
            <small>Source</small>
            <strong>{current.source[0]}</strong>
            <span class="chips">{#each current.source.slice(1) as part}<span class="badge">{part}</span>{/each}</span>
          </div>
          <ArrowRight size={18} />
          <div class="side">
            <small>Playing on</small>
            <strong>{current.device}</strong>
            <span class="plan" class:direct={!current.reason}>{current.plan}</span>
          </div>
        </div>
        <p class="why"><Info size={16} /><span>{#if current.reason}<em>Why: {current.reason}.</em> {/if}{current.detail}</span></p>
      </div>
    {/key}
  </div>

  <div class="panel ladder">
    <span class="eyebrow">Encoder ladder</span>
    <ol>
      {#each ladder as rung, i}
        <li style:--i={i}><strong>{rung.codec}</strong><small>{rung.note}</small></li>
      {/each}
    </ol>
    <div class="hw">
      <span><Cpu size={15} />Hardware encoding on</span>
      <ul>{#each backends as name}<li class="chip">{name}</li>{/each}</ul>
    </div>
  </div>
</div>

<style>
  .grid { display: grid; grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr); gap: 16px; }
  .top { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .tabs { width: 300px; max-width: 100%; }
  .tabs button { flex: 1; justify-content: center; padding: 0 8px; height: 30px; font-size: 0.8rem; }
  .tabs .indicator { left: 4px; }
  .body { display: grid; gap: 18px; margin-top: 22px; animation: fade-up 480ms var(--ease-out) both; }
  .flow { display: grid; grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr); align-items: center; gap: 14px; color: var(--text-3); }
  .side { display: grid; gap: 6px; justify-items: start; padding: 14px; border-radius: var(--r); background: rgb(0 0 0 / 0.22); border: 1px solid var(--line); min-width: 0; }
  .side small { font-size: 0.72rem; color: var(--text-3); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; }
  .side strong { color: var(--text); font-size: 0.98rem; }
  .chips { display: flex; flex-wrap: wrap; gap: 5px; }
  .plan { display: inline-flex; align-items: center; height: 22px; padding: 0 9px; border-radius: 99px; background: rgb(255 193 100 / 0.14); color: var(--warning); font-size: 0.74rem; font-weight: 650; }
  .plan.direct { background: var(--accent-soft); color: var(--accent); }
  .why { display: flex; gap: 10px; color: var(--text-2); font-size: 0.9rem; }
  .why :global(svg) { flex: none; margin-top: 3px; color: var(--text-3); }
  .why em { font-style: normal; color: var(--text); font-weight: 550; }
  .ladder { display: grid; align-content: start; gap: 16px; }
  ol { display: grid; gap: 8px; counter-reset: rung; }
  ol li { display: grid; grid-template-columns: 64px minmax(0, 1fr); align-items: center; gap: 12px; padding: 10px 12px; border-radius: var(--r); border: 1px solid var(--line); background: var(--surface); }
  ol li:first-child { border-color: rgb(212 255 90 / 0.4); background: var(--accent-soft); }
  ol li:first-child strong { color: var(--accent); }
  ol strong { font-size: 1rem; letter-spacing: -0.02em; }
  ol small { color: var(--text-3); font-size: 0.8rem; }
  .hw { display: grid; gap: 10px; }
  .hw > span { display: inline-flex; align-items: center; gap: 7px; font-size: 0.8rem; color: var(--text-3); font-weight: 550; }
  .hw ul { display: flex; flex-wrap: wrap; gap: 6px; }
  @media (max-width: 899px) { .grid { grid-template-columns: 1fr; } }
  @media (max-width: 520px) { .flow { grid-template-columns: 1fr; } .flow > :global(svg) { transform: rotate(90deg); justify-self: center; } }
</style>
