<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import AndroidLogo from "phosphor-svelte/lib/AndroidLogo";
  import AppleLogo from "phosphor-svelte/lib/AppleLogo";
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import Browser from "phosphor-svelte/lib/Browser";
  import GithubLogo from "phosphor-svelte/lib/GithubLogo";
  import LinuxLogo from "phosphor-svelte/lib/LinuxLogo";
  import Television from "phosphor-svelte/lib/Television";
  import WindowsLogo from "phosphor-svelte/lib/WindowsLogo";
  import PosterWall from "$components/PosterWall.svelte";
  import { links } from "$lib/site";

  const platforms = [
    { icon: Browser, label: "Web" },
    { icon: AndroidLogo, label: "Android", fill: true },
    { icon: AppleLogo, label: "iOS & macOS", fill: true },
    { icon: WindowsLogo, label: "Windows", fill: true },
    { icon: LinuxLogo, label: "Linux", fill: true },
    { icon: Television, label: "Smart TVs" },
  ];
</script>

<section class="hero">
  <PosterWall />
  <div class="shade" aria-hidden="true"></div>
  <div class="wrap content">
    <a class="status" href="#open-source"><span class="dot warn"></span>Pre-alpha<span class="sep"></span><span class="faint">Open source, in active development</span><ArrowRight size={13} weight="bold" /></a>
    <h1>Your media,<br /><span>your server.</span></h1>
    <p class="lede">NoFlex is a self-hosted media server for your movies, shows, music and live TV. It gets the next episode ready before you ask, plays it on every screen you own, and answers to no one but you.</p>
    <div class="ctas">
      <a class="btn primary lg" href="/demo">Try the demo<ArrowRight class="arrow" size={18} weight="bold" /></a>
      <a class="btn glass lg" href={links.repo}><GithubLogo size={19} weight="fill" />View on GitHub</a>
    </div>
    <ul class="platforms" aria-label="Apps for">
      {#each platforms as { icon: Icon, label, fill }, i}
        <li style:--i={i}><Icon size={17} weight={fill ? "fill" : "regular"} />{label}</li>
      {/each}
    </ul>
  </div>
  <a class="cue" href="#tour" aria-label="Scroll to the tour"><span></span></a>
</section>

<style>
  .hero { position: relative; isolation: isolate; min-height: max(680px, 100svh); display: flex; align-items: center; padding: calc(var(--topbar-h) + 40px) 0 120px; overflow: hidden; }
  .shade {
    position: absolute;
    inset: 0;
    z-index: 0;
    pointer-events: none;
    background:
      radial-gradient(ellipse 60% 50% at 18% 52%, rgb(7 8 11 / 0.92), transparent 75%),
      linear-gradient(90deg, rgb(7 8 11 / 0.96) 0%, rgb(7 8 11 / 0.7) 34%, rgb(7 8 11 / 0.1) 68%, rgb(7 8 11 / 0.35) 100%),
      linear-gradient(0deg, var(--bg) 0%, rgb(7 8 11 / 0) 30%),
      linear-gradient(180deg, rgb(7 8 11 / 0.7) 0%, transparent 22%);
  }
  .content { position: relative; z-index: 1; display: grid; justify-items: start; gap: 26px; }
  .content > * { animation: hero-in 1s var(--ease-out) both; }
  .content > :nth-child(2) { animation-delay: 80ms; }
  .content > :nth-child(3) { animation-delay: 180ms; }
  .content > :nth-child(4) { animation-delay: 260ms; }
  .content > :nth-child(5) { animation-delay: 340ms; }
  @keyframes hero-in { from { opacity: 0; transform: translateY(18px); filter: blur(6px); } }

  .status { display: inline-flex; align-items: center; gap: 9px; height: 32px; padding: 0 14px 0 12px; border-radius: 999px; background: rgb(255 255 255 / 0.06); border: 1px solid var(--line); backdrop-filter: blur(14px); font-size: 0.82rem; font-weight: 600; transition: background var(--t), border-color var(--t); }
  .status:hover { background: rgb(255 255 255 / 0.1); border-color: var(--line-strong); }
  .status .sep { width: 1px; height: 12px; background: var(--line-strong); }
  .status :global(svg) { color: var(--text-3); transition: transform var(--t) var(--ease-spring); }
  .status:hover :global(svg) { transform: translateX(2px); }

  h1 { font-size: clamp(3.1rem, 8.4vw, 7.4rem); font-weight: 760; letter-spacing: -0.055em; line-height: 0.92; text-shadow: 0 6px 60px rgb(0 0 0 / 0.6); }
  h1 span { color: var(--accent); }
  .lede { max-width: 54ch; color: var(--text-2); font-size: clamp(1.04rem, 1.4vw, 1.2rem); line-height: 1.6; }
  .ctas { display: flex; flex-wrap: wrap; gap: 12px; }
  .platforms { display: flex; flex-wrap: wrap; gap: 8px 20px; margin-top: 10px; color: var(--text-3); font-size: 0.84rem; font-weight: 550; }
  .platforms li { display: inline-flex; align-items: center; gap: 7px; animation: fade-up 700ms var(--ease-out) both; animation-delay: calc(500ms + var(--i) * 60ms); }

  .cue { position: absolute; z-index: 1; left: 50%; bottom: 28px; width: 24px; height: 38px; margin-left: -12px; border-radius: 99px; border: 1.5px solid var(--line-strong); display: grid; justify-items: center; padding-top: 7px; transition: border-color var(--t); }
  .cue:hover { border-color: var(--text-3); }
  .cue span { width: 3px; height: 8px; border-radius: 9px; background: var(--text-2); animation: cue 2s var(--ease-in-out) infinite; }
  @keyframes cue { 0% { transform: translateY(0); opacity: 0; } 30% { opacity: 1; } 70% { transform: translateY(12px); opacity: 0; } 100% { opacity: 0; } }

  @media (max-width: 759px) {
    .hero { align-items: flex-end; }
    .shade { background: linear-gradient(0deg, var(--bg) 8%, rgb(7 8 11 / 0.85) 50%, rgb(7 8 11 / 0.3) 100%); }
    .cue { display: none; }
  }
</style>
