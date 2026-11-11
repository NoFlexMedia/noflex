<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import ArrowUpRight from "phosphor-svelte/lib/ArrowUpRight";
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import ChartLine from "phosphor-svelte/lib/ChartLine";
  import DownloadSimple from "phosphor-svelte/lib/DownloadSimple";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import GitPullRequest from "phosphor-svelte/lib/GitPullRequest";
  import GithubLogo from "phosphor-svelte/lib/GithubLogo";
  import Key from "phosphor-svelte/lib/Key";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
  import PuzzlePiece from "phosphor-svelte/lib/PuzzlePiece";
  import QrCode from "phosphor-svelte/lib/QrCode";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import Snowflake from "phosphor-svelte/lib/Snowflake";
  import Subtitles from "phosphor-svelte/lib/Subtitles";
  import Sun from "phosphor-svelte/lib/Sun";
  import Television from "phosphor-svelte/lib/Television";
  import UsersThree from "phosphor-svelte/lib/UsersThree";
  import { siFlutter, siGraphql, siNixos, siRust, siSvelte, siTurso, siWebassembly } from "simple-icons";
  import Hero from "$components/Hero.svelte";
  import SimpleIcon from "$components/SimpleIcon.svelte";
  import Terminal from "$components/Terminal.svelte";
  import Decision from "$components/home/Decision.svelte";
  import Devices from "$components/home/Devices.svelte";
  import Friends from "$components/home/Friends.svelte";
  import Gallery from "$components/home/Gallery.svelte";
  import Parser from "$components/home/Parser.svelte";
  import Plugins from "$components/home/Plugins.svelte";
  import ReadyQueue from "$components/home/ReadyQueue.svelte";
  import SmartSearch from "$components/home/SmartSearch.svelte";
  import { reveal, tilt } from "$lib/actions";
  import { backdrop, links, stills } from "$lib/site";

  const more = [
    { icon: Broadcast, title: "NoFlex Connect", text: "Send a title to the TV and steer it from your phone. The TV picks the version it plays best. Google Cast and AirPlay from the browser are in testing." },
    { icon: UsersThree, title: "Who's watching?", text: "A shared TV keeps a profile for each person who signs in, and a PIN locks one on the server itself, not just on screen." },
    { icon: QrCode, title: "Sign in with a code", text: "No typing passwords with a remote. Scan the code on the TV and approve it from your phone." },
    { icon: Television, title: "Live TV and DVR", text: "Bring M3U and XMLTV sources for channels and a programme guide, and record what's on." },
    { icon: Sun, title: "HDR, handled", text: "HDR10, HDR10+, HLG and Dolby Vision, tone-mapped on the GPU for screens that can't show them." },
    { icon: Subtitles, title: "Subtitles and audio", text: "Text subtitles stay text, image ones are burned in only when needed, and picking a language only transcodes if it must." },
    { icon: FilmStrip, title: "Trickplay and chapters", text: "Scrub with thumbnails, jump by chapter, and skip intros the file's chapters mark." },
    { icon: DownloadSimple, title: "Offline copies", text: "Keep the original as one file, or let Smart Downloads pick for you within the space you give it." },
    { icon: ShieldCheck, title: "Parental controls", text: "Allowed ratings, library access, and bitrate limits for each person." },
    { icon: Key, title: "Your own sign-in", text: "OpenID Connect with Authentik, Keycloak and the rest, plus API keys for your scripts." },
    { icon: ChartLine, title: "A status page worth opening", text: "Live CPU, memory and network, who's streaming and why it transcodes, and what's been played." },
    { icon: MusicNotes, title: "Music and photos", text: "Artists, albums and tracks beside your films, and your photos too. More for both is on the way." },
  ];

  const apps = [
    { group: "Watch", items: [
      { name: "Web", detail: "Built into the server, phone width included", status: "ready" },
      { name: "Android", detail: "Phones, tablets and foldables; release APKs build today", status: "preview" },
      { name: "Linux desktop", detail: "Runs on Ubuntu 24.04 and Fedora 43", status: "preview" },
      { name: "iPhone, iPad and Mac", detail: "Same app, not yet run on Apple hardware", status: "dev" },
      { name: "Windows", detail: "Same app, not yet run on Windows", status: "dev" },
    ] },
    { group: "TV", items: [
      { name: "Any TV browser", detail: "Vizio, Hisense, Titan OS, Xbox; verified in Chrome", status: "preview" },
      { name: "Android TV, Google TV, Fire TV", detail: "Runs in the Android emulator; awaiting real TVs", status: "dev" },
      { name: "Samsung and LG", detail: "Tizen 3+ and webOS 3+ builds; awaiting real TVs", status: "dev" },
      { name: "Apple TV", detail: "Bundles; not yet run on Apple hardware", status: "dev" },
    ] },
    { group: "Server", items: [
      { name: "Linux (x86-64)", detail: "One bundle with our ffmpeg and the search model", status: "ready" },
      { name: "NixOS", detail: "A hardened module with backups", status: "ready" },
      { name: "macOS (Apple silicon)", detail: "Has a CI recipe; not yet run on a Mac", status: "dev" },
    ] },
  ];
  const statusLabel = { ready: "Works today", preview: "Preview", dev: "In development" };

  const stack = [
    { icon: siRust, name: "Rust" },
    { icon: siGraphql, name: "GraphQL" },
    { icon: siWebassembly, name: "WebAssembly" },
    { icon: siSvelte, name: "Svelte" },
    { icon: siFlutter, name: "Flutter" },
    { icon: siTurso, name: "Turso" },
    { icon: siNixos, name: "Nix" },
  ];

  const promises = ["No account with us.", "No relay in the middle.", "No telemetry. No ads."];
  let spoken = $state(0);
  function speak() {
    for (let i = 1; i <= promises.length; i++) setTimeout(() => (spoken = i), i * 380);
  }
