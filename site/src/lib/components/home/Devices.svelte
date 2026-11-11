<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { scrollProgress } from "$lib/actions";

  const views = [
    { id: "web", label: "Browser", caption: "The web client, built into the server: nothing to install." },
    { id: "phone", label: "Phone", caption: "The app for Android and iOS, with Smart Downloads for the commute." },
    { id: "tv", label: "TV", caption: "One TV app for Samsung, LG, Android TV, Fire TV and Apple TV, drawn on the GPU." },
  ];
  let active = $state("web");
  let stage = $state();
  let tabs = $state();
  let indicator = $state({ x: 0, w: 0 });

  $effect(() => {
    active;
    const place = () => {
      const button = tabs?.querySelector('[aria-selected="true"]');
      if (button) indicator = { x: button.offsetLeft, w: button.offsetWidth };
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(tabs);
    return () => observer.disconnect();
  });
</script>

<div class="devices" use:scrollProgress={(p) => stage?.style.setProperty("--p", Math.min(1, Math.max(0, (p - 0.08) / 0.42)).toFixed(4))}>
  <div class="stage" bind:this={stage} data-active={active}>
    <figure class="tv" aria-label="The TV app">
      <div class="bezel"><img src="/shots/tv-home.webp" alt="The NoFlex TV app's home screen, featuring Charge, with Continue watching below" width="1920" height="1080" loading="lazy" decoding="async" /></div>
      <span class="foot"></span>
    </figure>
    <figure class="browser" aria-label="The web client">
      <div class="chrome"><span></span><span></span><span></span><i>noflex.home.arpa</i></div>
      <img src="/shots/web-detail.webp" alt="Sintel's title page in the NoFlex web client: poster, logo, rating and Play" width="1600" height="1000" loading="lazy" decoding="async" />
    </figure>
    <figure class="phone" aria-label="The phone app">
      <img src="/shots/phone-home.webp" alt="The NoFlex app on a phone, featuring Big Buck Bunny, with a bottom navigation bar" width="600" height="1333" loading="lazy" decoding="async" />
    </figure>
  </div>

  <div class="controls">
    <div class="tabs" role="tablist" aria-label="Show a device" bind:this={tabs}>
      <span class="indicator" style:width="{indicator.w}px" style:transform="translateX({indicator.x}px)"></span>
      {#each views as view}
        <button role="tab" aria-selected={active === view.id} onclick={() => (active = view.id)}>{view.label}</button>
      {/each}
    </div>
    {#key active}<p class="caption">{views.find((v) => v.id === active)?.caption}</p>{/key}
  </div>
</div>

<style>
  .devices { display: grid; gap: 36px; justify-items: center; }
  .stage {
    --p: 1;
    --q: calc(1 - var(--p));
    position: relative;
    width: min(100%, 1100px);
    aspect-ratio: 16 / 9.4;
    perspective: 1800px;
    transform-style: preserve-3d;
  }
  figure { position: absolute; margin: 0; transform-style: preserve-3d; transition: transform 900ms var(--ease-out), filter 600ms var(--ease-out), opacity 600ms; will-change: transform; }
  img { width: 100%; height: auto; }

  /* Each device opens out from a stack as the section scrolls in, then settles. */
  .tv { left: 3%; top: 0; width: 66%; transform: translate3d(calc(var(--q) * 40%), calc(var(--q) * 20%), calc(-120px - var(--q) * 300px)) rotateY(calc(var(--q) * 34deg)) rotateX(calc(var(--q) * 12deg)); }
  .bezel { padding: 1.1%; border-radius: 10px; background: linear-gradient(180deg, #1b1d22, #0d0e11); box-shadow: 0 0 0 1px rgb(255 255 255 / 0.06), var(--shadow-3); }
  .bezel img { border-radius: 4px; }
  .foot { display: block; width: 22%; height: 10px; margin: 0 auto; border-radius: 0 0 8px 8px; background: #121317; }
  .browser { left: 24%; top: 22%; width: 62%; border-radius: 12px; overflow: hidden; background: #111317; box-shadow: 0 0 0 1px rgb(255 255 255 / 0.08), 0 50px 100px -30px rgb(0 0 0 / 0.95); transform: translate3d(calc(var(--q) * -10%), calc(var(--q) * 40%), calc(var(--q) * -200px)) rotateY(calc(var(--q) * -26deg)) rotateX(calc(var(--q) * 22deg)); }
  .chrome { display: flex; align-items: center; gap: 6px; height: 26px; padding: 0 12px; background: #16181d; border-bottom: 1px solid var(--line); }
  .chrome span { width: 9px; height: 9px; border-radius: 50%; background: rgb(255 255 255 / 0.14); }
  .chrome i { margin: 0 auto; padding: 2px 30px; border-radius: 6px; background: rgb(255 255 255 / 0.05); color: var(--text-3); font-size: 0.66rem; font-style: normal; }
  .phone { right: 2%; top: 34%; width: 19%; padding: 0.9%; border-radius: 30px; background: #0d0e11; box-shadow: 0 0 0 1px rgb(255 255 255 / 0.1), 0 50px 90px -24px rgb(0 0 0 / 0.95); transform: translate3d(calc(var(--q) * -60%), calc(var(--q) * 30%), calc(var(--q) * 160px + 60px)) rotateY(calc(var(--q) * -40deg)) rotateZ(calc(var(--q) * 8deg)); }
  .phone img { border-radius: 24px; }

  /* The chosen device comes forward; the others step back. */
  [data-active="web"] .tv, [data-active="web"] .phone, [data-active="phone"] .tv, [data-active="phone"] .browser, [data-active="tv"] .browser, [data-active="tv"] .phone { filter: brightness(0.55) saturate(0.8); }
  [data-active="tv"] .tv { transform: translate3d(8%, 10%, 160px); z-index: 3; }
  [data-active="phone"] .phone { transform: translate3d(-40%, -12%, 220px) scale(1.12); z-index: 3; }
  [data-active="tv"] .browser { transform: translate3d(0, 10%, -60px); }
  .stage:not([data-active="tv"]) .tv { transition-duration: 900ms; }

  .controls { display: grid; justify-items: center; gap: 14px; text-align: center; }
  .caption { color: var(--text-2); max-width: 52ch; min-height: 3em; animation: fade-up 420ms var(--ease-out) both; }
  @media (max-width: 599px) {
    .stage { aspect-ratio: 4 / 3.6; }
    .tv { width: 80%; left: 0; }
    .browser { width: 70%; left: 10%; top: 26%; }
    .phone { width: 30%; top: 30%; border-radius: 20px; }
    .phone img { border-radius: 15px; }
  }
</style>
