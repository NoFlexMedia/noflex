<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowUpRight from "phosphor-svelte/lib/ArrowUpRight";
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import Bug from "phosphor-svelte/lib/Bug";
  import ChatsCircle from "phosphor-svelte/lib/ChatsCircle";
  import ShieldWarning from "phosphor-svelte/lib/ShieldWarning";
  import { reveal, tilt } from "$lib/actions";
  import { links } from "$lib/site";

  const channels = [
    { icon: BookOpen, title: "Read the docs", text: "Installing, configuring, remote access and hardware transcoding.", href: links.gettingStarted, cta: "Getting started" },
    { icon: ChatsCircle, title: "Ask a question", text: "Setup help, ideas, and showing off your setup.", href: links.discussions, cta: "GitHub Discussions" },
    { icon: Bug, title: "Report a bug", text: "Something broken? Tell us what you did, what you expected, and what happened.", href: links.newIssue, cta: "Open an issue" },
    { icon: ShieldWarning, title: "Report a vulnerability", text: "Privately, through GitHub, so it isn't public before there's a fix.", href: links.advisories, cta: "Security advisories" },
  ];

  const faq = [
    ["Is NoFlex free?", "Yes. The server and the apps are free software under the AGPL. There's no subscription, no premium tier and no account with us."],
    ["Is it ready to hold my library?", "Not yet. NoFlex is pre-alpha: it works, but it changes quickly and may lose your watch history between versions. Try the demo, or run it alongside the server you use today."],
    ["Do I need an account with you?", "No. Accounts live on your own server. The apps never talk to us."],
    ["I forgot my password.", "NoFlex can't reset a forgotten password yet. If you're still signed in on a device, change it from your account there; otherwise ask whoever runs your server."],
    ["Can I watch away from home?", "Yes, through a VPN such as Tailscale or WireGuard, or a reverse proxy. NoFlex has no relay of its own; the deployment guide walks through each option."],
    ["Which devices can I watch on?", "Any modern browser today, with apps for Android, iPhone and iPad, Mac, Windows, Linux and TVs in progress. The apps section on the home page shows where each one stands."],
    ["Why is a video transcoding?", "Open Info in the player. Every app shows how a title is playing and, when it transcodes, exactly why: a codec the device can't decode, HDR on a screen that can't show it, a bitrate limit, and so on."],
  ];
</script>

<svelte:head>
  <title>Support · NoFlex</title>
  <meta name="description" content="Get help with NoFlex: documentation, questions, bug reports, and how to report a security issue." />
</svelte:head>

<div class="support wrap">
  <header>
    <p class="eyebrow">Support</p>
    <h1>How can we help?</h1>
    <p class="lede">NoFlex is built in the open, and so is its support: everything happens on GitHub. Start with the docs, ask in Discussions, or tell us what broke.</p>
  </header>

  <ul class="channels">
    {#each channels as channel, i}
      {@const Icon = channel.icon}
      <li style:--i={i}>
        <a href={channel.href} use:tilt={4}>
          <span class="icon"><Icon size={22} /></span>
          <strong>{channel.title}</strong>
          <span class="text">{channel.text}</span>
          <span class="cta">{channel.cta}<ArrowUpRight size={14} /></span>
        </a>
      </li>
    {/each}
  </ul>

  <section class="faq" use:reveal>
    <h2>Questions people ask</h2>
    <div class="items">
      {#each faq as [question, answer]}
        <details>
          <summary>{question}<span class="chev" aria-hidden="true"></span></summary>
          <p>{answer}</p>
        </details>
      {/each}
    </div>
  </section>

  <section class="security panel" id="security" use:reveal>
    <span class="icon"><ShieldWarning size={24} /></span>
    <div>
      <h2>Found a security issue?</h2>
      <p>Please don't open a public issue. Report it privately through <a href={links.advisories}>GitHub's security advisories</a>. We'll confirm we've received it within a few days, keep you updated while we work on a fix, and credit you when it's released, if you'd like.</p>
    </div>
  </section>
</div>

<style>
  .support { padding-top: calc(var(--topbar-h) + clamp(48px, 9vh, 96px)); padding-bottom: 120px; }
  header { display: grid; gap: 14px; max-width: 720px; margin-bottom: 48px; animation: fade-up 700ms var(--ease-out) both; }
  h1 { font-size: clamp(2.4rem, 5.6vw, 4.2rem); font-weight: 740; letter-spacing: -0.05em; line-height: 1; }
  .lede { color: var(--text-2); font-size: clamp(1.04rem, 1.4vw, 1.2rem); }
  .channels { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 250px), 1fr)); gap: 14px; }
  .channels li { animation: fade-up 700ms var(--ease-out) both; animation-delay: calc(120ms + var(--i) * 60ms); }
  .channels a { display: grid; gap: 8px; height: 100%; padding: 22px; border-radius: var(--r-l); border: 1px solid var(--line); background: var(--surface); transform: perspective(800px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg)); transition: transform 500ms var(--ease-out), background var(--t), border-color var(--t); }
  .channels a:global([data-tilting]) { transition-duration: 120ms, var(--t), var(--t); }
  .channels a:hover { background: var(--surface-2); border-color: var(--line-strong); }
  .icon { display: grid; place-items: center; width: 44px; height: 44px; margin-bottom: 8px; border-radius: 13px; background: var(--surface-2); color: var(--text-2); transition: background var(--t), color var(--t); }
  .channels a:hover .icon { background: var(--accent); color: var(--accent-ink); }
  .channels strong { font-size: 1.04rem; }
  .text { color: var(--text-2); font-size: 0.9rem; }
  .cta { display: inline-flex; align-items: center; gap: 4px; margin-top: 6px; color: var(--text); font-size: 0.86rem; font-weight: 600; word-break: break-all; }

  .faq { margin-top: clamp(72px, 12vh, 120px); display: grid; grid-template-columns: minmax(0, 0.7fr) minmax(0, 1.3fr); gap: 32px; }
  .faq h2 { font-size: clamp(1.8rem, 3.4vw, 2.6rem); font-weight: 700; letter-spacing: -0.04em; }
  .items { display: grid; }
  details { border-top: 1px solid var(--line); }
  details:last-child { border-bottom: 1px solid var(--line); }
  summary { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 18px 2px; font-weight: 600; cursor: pointer; list-style: none; transition: color var(--t); }
  summary::-webkit-details-marker { display: none; }
  summary:hover { color: #fff; }
  .chev { position: relative; flex: none; width: 14px; height: 14px; }
  .chev::before, .chev::after { content: ""; position: absolute; left: 0; top: 6px; width: 14px; height: 2px; border-radius: 2px; background: var(--text-3); transition: transform var(--t-slow) var(--ease-spring); }
  .chev::after { transform: rotate(90deg); }
  details[open] .chev::after { transform: rotate(0deg); }
  details p { padding: 0 2px 20px; color: var(--text-2); max-width: 64ch; animation: fade-up 360ms var(--ease-out) both; }

  .security { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 18px; margin-top: clamp(72px, 12vh, 120px); padding: 28px; scroll-margin-top: calc(var(--topbar-h) + 24px); }
  .security .icon { background: rgb(255 193 100 / 0.14); color: var(--warning); margin: 0; }
  .security h2 { font-size: 1.3rem; margin-bottom: 8px; }
  .security p { color: var(--text-2); max-width: 72ch; }
  .security a { color: var(--text); text-decoration: underline; text-decoration-color: rgb(212 255 90 / 0.5); text-underline-offset: 3px; }
  @media (max-width: 759px) { .faq { grid-template-columns: 1fr; } .security { grid-template-columns: 1fr; } }
</style>
