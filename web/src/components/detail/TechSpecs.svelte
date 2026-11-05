<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { Tabs as BitsTabs } from "bits-ui";
  import { audioFormat, bytes, codecName, dynamicRange, levelName, profileName, timecode } from "$lib/format.js";
  import Tabs from "$components/ui/Tabs.svelte";

  /** `file` is the chosen version and `audio` the chosen track. */
  let { edition, file = null, audio = null, onchapter } = $props();

  let cues = $derived(edition.trickplay?.chapterCues ?? []);
  let chapters = $derived(cues.length ? cues : edition.chapters ?? []);
  let items = $derived([
    ...(chapters.length ? [{ value: "chapters", label: "Chapters", count: chapters.length }] : []),
    { value: "file", label: "Technical" },
  ]);
  let tab = $state("");
  $effect(() => { if (!items.some((item) => item.value === tab)) tab = items[0].value; });

  const THUMB_W = 224;
  let probed = $derived(file?.probe?.video);
  let stream = $derived(probed ? { codec: probed.codec, profile: probed.profile, level: probed.level, width: probed.width, height: probed.height, frameRate: probed.frame_rate, bitDepth: probed.bit_depth, pixelFormat: probed.pixel_format?.toUpperCase(), range: dynamicRange(probed) || (probed.transfer && probed.transfer !== "sdr" ? probed.transfer.toUpperCase() : "SDR") } : {});
  let video = $derived([
    ["Video", [codecName(stream.codec), profileName(stream.profile), levelName(stream.codec, stream.level)].filter(Boolean).join(" · ")],
    ["Resolution", stream.width && stream.height ? `${stream.width} × ${stream.height}${stream.frameRate ? ` @ ${stream.frameRate.toFixed(3).replace(/\.?0+$/, "")} fps` : ""}` : null],
    ["Dynamic range", stream.range],
    ["Bit depth", stream.bitDepth ? `${stream.bitDepth}-bit${stream.pixelFormat ? ` · ${stream.pixelFormat}` : ""}` : null],
    ["Audio", audio ? audioFormat(audio) : null],
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
  .facts { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 18px 32px; max-width: 980px; }
  dt { font-size: 0.76rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-3); margin-bottom: 4px; }
  dd { font-size: 0.94rem; font-weight: 550; word-break: break-word; }
</style>