</script>

<svelte:head>
  <title>NoFlex · Your media, your server</title>
  <meta name="description" content="NoFlex is a free, self-hosted media server for your movies, shows, music and live TV, with apps for the web, phones, desktops and TVs." />
</svelte:head>

<Hero />

<section class="section" id="tour">
  <div class="wrap">
    <header class="section-head center" use:reveal>
      <p class="eyebrow">One product, every screen</p>
      <h2>Pick up on any screen you own.</h2>
      <p>A browser, a phone, a foldable, a laptop, the TV across the room. NoFlex is one design on all of them, shaped to the screen and to whatever is in your hand.</p>
    </header>
    <Devices />
  </div>
</section>

<section class="section" id="features">
  <div class="glow" aria-hidden="true"></div>
  <div class="wrap">
    <header class="section-head" use:reveal>
      <p class="eyebrow">Ready before you ask</p>
      <h2>The next episode is already waiting.</h2>
      <p>NoFlex works out what each person is likely to watch next and prepares it for each of their devices while the server is quiet, then syncs it to their phone on Wi-Fi. It keeps under a CPU cap, steps aside the moment anyone presses play, and stays within the space you give it.</p>
    </header>
    <div use:reveal><ReadyQueue /></div>
    <header class="section-head sub" use:reveal>
      <h3>AV1 first, and it tells you why.</h3>
      <p>Encodes try AV1 first for any device that decodes it, on whichever GPU you have. When something does have to transcode, every app can show you why, in plain words.</p>
    </header>
    <div use:reveal><Decision /></div>
  </div>
</section>

<section class="section" id="friends">
  <div class="wrap">
    <header class="section-head" use:reveal>
      <p class="eyebrow">Better with friends</p>
      <h2>Your friends' libraries, right beside yours.</h2>
      <p>Link a friend's server and their films show up next to your own. Browse, search, stream and download, and what you've watched follows you between them. Then open a room and watch together, in sync.</p>
    </header>
    <div use:reveal><Friends /></div>
  </div>
</section>

<section class="section" id="search">
  <div class="wrap split">
    <header class="section-head" use:reveal>
      <p class="eyebrow">Smart, and private</p>
      <h2>Search for what it's about.</h2>
      <p>NoFlex reads every synopsis with a language model that runs on your own server, on its CPU, GPU or NPU. Search blends meaning with exact words, and recommendations come from what you've watched. None of it goes to a cloud.</p>
      <ul class="points">
        <li>Indexes while the server is idle, and waits while it's busy</li>
        <li>Recommendations that weigh what you watched lately</li>
        <li>Finds a title from half a memory, or a name from part of a word</li>
      </ul>
    </header>
    <div use:reveal><SmartSearch /></div>
  </div>
