<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script module>
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
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
      {#if kind !== "logo"}
        <Glyph size={kind === "poster" ? 22 : 28} weight="light" />
        {#if kind === "poster" && work?.title}<span>{work.title}</span>{/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .art { position: relative; overflow: hidden; background: hsl(var(--h) 22% 12%); isolation: isolate; }
  .art.rounded { border-radius: var(--r); }
  .art.poster { aspect-ratio: 2 / 3; }
  .art.backdrop, .art.thumb { aspect-ratio: 16 / 9; }
  .art.logo { background: none; }
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
