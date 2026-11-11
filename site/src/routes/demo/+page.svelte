<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { onMount } from "svelte";
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import GithubLogo from "phosphor-svelte/lib/GithubLogo";
  import Terminal from "$components/Terminal.svelte";
  import { prefersReducedMotion } from "$lib/actions";
  import { backdrop, DEMO_URL, links, stills } from "$lib/site";

  let index = $state(0);
  let current = $derived(stills[index]);
  onMount(() => {
    if (DEMO_URL) { location.replace(DEMO_URL); return; }
    if (prefersReducedMotion()) return;
    const timer = setInterval(() => (index = (index + 1) % stills.length), 5200);
    return () => clearInterval(timer);
  });
</script>

<svelte:head>
  <title>Demo · NoFlex</title>
  <meta name="description" content="A public NoFlex demo is on its way. Until then, run one yourself in a minute." />
</svelte:head>

<section class="demo" style:--ambient={current.ambient}>
  <div class="stage" aria-hidden="true">
    {#each stills as item, i}
      <img src={backdrop(item.slug)} alt="" width="1600" height="900" class:active={i === index} decoding="async" />
    {/each}
  </div>
  <div class="shade" aria-hidden="true"></div>
  <div class="wrap content">
    <p class="eyebrow">Live demo</p>
    <h1>Coming soon.</h1>
    <p class="lede">We're putting up a public server you can sign in to and click around, with Blender's open movies on it. Until it's ready, you can run the same demo on your own machine.</p>
    <div class="ctas">
      <a class="btn primary lg" href={links.releases}><GithubLogo size={19} weight="fill" />Watch for releases</a>
      <a class="btn glass lg" href="/download">Get NoFlex<ArrowRight class="arrow" size={18} weight="bold" /></a>
    </div>
    <div class="terminal">
      <Terminal title="Run the demo yourself" lines={["git clone https://github.com/NoFlexMedia/noflex", "cd noflex && nix develop", "just demo up"]} note="Then open http://127.0.0.1:28793 and sign in as admin, with the password it prints." />
    </div>
  </div>
</section>

<style>
  .demo { position: relative; isolation: isolate; min-height: 100svh; display: flex; align-items: center; padding: calc(var(--topbar-h) + 48px) 0 80px; overflow: hidden; }
  .stage { position: absolute; inset: 0; z-index: -2; }
  .stage img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: 0; transform: scale(1.08); transition: opacity 1.4s var(--ease-in-out), transform 9s linear; }
  .stage img.active { opacity: 0.55; transform: scale(1); }
  .shade { position: absolute; inset: 0; z-index: -1; background: radial-gradient(ellipse 80% 70% at 20% 60%, rgb(var(--ambient) / 0.25), transparent 70%), linear-gradient(90deg, rgb(7 8 11 / 0.95), rgb(7 8 11 / 0.7) 45%, rgb(7 8 11 / 0.3)), linear-gradient(0deg, var(--bg), transparent 40%); transition: background 1.4s; }
  .content { display: grid; justify-items: start; gap: 22px; }
  .content > * { animation: fade-up 900ms var(--ease-out) both; }
  .content > :nth-child(2) { animation-delay: 80ms; }
  .content > :nth-child(3) { animation-delay: 160ms; }
  .content > :nth-child(4) { animation-delay: 240ms; }
  .content > :nth-child(5) { animation-delay: 320ms; }
  h1 { font-size: clamp(3rem, 9vw, 7rem); font-weight: 760; letter-spacing: -0.055em; line-height: 0.92; }
  .lede { max-width: 54ch; color: var(--text-2); font-size: clamp(1.04rem, 1.4vw, 1.2rem); }
  .ctas { display: flex; flex-wrap: wrap; gap: 12px; }
  .terminal { width: min(100%, 560px); margin-top: 12px; }
</style>
