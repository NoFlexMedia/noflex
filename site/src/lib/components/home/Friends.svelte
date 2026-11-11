<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import DeviceMobile from "phosphor-svelte/lib/DeviceMobile";
  import HardDrives from "phosphor-svelte/lib/HardDrives";
  import Pause from "phosphor-svelte/lib/Pause";
  import Play from "phosphor-svelte/lib/Play";
  import { prefersReducedMotion, reveal, tilt } from "$lib/actions";
  import { poster } from "$lib/site";

  const servers = [
    { name: "Home", owner: "Yours", y: 18 },
    { name: "Sam's place", owner: "Linked", y: 50 },
    { name: "Priya's NAS", owner: "Linked", y: 82 },
  ];
  const rail = [
    { slug: "sintel", server: "Home" },
    { slug: "charge", server: "Sam's place" },
    { slug: "sprite-fright", server: "Home" },
    { slug: "elephants-dream", server: "Priya's NAS" },
    { slug: "big-buck-bunny", server: "Sam's place" },
  ];
  const people = [
    { initials: "AL", hue: "linear-gradient(135deg, var(--accent), #8de2c5)" },
    { initials: "SM", hue: "linear-gradient(135deg, #a5dcff, #b9a5ff)" },
    { initials: "PK", hue: "linear-gradient(135deg, #ffc164, #ff8f8f)" },
  ];

  let live = $state(false);
  let playing = $state(true);
  let position = $state(0.34);
  let timer = 0;
  function start() {
    live = true;
    if (prefersReducedMotion()) return;
    let t = 0;
    timer = setInterval(() => {
      t++;
      if (t % 60 === 40) playing = false;
      if (t % 60 === 52) playing = true;
      if (playing) position = (position + 0.0025) % 1;
    }, 100);
  }
  $effect(() => () => clearInterval(timer));
</script>

