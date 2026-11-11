<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { prefersReducedMotion, reveal, tilt } from "$lib/actions";

  const shots = [
    { id: "detail", label: "Title page", src: "/shots/web-detail.webp", w: 1600, h: 1000, alt: "Sintel's title page in the web client: poster, logo, rating, codec badges, Play, and cast", caption: "Every title page leads with its artwork, and tints itself to match." },
    { id: "player", label: "Player", src: "/shots/web-player.webp", w: 1600, h: 1000, alt: "The web player paused on Sintel at 5:18, with its controls showing", caption: "Scrub with thumbnails, skip the intro, pick a track. It plays the original whenever it can." },
    { id: "together", label: "Watch together", src: "/shots/web-watch-together.webp", w: 1600, h: 1000, alt: "A watch-together room for Sintel, waiting to start, with its member list", caption: "Open a room, send the link, and press play together. Pauses and seeks stay in sync." },
    { id: "palette", label: "Jump to anything", src: "/shots/web-command-palette.webp", w: 1600, h: 1000, alt: "The command palette open over the library, listing matching titles", caption: "Ctrl K finds any title, person or page from anywhere." },
    { id: "tv", label: "On the TV", src: "/shots/tv-detail.webp", w: 1920, h: 1080, alt: "Sintel's title page in the TV app, with Play focused", caption: "Built for a remote: big targets, quick focus, and native video under the interface." },
    { id: "signin", label: "Sign in by code", src: "/shots/tv-signin.webp", w: 1920, h: 1080, alt: "The TV's sign-in screen showing a six-letter code and a QR code", caption: "No typing passwords with a remote. Scan the code and approve it from your phone." },
  ];

  let active = $state(0);
  let tabs = $state();
  let indicator = $state({ x: 0, w: 0 });
  let timer = 0;

  $effect(() => {
    active;
    const button = tabs?.querySelector('[aria-selected="true"]');
    if (!button) return;
    indicator = { x: button.offsetLeft, w: button.offsetWidth };
    if (tabs.scrollWidth > tabs.clientWidth) tabs.scrollTo({ left: button.offsetLeft - (tabs.clientWidth - button.offsetWidth) / 2, behavior: "smooth" });
  });
  function start() {
    if (prefersReducedMotion()) return;
    timer = setInterval(() => (active = (active + 1) % shots.length), 6000);
  }
  // Choosing a tab means the reader has taken over.
  function choose(i) { clearInterval(timer); active = i; }
  $effect(() => () => clearInterval(timer));
</script>

<div class="gallery" use:reveal={start}>
  <div class="tabs" role="tablist" aria-label="Screens" bind:this={tabs}>
    <span class="indicator" style:width="{indicator.w}px" style:transform="translateX({indicator.x}px)"></span>
    {#each shots as shot, i}
      <button role="tab" id="shot-tab-{shot.id}" aria-controls="shot-panel" aria-selected={i === active} onclick={() => choose(i)}>{shot.label}</button>
    {/each}
  </div>
  <div class="frame" use:tilt={3} id="shot-panel" role="tabpanel" aria-labelledby="shot-tab-{shots[active].id}">
    {#each shots as shot, i}
      <img src={shot.src} alt={i === active ? shot.alt : ""} width={shot.w} height={shot.h} class:active={i === active} loading="lazy" decoding="async" />
    {/each}
    <span class="glare" aria-hidden="true"></span>
  </div>
  {#key active}<p class="caption">{shots[active].caption}</p>{/key}
</div>

<style>
  .gallery { display: grid; justify-items: center; gap: 22px; }
  .tabs { max-width: 100%; }
  .frame {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 10;
    border-radius: var(--r-l);
    overflow: hidden;
    background: var(--bg-raised);
    box-shadow: 0 0 0 1px rgb(255 255 255 / 0.08), 0 60px 120px -40px rgb(0 0 0 / 0.95), 0 0 120px -40px rgb(var(--ambient) / 0.5);
    transform: perspective(1600px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg));
    transition: transform 700ms var(--ease-out);
  }
  .frame:global([data-tilting]) { transition-duration: 160ms; }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; object-position: top; opacity: 0; transform: scale(1.03); transition: opacity 700ms var(--ease-out), transform 1.2s var(--ease-out); }
  img.active { opacity: 1; transform: none; }
  .glare { position: absolute; inset: 0; pointer-events: none; opacity: 0; background: radial-gradient(circle at var(--gx, 50%) var(--gy, 0%), rgb(255 255 255 / 0.1), transparent 50%); transition: opacity 400ms; }
  .frame:global([data-tilting]) .glare { opacity: 1; }
  .caption { color: var(--text-2); text-align: center; max-width: 56ch; min-height: 3em; animation: fade-up 420ms var(--ease-out) both; }
</style>