</section>

<section class="section" id="plugins">
  <div class="wrap">
    <header class="section-head" use:reveal>
      <p class="eyebrow">Extensible, safely</p>
      <h2>Every integration is a sandboxed plugin.</h2>
      <p>Metadata, sign-in, scrobbling, notifications and search are all WebAssembly plugins, ours included. Each one declares up front what it needs, reaches only the hosts it names, never touches the database, and has every call written to the audit log.</p>
    </header>
    <div use:reveal><Plugins /></div>
    <p class="aside" use:reveal><PuzzlePiece size={16} />Plugins can use any license. <a href={links.plugins}>Write your own<ArrowUpRight size={14} /></a></p>
  </div>
</section>

<section class="section" id="library">
  <div class="wrap split reverse">
    <header class="section-head" use:reveal>
      <p class="eyebrow">Bring your library as it is</p>
      <h2>No renaming weekend required.</h2>
      <p>Tidy Plex-style folders work, and so do release names, edition tags, and one show split across 1080p and 4K season folders. Local artwork, sidecar subtitles and multi-part films are picked up as they are.</p>
    </header>
    <div use:reveal><Parser /></div>
  </div>
</section>

<section class="section" id="gallery">
  <div class="wrap">
    <header class="section-head center" use:reveal>
      <p class="eyebrow">Everything answers</p>
      <h2>Made to feel instant.</h2>
      <p>Pages paint from cache, posters carry into their title pages, and every control responds the moment you touch it.</p>
    </header>
    <Gallery />
  </div>
</section>