<div class="grid" use:reveal={start}>
  <div class="panel links" class:live>
    <div class="diagram">
      <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        {#each servers as server}
          <path d="M 18 50 C 45 50, 50 {server.y}, 78 {server.y}" vector-effect="non-scaling-stroke" />
        {/each}
      </svg>
      <div class="client"><DeviceMobile size={22} /><span>Your app</span></div>
      {#each servers as server, i}
        <div class="server" style:top="{server.y}%" style:--i={i}>
          <HardDrives size={18} />
          <span><strong>{server.name}</strong><small>{server.owner}</small></span>
        </div>
      {/each}
    </div>
    <p class="note"><span class="dot on"></span>Your app talks to each server directly. The servers never talk to each other, and nothing passes through us.</p>
  </div>

  <div class="stack">
    <div class="panel merged">
      <div class="row-head"><strong>Recently added</strong><span class="faint">from 3 servers</span></div>
      <ul>
        {#each rail as item, i}
          <li style:--i={i}>
            <div class="poster" use:tilt><img src={poster(item.slug)} alt="" width="480" height="720" loading="lazy" decoding="async" /></div>
            <span class="tag" class:home={item.server === "Home"}>{item.server}</span>
          </li>
        {/each}
      </ul>
      <p class="sync"><ArrowsClockwise size={14} />Finish it on Sam's, and it's marked watched on yours.</p>
    </div>

    <div class="panel room">
      <div class="row-head"><strong>Watch together</strong><span class="faint">Sintel · room of 3</span></div>
      <div class="people">
        {#each people as person, i}
          <span class="avatar" style:background={person.hue} style:--i={i}>{person.initials}</span>
        {/each}
        <span class="toast" class:show={!playing}><Pause size={12} weight="fill" />Sam paused</span>
      </div>
      <div class="player">
        <span class="icon">{#if playing}<Play size={14} weight="fill" />{:else}<Pause size={14} weight="fill" />{/if}</span>
        <span class="track"><span class="fill" style:transform="scaleX({position})"></span><span class="head" style:left="{position * 100}%"></span></span>
        <span class="time">{Math.floor(position * 14)}:{String(Math.floor((position * 14 * 60) % 60)).padStart(2, "0")}</span>
      </div>
    </div>
  </div>
</div>

<style>
  .grid { display: grid; grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr); gap: 16px; }
  .links { display: grid; gap: 18px; align-content: space-between; }
  .diagram { position: relative; height: 320px; }
  svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
  path { fill: none; stroke: var(--line-strong); stroke-width: 1.5; stroke-dasharray: 3 6; }
  .live path { stroke: rgb(212 255 90 / 0.55); animation: flow 1.4s linear infinite; }
  @keyframes flow { to { stroke-dashoffset: -18; } }
  .client { position: absolute; left: 0; top: 50%; translate: 0 -50%; display: grid; justify-items: center; gap: 8px; width: 92px; padding: 16px 8px; border-radius: var(--r-l); background: var(--bg-raised); border: 1px solid var(--line-strong); font-size: 0.78rem; font-weight: 600; color: var(--text-2); box-shadow: var(--shadow-2); }
  .client :global(svg) { position: static; width: 22px; height: 22px; color: var(--accent); }
  .server { position: absolute; right: 0; translate: 0 -50%; display: flex; align-items: center; gap: 10px; width: min(48%, 200px); padding: 12px 14px; border-radius: var(--r); background: var(--bg-raised); border: 1px solid var(--line); box-shadow: var(--shadow-2); color: var(--text-3); transition: border-color var(--t), transform var(--t) var(--ease-spring); }
  .server:hover { border-color: var(--line-strong); transform: translateX(-3px); }
  .server :global(svg) { position: static; width: 18px; height: 18px; flex: none; }
  .server span { display: grid; min-width: 0; }
  .server strong { color: var(--text); font-size: 0.86rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .server small { font-size: 0.74rem; }
  .live .server { animation: fade-up 700ms var(--ease-out) both; animation-delay: calc(200ms + var(--i) * 120ms); }
  .note { display: flex; align-items: baseline; gap: 10px; color: var(--text-2); font-size: 0.9rem; }
  .note .dot { translate: 0 -1px; }

  .stack { display: grid; gap: 16px; }
  .row-head { display: flex; align-items: baseline; justify-content: space-between; gap: 10px; margin-bottom: 14px; font-size: 0.9rem; }
  .row-head .faint { font-size: 0.8rem; }
  .merged ul { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 10px; }
  .merged li { display: grid; gap: 7px; justify-items: start; }
  .poster { width: 100%; border-radius: 9px; transform: perspective(600px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg)); transition: transform 500ms var(--ease-out); box-shadow: 0 10px 22px -12px rgb(0 0 0 / 0.9); }
  .poster:global([data-tilting]) { transition-duration: 120ms; }
  .poster img { width: 100%; height: auto; aspect-ratio: 2 / 3; object-fit: cover; border-radius: inherit; }
  .tag { font-size: 0.68rem; font-weight: 600; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 100%; }
  .tag.home { color: var(--accent); }
  .sync { display: flex; align-items: center; gap: 8px; margin-top: 14px; color: var(--text-2); font-size: 0.84rem; }

  .people { position: relative; display: flex; align-items: center; margin-bottom: 16px; }
  .avatar { display: grid; place-items: center; width: 34px; height: 34px; margin-right: -8px; border-radius: 50%; border: 2px solid var(--bg); font-size: 0.72rem; font-weight: 700; color: var(--accent-ink); }
  .toast { margin-left: auto; display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border-radius: 99px; background: #f4f5f1; color: #0b0c0f; font-size: 0.76rem; font-weight: 650; opacity: 0; transform: translateY(6px) scale(0.94); transition: opacity var(--t) var(--ease-out), transform var(--t-slow) var(--ease-spring); }
  .toast.show { opacity: 1; transform: none; }
  .player { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 12px; }
  .icon { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 50%; background: rgb(255 255 255 / 0.92); color: #0b0c0f; }
  .track { position: relative; height: 4px; border-radius: 99px; background: rgb(255 255 255 / 0.14); }
  .fill { position: absolute; inset: 0; border-radius: inherit; background: var(--accent); transform-origin: left; transition: transform 100ms linear; }
  .head { position: absolute; top: 50%; width: 12px; height: 12px; margin: -6px 0 0 -6px; border-radius: 50%; background: #fff; box-shadow: var(--shadow-1); transition: left 100ms linear; }
  .time { font-size: 0.78rem; color: var(--text-3); font-variant-numeric: tabular-nums; }
  @media (max-width: 899px) { .grid { grid-template-columns: 1fr; } .diagram { height: 260px; } }
</style>
