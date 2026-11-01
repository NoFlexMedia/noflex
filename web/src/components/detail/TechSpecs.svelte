<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { Tabs as BitsTabs } from "bits-ui";
  import { bytes, languageName, timecode } from "$lib/format.js";
  import Tabs from "$components/ui/Tabs.svelte";

  let { edition, file = null, onchapter } = $props();

  // The file's list holds embedded and sidecar tracks; the edition's only
  // what was probed inside its first file.
  const SUB_CODECS = { sub_rip: "SRT", subrip: "SRT", srt: "SRT", webvtt: "WebVTT", vtt: "WebVTT", ass: "ASS", ssa: "SSA", mov_text: "TX3G", hdmv_pgs_subtitle: "PGS", pgs_image: "PGS", dvd_subtitle: "VobSub", dvb_subtitle: "DVB" };
  const subCodec = (codec) => SUB_CODECS[codec?.toLowerCase()] ?? codec?.toUpperCase();
  let subtitles = $derived(file?.subtitleTracks?.length ? file.subtitleTracks : edition.subtitleTracks ?? []);
  let cues = $derived(edition.trickplay?.chapterCues ?? []);
  let chapters = $derived(cues.length ? cues : edition.chapters ?? []);
  let items = $derived([
    ...(chapters.length ? [{ value: "chapters", label: "Chapters", count: chapters.length }] : []),
    { value: "tracks", label: "Audio & subtitles" },
    { value: "file", label: "Technical" },
  ]);
  let tab = $state("");
  $effect(() => { if (!items.some((item) => item.value === tab)) tab = items[0].value; });

  const THUMB_W = 224;
  let video = $derived([
    ["Video", [edition.videoCodec ?? edition.codecVideo, edition.videoProfile, edition.videoLevel ? `L${edition.videoLevel}` : null].filter(Boolean).join(" · ")],
    ["Resolution", edition.width && edition.height ? `${edition.width} × ${edition.height}${edition.frameRate ? ` @ ${edition.frameRate.toFixed(3).replace(/\.?0+$/, "")} fps` : ""}` : null],
    ["Dynamic range", edition.hdrFormat ?? (edition.transferCharacteristics && edition.transferCharacteristics !== "SDR" ? edition.transferCharacteristics : "SDR")],
    ["Bit depth", edition.bitDepth ? `${edition.bitDepth}-bit${edition.pixelFormat ? ` · ${edition.pixelFormat}` : ""}` : null],
    ["Audio", edition.codecAudio],
    ["Container", file?.container?.toUpperCase()],
    ["File size", file ? bytes(file.sizeBytes) : null],
    ["Path", file?.path],
  ].filter(([, value]) => value));
</script>

<section class="section specs">
  <Tabs bind:value={tab} {items} label="Details">
    <BitsTabs.Content value="chapters" class="panel-body">
      <ol class="chapters stagger">
        {#each chapters as chapter, index (index)}
          <li style:--i={index}>
            <button onclick={() => onchapter(chapter.positionMs)}>
              {#if chapter.sheetUrl}
                <span class="frame" style:--k={THUMB_W / chapter.width} style:aspect-ratio="{chapter.width} / {chapter.height}">
                  <span class="sprite" style:width="{chapter.width}px" style:height="{chapter.height}px" style:background-image="url({chapter.sheetUrl})" style:background-position="-{chapter.x}px -{chapter.y}px"></span>
                </span>
              {/if}
              <span class="label"><strong>{chapter.title || `Chapter ${index + 1}`}</strong><small class="tabular">{timecode(chapter.positionMs)}</small></span>
            </button>
          </li>
        {/each}
      </ol>
    </BitsTabs.Content>
    <BitsTabs.Content value="tracks" class="panel-body">
      <div class="cols">
        <div>
          <h3>Audio</h3>
          {#each edition.audioTracks ?? [] as track}
            <p class="track"><strong>{track.title || languageName(track.language) || "Unknown"}</strong><span>{[track.codec?.toUpperCase(), track.channels ? `${track.channels} ch` : null, track.default ? "Default" : null].filter(Boolean).join(" · ")}</span></p>
          {:else}<p class="empty-note">No audio tracks reported.</p>{/each}
        </div>
        <div>
          <h3>Subtitles</h3>
          {#each subtitles as track}
            <p class="track"><strong>{track.title || languageName(track.language) || "Unknown"}</strong><span>{[subCodec(track.codec), track.forced ? "Forced" : null, track.external ? "Sidecar" : "Embedded"].filter(Boolean).join(" · ")}</span></p>
          {:else}<p class="empty-note">No subtitles.</p>{/each}
        </div>
      </div>
    </BitsTabs.Content>
    <BitsTabs.Content value="file" class="panel-body">
      <dl class="facts">
        {#each video as [term, value]}<div><dt>{term}</dt><dd class:mono={term === "Path"}>{value}</dd></div>{/each}
      </dl>
    </BitsTabs.Content>
  </Tabs>
</section>

<style>
  .specs { margin-top: 56px; }
  .specs :global(.panel-body) { margin-top: 20px; animation: fade-up 360ms var(--ease-out); }
  .chapters { display: grid; grid-auto-flow: column; grid-auto-columns: 224px; gap: 14px; overflow-x: auto; padding-bottom: 12px; scroll-snap-type: x mandatory; }
  .chapters li { scroll-snap-align: start; }
  .chapters button { display: grid; gap: 9px; width: 100%; text-align: left; }
  .frame { position: relative; display: block; width: 224px; overflow: hidden; border-radius: 10px; background: var(--surface-2); outline: 1px solid var(--line); outline-offset: -1px; }
  .sprite { position: absolute; left: 0; top: 0; transform: scale(var(--k)); transform-origin: 0 0; background-repeat: no-repeat; transition: filter var(--t); }
  .chapters button:hover .sprite { filter: brightness(1.15); }
  .chapters button:hover .frame { outline-color: var(--line-strong); }
  .label { display: grid; }
  .label strong { font-size: 0.88rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .label small { color: var(--text-3); font-size: 0.8rem; }
  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 32px; max-width: 900px; }
  h3 { font-size: 0.78rem; letter-spacing: 0.12em; text-transform: uppercase; color: var(--text-3); margin-bottom: 10px; }
  .track { display: flex; justify-content: space-between; gap: 16px; padding: 9px 0; border-top: 1px solid var(--line); font-size: 0.9rem; }
  .track span { color: var(--text-3); text-align: right; }
  .facts { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 18px 32px; max-width: 980px; }
  dt { font-size: 0.76rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-3); margin-bottom: 4px; }
  dd { font-size: 0.94rem; font-weight: 550; word-break: break-word; }
</style>
