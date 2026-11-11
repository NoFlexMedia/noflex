<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowDown from "phosphor-svelte/lib/ArrowDown";
  import { prefersReducedMotion, reveal } from "$lib/actions";
  import { art, poster } from "$lib/site";

  const bySlug = Object.fromEntries(art.map((a) => [a.slug, a]));
  // Shapes the scanner's parser tests cover (noflex-scanner/src/parser.rs):
  // release names, edition tags, and season folders named for their encode.
  const examples = [
    { slug: "sintel", parts: [["Sintel", "title"], [".", ""], ["2010", "year"], [".2160p.UHD.BluRay.x265.HDR10-GROUP", "noise"], [".mkv", "ext"]], result: "Sintel (2010)", detail: "Movie" },
    { slug: "tears-of-steel", parts: [["Tears of Steel", "title"], [" (", ""], ["2012", "year"], [") ", ""], ["{edition-Extended Cut}", "edition"], [".mkv", "ext"]], result: "Tears of Steel (2012)", detail: "Edition: Extended Cut" },
    { slug: "caminandes", parts: [["Caminandes Season 1 (1080p Tigole)", "folder"], ["/", ""], ["Caminandes", "title"], [" - ", ""], ["S01E02", "episode"], [" - ", ""], ["Gran Dillama", "name"], [".mp4", "ext"]], result: "Caminandes", detail: "Season 1 · Episode 2 · Gran Dillama" },
  ];
  const legend = { title: "Title", year: "Year", edition: "Edition", episode: "Episode", folder: "Season folder", name: "Episode title" };

  let index = $state(0);
  let phase = $state("parsed");
  let timers = [];
  const wait = (ms) => new Promise((resolve) => timers.push(setTimeout(resolve, ms)));

  async function start() {
    if (prefersReducedMotion()) return;
    for (let i = 0; ; i = (i + 1) % examples.length) {
      index = i;
      phase = "raw";
      await wait(900);
      phase = "tokens";
      await wait(1300);
      phase = "parsed";
      await wait(3200);
    }
  }
  $effect(() => () => timers.forEach(clearTimeout));
  let example = $derived(examples[index]);
  let work = $derived(bySlug[example.slug]);
</script>

<div class="parser panel" use:reveal={start} data-phase={phase}>
  {#key index}
    <p class="file mono" aria-label="File: {example.parts.map((p) => p[0]).join('')}">
      {#each example.parts as [text, kind], i}<span class={kind} style:--i={i}>{text}</span>{/each}
    </p>
    <div class="legend" aria-hidden="true">
      {#each [...new Set(example.parts.map((p) => p[1]).filter((k) => legend[k]))] as kind, i}
        <span class="key {kind}" style:--i={i}><i></i>{legend[kind]}</span>
      {/each}
    </div>
    <span class="arrow" aria-hidden="true"><ArrowDown size={16} /></span>
    <div class="result">
      <img src={poster(example.slug)} alt="" width="480" height="720" loading="lazy" decoding="async" />
      <div>
        <strong>{example.result}</strong>
        <small>{example.detail}</small>
        <span class="matched"><span class="dot on"></span>Matched · {work.kind}</span>
      </div>
    </div>
  {/key}
</div>

<style>
  .parser { display: grid; gap: 16px; background: rgb(14 16 21 / 0.9); box-shadow: var(--shadow-3); }
  .file { padding: 14px 16px; border-radius: var(--r); background: rgb(0 0 0 / 0.3); border: 1px solid var(--line); font-size: 0.88rem; line-height: 1.7; word-break: break-all; color: var(--text-2); }
  .file span { border-radius: 4px; transition: background var(--t-slow) var(--ease-out), color var(--t-slow) var(--ease-out), opacity var(--t-slow); transition-delay: calc(var(--i) * 60ms); }
  [data-phase="tokens"] .file .title, [data-phase="parsed"] .file .title { background: rgb(212 255 90 / 0.16); color: var(--accent); }
  [data-phase="tokens"] .file .year, [data-phase="parsed"] .file .year { background: rgb(165 220 255 / 0.16); color: var(--focus); }
  [data-phase="tokens"] .file :is(.edition, .episode), [data-phase="parsed"] .file :is(.edition, .episode) { background: rgb(255 193 100 / 0.16); color: var(--warning); }
  [data-phase="tokens"] .file :is(.folder, .name), [data-phase="parsed"] .file :is(.folder, .name) { background: rgb(110 231 168 / 0.13); color: var(--success); }
  [data-phase="tokens"] .file :is(.noise, .ext), [data-phase="parsed"] .file :is(.noise, .ext) { opacity: 0.35; }
  .legend { display: flex; flex-wrap: wrap; gap: 6px 14px; min-height: 20px; }
  .key { display: inline-flex; align-items: center; gap: 6px; font-size: 0.76rem; color: var(--text-3); opacity: 0; transform: translateY(4px); transition: opacity var(--t) var(--ease-out), transform var(--t) var(--ease-out); transition-delay: calc(var(--i) * 70ms); }
  .key i { width: 8px; height: 8px; border-radius: 2px; }
  .key.title i { background: var(--accent); }
  .key.year i { background: var(--focus); }
  .key:is(.edition, .episode) i { background: var(--warning); }
  .key:is(.folder, .name) i { background: var(--success); }
  [data-phase="tokens"] .key, [data-phase="parsed"] .key { opacity: 1; transform: none; }
  .arrow { justify-self: center; color: var(--text-3); opacity: 0.3; transition: opacity var(--t); }
  [data-phase="parsed"] .arrow { opacity: 1; }
  .result { display: grid; grid-template-columns: 64px minmax(0, 1fr); gap: 14px; align-items: center; padding: 12px; border-radius: var(--r); background: var(--surface-2); opacity: 0.25; filter: blur(3px); transform: translateY(6px) scale(0.98); transition: opacity 600ms var(--ease-out), filter 600ms var(--ease-out), transform 600ms var(--ease-spring); }
  [data-phase="parsed"] .result { opacity: 1; filter: none; transform: none; }
  .result img { width: 100%; height: auto; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 7px; }
  .result div { display: grid; gap: 3px; }
  .result strong { font-size: 1rem; }
  .result small { color: var(--text-2); font-size: 0.84rem; }
  .matched { display: inline-flex; align-items: center; gap: 8px; margin-top: 4px; color: var(--text-3); font-size: 0.76rem; font-weight: 550; }
</style>
