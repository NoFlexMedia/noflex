<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import AndroidLogo from "phosphor-svelte/lib/AndroidLogo";
  import AppleLogo from "phosphor-svelte/lib/AppleLogo";
  import ArrowUpRight from "phosphor-svelte/lib/ArrowUpRight";
  import Browser from "phosphor-svelte/lib/Browser";
  import LinuxLogo from "phosphor-svelte/lib/LinuxLogo";
  import Package from "phosphor-svelte/lib/Package";
  import Television from "phosphor-svelte/lib/Television";
  import WindowsLogo from "phosphor-svelte/lib/WindowsLogo";
  import { siNixos } from "simple-icons";
  import SimpleIcon from "$components/SimpleIcon.svelte";
  import Terminal from "$components/Terminal.svelte";
  import { reveal } from "$lib/actions";
  import { links } from "$lib/site";

  const nix = `# flake.nix
inputs.noflex.url = "github:NoFlexMedia/noflex";

# configuration.nix
imports = [ inputs.noflex.nixosModules.default ];
services.noflex = {
  enable = true;
  package = inputs.noflex.packages.\${pkgs.system}.default;
  openFirewall = true;
  mediaPaths = [ "/media/movies" "/media/shows" ];
};`;

  const stores = [
    { icon: AppleLogo, fill: true, name: "App Store", detail: "iPhone, iPad, Mac and Apple TV" },
    { icon: AndroidLogo, fill: true, name: "Google Play", detail: "Phones, tablets and Android TV" },
    { icon: Package, name: "Amazon Appstore", detail: "Fire TV" },
    { icon: Television, name: "Samsung and LG", detail: "Tizen and webOS TVs" },
    { icon: WindowsLogo, fill: true, name: "Windows", detail: "Microsoft Store" },
    { icon: LinuxLogo, fill: true, name: "Linux", detail: "Desktop app" },
  ];
</script>

<svelte:head>
  <title>Get NoFlex</title>
  <meta name="description" content="Install the NoFlex server on Linux or NixOS, and find the NoFlex apps for your devices." />
</svelte:head>

<div class="download wrap">
  <header>
    <p class="eyebrow">Get NoFlex</p>
    <h1>Two pieces, both yours.</h1>
    <p class="lede">Run the server on a machine you own, a home server, a NAS or an old laptop. Then watch from a browser or the apps.</p>
    <p class="note"><span class="dot warn"></span>NoFlex is pre-alpha. It works, but expect rough edges, and don't trust it with the only copy of anything.</p>
  </header>

  <section use:reveal>
    <h2><span class="step">1</span>Run the server</h2>
    <div class="servers">
      <div class="panel">
        <div class="head"><SimpleIcon icon={siNixos} size={20} /><h3>NixOS</h3><span class="chip accent">Recommended</span></div>
        <p>A hardened service: read-only access to your media, backups on a schedule, and hardware transcoding set up for you.</p>
        <pre class="code"><code>{nix}</code></pre>
        <a class="more" href={links.deployment}>Deployment guide<ArrowUpRight size={14} /></a>
      </div>
      <div class="stack">
        <div class="panel">
          <div class="head"><LinuxLogo size={20} weight="fill" /><h3>Linux bundle</h3><span class="chip">First release</span></div>
          <p>One archive with the server, its plugins, our own ffmpeg build and the search model. Unpack it and run it. Bundles come with the first release, on GitHub.</p>
          <a class="more" href={links.releases}>Releases<ArrowUpRight size={14} /></a>
        </div>
        <div class="panel">
          <div class="head"><Browser size={20} /><h3>From source</h3></div>
          <p>Everything builds from one Nix dev shell. The demo starts a server with a real library in one command.</p>
          <Terminal title="Shell" lines={["git clone https://github.com/NoFlexMedia/noflex", "cd noflex && nix develop", "just demo up"]} />
          <a class="more" href={links.gettingStarted}>Getting started<ArrowUpRight size={14} /></a>
        </div>
      </div>
    </div>
  </section>

  <section use:reveal>
    <h2><span class="step">2</span>Watch anywhere</h2>
    <div class="panel web">
      <Browser size={22} />
      <p><strong>The web client is already there.</strong> Open your server's address, such as <code>http://your-server:8096</code>, in any modern browser, phones included.</p>
    </div>
    <ul class="stores">
      {#each stores as store, i}
        {@const Icon = store.icon}
        <li style:--i={i}>
          <Icon size={24} weight={store.fill ? "fill" : "regular"} />
          <span><strong>{store.name}</strong><small>{store.detail}</small></span>
          <span class="soon">Coming soon</span>
        </li>
      {/each}
    </ul>
  </section>
</div>

<style>
  .download { padding-top: calc(var(--topbar-h) + clamp(48px, 9vh, 96px)); padding-bottom: 120px; }
  header { display: grid; gap: 14px; max-width: 720px; margin-bottom: 56px; animation: fade-up 700ms var(--ease-out) both; }
  h1 { font-size: clamp(2.4rem, 5.6vw, 4.2rem); font-weight: 740; letter-spacing: -0.05em; line-height: 1; }
  .lede { color: var(--text-2); font-size: clamp(1.04rem, 1.4vw, 1.2rem); }
  .note { display: flex; align-items: baseline; gap: 10px; color: var(--text-3); font-size: 0.9rem; }
  section { margin-top: 56px; }
  section > h2 { display: flex; align-items: center; gap: 14px; font-size: clamp(1.5rem, 2.6vw, 2rem); letter-spacing: -0.035em; margin-bottom: 22px; }
  .step { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 50%; background: var(--accent); color: var(--accent-ink); font-size: 1rem; font-weight: 750; letter-spacing: 0; }
  .servers { display: grid; grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr); gap: 14px; align-items: start; }
  .stack { display: grid; gap: 14px; }
  .panel { display: grid; gap: 14px; align-content: start; }
  .head { display: flex; align-items: center; gap: 10px; color: var(--text-2); }
  .head h3 { color: var(--text); font-size: 1.1rem; margin-right: auto; }
  .panel > p { color: var(--text-2); font-size: 0.94rem; }
  .code { margin: 0; padding: 16px 18px; border-radius: var(--r); background: #0a0b0e; border: 1px solid var(--line); overflow-x: auto; font-size: 0.84rem; line-height: 1.7; color: var(--text-2); }
  .more { display: inline-flex; align-items: center; gap: 4px; justify-self: start; color: var(--text); font-size: 0.88rem; font-weight: 600; transition: color var(--t); }
  .more:hover { color: var(--accent); }
  .web { grid-template-columns: auto minmax(0, 1fr); align-items: center; color: var(--text-2); margin-bottom: 14px; }
  .web strong { color: var(--text); }
  .web code { color: var(--text); background: var(--surface-2); padding: 1px 6px; border-radius: 5px; }
  .stores { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr)); gap: 10px; }
  .stores li { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 14px; padding: 16px 18px; border-radius: var(--r-l); border: 1px dashed var(--line-strong); color: var(--text-2); animation: fade-up 600ms var(--ease-out) both; animation-delay: calc(var(--i) * 50ms); }
  .stores span:not(.soon) { display: grid; }
  .stores strong { color: var(--text); font-size: 0.94rem; }
  .stores small { color: var(--text-3); font-size: 0.8rem; }
  .soon { font-size: 0.72rem; font-weight: 650; color: var(--text-3); white-space: nowrap; }
  @media (max-width: 899px) { .servers { grid-template-columns: 1fr; } }
</style>