<section class="section" id="more">
  <div class="wrap">
    <header class="section-head" use:reveal>
      <p class="eyebrow">And the rest</p>
      <h2>Everything else you'd want from a media server.</h2>
    </header>
    <ul class="more">
      {#each more as item, i}
        {@const Icon = item.icon}
        <li class="panel" use:reveal style:--i={i % 3}>
          <span class="icon"><Icon size={20} /></span>
          <h3>{item.title}</h3>
          <p>{item.text}</p>
        </li>
      {/each}
    </ul>
  </div>
</section>

<section class="section" id="apps">
  <div class="wrap">
    <header class="section-head" use:reveal>
      <p class="eyebrow">Apps</p>
      <h2>On the screens you already have.</h2>
      <p>One app for phones, tablets, foldables and desktops, one for TVs, and a web client built into the server. Nothing is in an app store yet. Here's what each one has run on so far.</p>
    </header>
    <div class="apps">
      {#each apps as group, g}
        <div class="panel" use:reveal style:--i={g}>
          <h3 class="eyebrow">{group.group}</h3>
          <ul>
            {#each group.items as item}
              <li>
                <span><strong>{item.name}</strong><small>{item.detail}</small></span>
                <span class="status {item.status}">{statusLabel[item.status]}</span>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </div>
  </div>
</section>

<section class="section promises" id="privacy" use:reveal={speak}>
  <div class="wrap">
    <p class="eyebrow">Self-hosted, end to end</p>
    <ul>
      {#each promises as line, i}
        <li class:on={spoken > i}>{line}</li>
      {/each}
    </ul>
    <p class="lede">By default, the only outside services NoFlex talks to are TMDB, TheTVDB and MusicBrainz, to name what's in your library. Everything else stays between your server and your screens.</p>
    <a class="btn ghost" href="/privacy">Read the privacy policy<ArrowRight class="arrow" size={15} /></a>
  </div>
</section>

<section class="section" id="open-source">
  <div class="wrap split">
    <header class="section-head" use:reveal>
      <p class="eyebrow">Open source</p>
      <h2>Free software, built in the open.</h2>
      <p>NoFlex is written from scratch in Rust and licensed under the AGPL. Read every line, run it anywhere, and help shape where it goes next.</p>
      <ul class="stack" aria-label="Built with">
        {#each stack as item}<li><SimpleIcon icon={item.icon} size={15} />{item.name}</li>{/each}
      </ul>
    </header>
    <div class="source" use:reveal>
      <Terminal title="Try it from source" lines={["git clone https://github.com/NoFlexMedia/noflex", "cd noflex && nix develop", "just demo up"]} note="Starts a throwaway server with Blender's open movies, at http://127.0.0.1:8096." />
      <div class="cards">
        <a class="card" href={links.repo} use:tilt={4}><GithubLogo size={20} weight="fill" /><strong>Source code</strong><small>NoFlexMedia/noflex</small></a>
        <a class="card" href={links.gettingStarted} use:tilt={4}><BookOpen size={20} /><strong>Documentation</strong><small>Build, configure, deploy</small></a>
        <a class="card" href={links.contributing} use:tilt={4}><GitPullRequest size={20} /><strong>Contributing</strong><small>How changes land</small></a>
        <a class="card" href={links.deployment} use:tilt={4}><Snowflake size={20} /><strong>Deployment</strong><small>NixOS, remote access, GPUs</small></a>
      </div>
    </div>
  </div>
</section>

<section class="finale">
  <div class="strip" aria-hidden="true">
    {#each [...stills, ...stills] as item}<img src={backdrop(item.slug)} alt="" width="1600" height="900" loading="lazy" decoding="async" />{/each}
  </div>
  <div class="wrap" use:reveal>
    <h2>Take it for a spin.</h2>
    <p>Browse a real library in the demo, or read the code that runs it.</p>
    <div class="ctas">
      <a class="btn primary lg" href="/demo">Try the demo<ArrowRight class="arrow" size={18} weight="bold" /></a>
      <a class="btn glass lg" href={links.repo}><GithubLogo size={19} weight="fill" />Star on GitHub</a>
    </div>
  </div>
</section>

<style>
  .section-head.sub { margin-top: clamp(64px, 10vh, 110px); }
  .section-head h3 { font-size: clamp(1.5rem, 2.6vw, 2.1rem); font-weight: 700; letter-spacing: -0.035em; }
  #features { overflow: clip; }
  .glow { position: absolute; z-index: -1; top: 10%; left: 50%; width: 70vw; height: 60vh; translate: -50% 0; background: radial-gradient(closest-side, rgb(212 255 90 / 0.07), transparent); pointer-events: none; }

  .split { display: grid; grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.1fr); gap: clamp(32px, 5vw, 80px); align-items: center; }
  .split > .section-head { margin-bottom: 0; }
  .split.reverse > :first-child { order: 2; }
  .points { display: grid; gap: 10px; margin-top: 8px; }
  .points li { display: flex; gap: 10px; color: var(--text-2); font-size: 0.95rem; }
  .points li::before { content: ""; flex: none; width: 6px; height: 6px; margin-top: 0.6em; border-radius: 50%; background: var(--accent); }

  .aside { display: flex; align-items: center; gap: 8px; margin-top: 18px; color: var(--text-3); font-size: 0.88rem; }
  .aside a { display: inline-flex; align-items: center; gap: 3px; color: var(--text); font-weight: 600; transition: color var(--t); }
  .aside a:hover { color: var(--accent); }

  .more { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr)); gap: 14px; }
  .more li { display: grid; align-content: start; gap: 8px; transition: background var(--t), border-color var(--t), transform var(--t-slow) var(--ease-spring); }
  .more li:hover { background: var(--surface-2); border-color: var(--line-strong); transform: translateY(-3px); }
  .more .icon { display: grid; place-items: center; width: 40px; height: 40px; margin-bottom: 6px; border-radius: 12px; background: var(--surface-2); color: var(--text-2); transition: background var(--t), color var(--t), transform var(--t-slow) var(--ease-spring); }
  .more li:hover .icon { background: var(--accent); color: var(--accent-ink); transform: rotate(-6deg) scale(1.06); }
  .more h3 { font-size: 1rem; }
  .more p { color: var(--text-2); font-size: 0.9rem; }

  .apps { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; align-items: start; }
  .apps h3 { margin-bottom: 8px; }
  .apps li { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 12px 0; border-top: 1px solid var(--line); }
  .apps li:first-child { border-top: 0; }
  .apps li > span:first-child { display: grid; gap: 2px; min-width: 0; }
  .apps strong { font-size: 0.92rem; font-weight: 600; }
  .apps small { color: var(--text-3); font-size: 0.8rem; }
  .status { flex: none; height: 22px; padding: 0 9px; border-radius: 99px; display: inline-flex; align-items: center; font-size: 0.72rem; font-weight: 650; background: var(--surface-2); color: var(--text-3); }
  .status.ready { background: var(--accent-soft); color: var(--accent); }
  .status.preview { background: rgb(165 220 255 / 0.12); color: var(--focus); }

  .promises { text-align: center; }
  .promises .wrap { display: grid; justify-items: center; gap: 26px; }
  .promises ul { display: grid; gap: 4px; }
  .promises li { font-size: clamp(2.2rem, 6.4vw, 5.2rem); font-weight: 760; letter-spacing: -0.05em; line-height: 1.05; color: var(--surface-3); transition: color 900ms var(--ease-out), transform 900ms var(--ease-out); transform: translateY(8px); }
  .promises li.on { color: var(--text); transform: none; }
  .promises li.on:last-child { color: var(--accent); }
  .promises .lede { max-width: 60ch; color: var(--text-2); font-size: 1.05rem; }

  .stack { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
  .stack li { display: inline-flex; align-items: center; gap: 7px; height: 32px; padding: 0 12px; border-radius: 999px; border: 1px solid var(--line); background: var(--surface); color: var(--text-2); font-size: 0.82rem; font-weight: 550; transition: color var(--t), border-color var(--t); }
  .stack li:hover { color: var(--text); border-color: var(--line-strong); }
  .source { display: grid; gap: 14px; }
  .cards { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .card { display: grid; gap: 2px; padding: 16px; border-radius: var(--r); border: 1px solid var(--line); background: var(--surface); transform: perspective(700px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg)); transition: transform 500ms var(--ease-out), background var(--t), border-color var(--t); }
  .card:global([data-tilting]) { transition-duration: 120ms, var(--t), var(--t); }
  .card:hover { background: var(--surface-2); border-color: var(--line-strong); }
  .card :global(svg) { margin-bottom: 10px; color: var(--text-2); }
  .card strong { font-size: 0.94rem; }
  .card small { color: var(--text-3); font-size: 0.8rem; }

  .finale { position: relative; isolation: isolate; overflow: hidden; padding: clamp(110px, 18vh, 200px) 0; text-align: center; }
  .finale .wrap { display: grid; justify-items: center; gap: 18px; }
  .finale h2 { font-size: clamp(2.6rem, 7vw, 5.6rem); font-weight: 760; letter-spacing: -0.05em; line-height: 0.95; }
  .finale p { color: var(--text-2); font-size: 1.1rem; }
  .ctas { display: flex; flex-wrap: wrap; justify-content: center; gap: 12px; margin-top: 10px; }
  .strip { position: absolute; z-index: -1; inset: 50% auto auto 0; display: flex; gap: 16px; width: max-content; translate: 0 -50%; rotate: -6deg; opacity: 0.32; filter: saturate(1.1); animation: slide 90s linear infinite; mask-image: linear-gradient(180deg, transparent, #000 30%, #000 70%, transparent); }
  .strip img { width: 380px; height: auto; aspect-ratio: 16 / 9; object-fit: cover; border-radius: var(--r); }
  .finale::after { content: ""; position: absolute; inset: 0; z-index: -1; background: radial-gradient(ellipse 50% 60% at 50% 50%, rgb(7 8 11 / 0.92), rgb(7 8 11 / 0.5) 70%, var(--bg)); }
  @keyframes slide { to { transform: translateX(-50%); } }

  @media (max-width: 899px) {
    .split { grid-template-columns: 1fr; }
    .split.reverse > :first-child { order: 0; }
    .apps { grid-template-columns: 1fr; }
  }
  @media (max-width: 520px) { .cards { grid-template-columns: 1fr; } }
</style>
