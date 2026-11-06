<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script module>
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
  import Tag from "phosphor-svelte/lib/Tag";
  import Television from "phosphor-svelte/lib/Television";

  const KIND_ICON = { MOVIE: FilmStrip, SERIES: Television, SEASON: Television, EPISODE: Television, ALBUM: MusicNotes, TRACK: MusicNotes, ARTIST: MusicNotes, AUDIO_BOOK: BookOpen, PHOTO: ImageIcon };

  function hue(text = "") {
    let hash = 0;
    for (let index = 0; index < text.length; index++) hash = (hash * 31 + text.charCodeAt(index)) | 0;
    return Math.abs(hash) % 360;
  }
</script>

<script>
  import { pickImage, sized, srcset } from "$lib/images.js";
  import { INDEXING_LABEL, watchIndexing } from "$lib/indexing.svelte.js";

  /** `width` is the rendered CSS width, used to pick srcset candidates. */
  let { work, kind = "poster", width = 220, eager = false, rounded = true, vt = null, fallback = null } = $props();
  let Glyph = $derived(KIND_ICON[work?.kind] ?? FilmStrip);

  const RATIO = { poster: 1.5, backdrop: 0.5625, thumb: 0.5625, logo: undefined };
  // `fallback` lends its art (e.g. a show's) to children that have none.
  let url = $derived(pickImage(work?.images, kind) ?? pickImage(fallback?.images, kind === "thumb" ? "backdrop" : kind));
  let ratio = $derived(RATIO[kind]);
  let loaded = $state(false);
  let failed = $state(false);
  let h = $derived(hue(work?.title));

  $effect(() => { url; loaded = false; failed = false; });
  // Backdrops and logos sit behind or beside a poster that already says so. A linked
  // server's title can't be followed from here, so it just shows what it has.
  let indexing = $derived((kind === "poster" || kind === "thumb") && !work?.server ? work?.indexing : null);
  let workId = $derived(work?.id);
  $effect(() => { if (indexing && workId) return watchIndexing(workId); });
</script>

<div class="art {kind}" class:rounded class:loaded style:--h={h} style:view-transition-name={vt}>
  {#if url && !failed}
    <img
      src={sized(url, width, ratio ? width * ratio : undefined)}
      srcset={srcset(url, width, ratio)}
      sizes="{width}px"
      alt=""
      loading={eager ? "eager" : "lazy"}
      fetchpriority={eager ? "high" : "auto"}
      decoding="async"
      onload={() => (loaded = true)}
      onerror={() => (failed = true)}
    />
  {/if}
  {#if !url || failed || !loaded}
    <div class="fallback" class:pending={url && !failed} aria-hidden="true">
      {#if kind !== "logo" && !indexing}
        <Glyph size={kind === "poster" ? 22 : 28} weight="light" />
        {#if kind === "poster" && work?.title}<span>{work.title}</span>{/if}
      {/if}
    </div>
  {/if}
  {#if indexing}
    <div class="indexing" class:on-art={url && !failed} role="status">
      <span class="tag"><Tag size={url && !failed ? 14 : 24} weight="duotone" /></span>
      <span class="stage">{INDEXING_LABEL[indexing]}</span>
    </div>
  {/if}
</div>

<style>
  .art { position: relative; overflow: hidden; background: hsl(var(--h) 22% 12%); isolation: isolate; }
  .art.rounded { border-radius: var(--r); }
  .art.poster { aspect-ratio: 2 / 3; }
  .art.backdrop, .art.thumb { aspect-ratio: 16 / 9; }
  .art.logo { background: none; }
  .art { container-type: inline-size; }
  .indexing { position: absolute; inset: 0; z-index: 1; display: grid; place-content: center; justify-items: center; gap: 10px; padding: 8px; overflow: hidden; color: hsl(var(--h) 40% 88%); font-size: 0.78rem; font-weight: 600; text-align: center; animation: index-in 320ms var(--ease-out); }
  .indexing::before { content: ""; position: absolute; inset: 0; background: linear-gradient(105deg, transparent 30%, rgb(255 255 255 / 0.09) 50%, transparent 70%); transform: translateX(-100%); animation: index-sheen 2600ms ease-in-out infinite; }
  .tag { position: relative; display: grid; place-items: center; width: 52px; height: 52px; border-radius: 50%; background: rgb(255 255 255 / 0.09); }
  .tag::after { content: ""; position: absolute; inset: 0; border-radius: inherit; border: 1.5px solid currentColor; animation: index-ring 2000ms var(--ease-out) infinite; }
  .indexing.on-art { inset: 8px auto auto 8px; grid-auto-flow: column; align-items: center; gap: 6px; padding: 4px 10px 4px 6px; border-radius: 99px; background: rgb(0 0 0 / 0.7); color: var(--text); backdrop-filter: blur(6px); }
  /* A wide card's title sits along its bottom. */
  .thumb .indexing:not(.on-art) { place-content: start; grid-auto-flow: column; align-items: center; justify-items: start; padding: 14px; }
  .on-art::before { display: none; }
  .on-art .tag { width: 20px; height: 20px; background: none; }
  .on-art .tag::after { display: none; }
  .on-art .tag :global(svg) { animation: index-breathe 1600ms ease-in-out infinite; }
  @container (max-width: 120px) { .stage { display: none; } }
  @keyframes index-in { from { opacity: 0; } }
  @keyframes index-sheen { to { transform: translateX(100%); } }
  @keyframes index-ring { from { transform: scale(1); opacity: 0.55; } to { transform: scale(1.55); opacity: 0; } }
  @keyframes index-breathe { 50% { opacity: 0.45; } }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: 0; transform: scale(1.04); filter: blur(10px); transition: opacity 520ms var(--ease-out), transform 900ms var(--ease-out), filter 520ms var(--ease-out); }
  .logo img { object-fit: contain; object-position: left bottom; }
  .loaded img { opacity: 1; transform: none; filter: none; }
  .fallback {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 10px;
    padding: 14%;
    color: hsl(var(--h) 40% 78% / 0.7);
    background:
      radial-gradient(120% 80% at 0% 0%, hsl(var(--h) 50% 30% / 0.9), transparent 60%),
      radial-gradient(90% 70% at 100% 100%, hsl(calc(var(--h) + 40) 45% 22% / 0.9), transparent 70%),
      hsl(var(--h) 25% 10%);
    transition: opacity 400ms var(--ease-out);
  }
  .fallback.pending { opacity: 0.6; }
  .fallback span { font-weight: 700; font-size: clamp(0.8rem, 1.1vw, 1rem); line-height: 1.12; letter-spacing: -0.02em; color: hsl(var(--h) 30% 92% / 0.92); display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .backdrop .fallback, .thumb .fallback { align-items: center; justify-content: center; }
</style>
