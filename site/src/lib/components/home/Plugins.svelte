<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import Prohibit from "phosphor-svelte/lib/Prohibit";
  import PuzzlePiece from "phosphor-svelte/lib/PuzzlePiece";
  import { prefersReducedMotion, reveal } from "$lib/actions";

  // The first-party plugins' real manifests (backend/plugins/*/Cargo.toml).
  const plugins = [
    { id: "tmdb", name: "The Movie Database", hooks: ["metadata", "image"], caps: ["http.egress", "secrets.read"], hosts: ["api.themoviedb.org", "image.tmdb.org"], calls: ["GET api.themoviedb.org/3/search/movie", "GET image.tmdb.org/t/p/original/…", "GET api.themoviedb.org/3/movie/45745"] },
    { id: "tvdb", name: "TheTVDB", hooks: ["metadata", "image"], caps: ["http.egress", "secrets.read"], hosts: ["api4.thetvdb.com", "artworks.thetvdb.com"], calls: ["POST api4.thetvdb.com/v4/login", "GET api4.thetvdb.com/v4/search", "GET artworks.thetvdb.com/banners/…"] },
    { id: "musicbrainz", name: "MusicBrainz", hooks: ["metadata"], caps: ["http.egress"], hosts: ["musicbrainz.org"], calls: ["GET musicbrainz.org/ws/2/release", "GET musicbrainz.org/ws/2/artist"] },
    { id: "trakt", name: "Trakt", hooks: ["scrobbler", "user-link"], caps: ["http.egress", "secrets.read", "user-secrets.read", "user-secrets.write", "background"], hosts: ["api.trakt.tv"], calls: ["POST api.trakt.tv/scrobble/start", "POST api.trakt.tv/scrobble/stop", "GET api.trakt.tv/sync/history"] },
    { id: "embeddings", name: "Local embeddings", hooks: ["embedder"], caps: ["assets.read", "inference.run"], hosts: [], calls: [] },
  ];

  let selected = $state(0);
  let plugin = $derived(plugins[selected]);
  let log = $state([]);
  let timer = 0;
  let n = 0;

  function line() {
    const p = plugins[selected];
    n++;
    const time = new Date(Date.UTC(2026, 10, 11, 20, 4, 30 + n)).toISOString().slice(11, 19);
    // Now and then, show the host turning something away.
    if (p.calls.length && n % 5 === 0) return { id: n, time, plugin: p.id, text: "GET 192.168.1.20/admin", ok: false, why: "private address" };
    if (!p.calls.length) return { id: n, time, plugin: p.id, text: "inference.run · 16 synopses", ok: true };
    return { id: n, time, plugin: p.id, text: p.calls[n % p.calls.length], ok: true };
  }
  function start() {
    log = [line(), line(), line()];
    if (prefersReducedMotion()) return;
    timer = setInterval(() => (log = [...log, line()].slice(-6)), 1500);
  }
  function choose(i) { selected = i; log = [line(), line()]; }
  $effect(() => () => clearInterval(timer));
</script>

<div class="grid" use:reveal={start}>
  <ul class="list" role="listbox" aria-label="First-party plugins">
    {#each plugins as item, i}
      <li role="option" aria-selected={i === selected}>
        <button onclick={() => choose(i)}>
          <span class="icon"><PuzzlePiece size={17} weight={i === selected ? "fill" : "regular"} /></span>
          <span class="name"><strong>{item.name}</strong><small>{item.hooks.join(" · ")}</small></span>
          <span class="wasm">.wasm</span>
        </button>
      </li>
    {/each}
  </ul>

  <div class="panel detail">
    {#key selected}
      <div class="manifest">
        <p class="eyebrow">Manifest · {plugin.id}</p>
        <dl>
          <dt>Capabilities</dt>
          <dd>{#each plugin.caps as cap}<span class="chip mono">{cap}</span>{:else}<span class="faint">None. It can't reach anything.</span>{/each}</dd>
          <dt>Network</dt>
          <dd>{#each plugin.hosts as host}<span class="chip outline mono">{host}</span>{:else}<span class="faint">No network access</span>{/each}</dd>
        </dl>
      </div>
    {/key}
    <div class="audit">
      <p class="eyebrow">Audit log</p>
      <ol aria-live="off">
        {#each log as entry (entry.id)}
          <li class:denied={!entry.ok}>
            <span class="time">{entry.time}</span>
            <span class="who">{entry.plugin}</span>
            <span class="what">{entry.text}</span>
            <span class="verdict">{#if entry.ok}<Check size={12} weight="bold" />{:else}<Prohibit size={12} weight="bold" />{entry.why}{/if}</span>
          </li>
        {/each}
      </ol>
    </div>
  </div>
</div>

<style>
  .grid { display: grid; grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.3fr); gap: 16px; }
  .list { display: grid; gap: 6px; align-content: start; }
  .list button { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 12px; width: 100%; padding: 12px 14px; border-radius: var(--r); border: 1px solid var(--line); background: var(--surface); text-align: left; transition: background var(--t), border-color var(--t), transform var(--t-fast) var(--ease-out); }
  .list button:hover { background: var(--surface-2); }
  .list button:active { transform: scale(0.985); }
  [aria-selected="true"] button { border-color: rgb(212 255 90 / 0.4); background: var(--accent-soft); }
  .icon { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 9px; background: var(--surface-2); color: var(--text-2); }
  [aria-selected="true"] .icon { background: var(--accent); color: var(--accent-ink); }
  .name { display: grid; min-width: 0; }
  .name strong { font-size: 0.9rem; font-weight: 600; }
  .name small { color: var(--text-3); font-size: 0.76rem; }
  .wasm { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 0.72rem; color: var(--text-3); }
  .detail { display: grid; gap: 22px; align-content: start; background: rgb(14 16 21 / 0.9); }
  .manifest { animation: fade-up 420ms var(--ease-out) both; }
  dl { display: grid; grid-template-columns: 110px minmax(0, 1fr); gap: 12px 16px; margin-top: 14px; align-items: start; }
  dt { color: var(--text-3); font-size: 0.82rem; padding-top: 3px; }
  dd { display: flex; flex-wrap: wrap; gap: 6px; font-size: 0.86rem; }
  .chip.mono { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 0.74rem; }
  .audit { border-top: 1px solid var(--line); padding-top: 18px; }
  .audit ol { display: grid; gap: 2px; margin-top: 10px; min-height: 186px; align-content: end; font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 0.76rem; }
  .audit li { display: grid; grid-template-columns: 64px 92px minmax(0, 1fr) auto; gap: 10px; align-items: center; padding: 5px 8px; border-radius: 6px; color: var(--text-2); animation: log-in 420ms var(--ease-out) both; }
  @keyframes log-in { from { opacity: 0; transform: translateY(8px); } }
  .time { color: var(--text-3); }
  .who { color: var(--focus); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .what { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .verdict { display: inline-flex; align-items: center; gap: 5px; color: var(--success); white-space: nowrap; }
  .denied { background: rgb(255 107 107 / 0.08); }
  .denied .verdict { color: var(--danger); }
  @media (max-width: 899px) { .grid { grid-template-columns: 1fr; } .list { grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); } }
  @media (max-width: 520px) { .audit li { grid-template-columns: 58px minmax(0, 1fr) auto; } .who { display: none; } dl { grid-template-columns: 1fr; gap: 6px; } }
</style>
