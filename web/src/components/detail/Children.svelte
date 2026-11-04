<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import ListBullets from "phosphor-svelte/lib/ListBullets";
  import Play from "phosphor-svelte/lib/Play";
  import SquaresFour from "phosphor-svelte/lib/SquaresFour";
  import { SeasonEpisodesStore, WorkChildrenStore } from "$houdini";
  import { runtime, timecode } from "$lib/format.js";
  import { failure, fetchQuery, nextPage } from "$lib/graphql.js";
  import { byIndex, player, playWork } from "$lib/playback.svelte.js";
  import Artwork from "$components/Artwork.svelte";
  import WorkCard from "$components/WorkCard.svelte";
  import Segmented from "$components/ui/Segmented.svelte";

  /**
   * Browse a container work: series → seasons → episodes, artist → albums,
   * album → tracks. `seasonStats`: { [seasonId]: { total, watched } };
   * `upNextId` marks the episode a Play would start.
   */
  let { work, art = null, seasonStats = {}, upNextId = null } = $props();

  // Episode layout is a per-browser preference; storage may be unavailable.
  const VIEW_KEY = "noflex.episodeView";
  let view = $state((() => { try { return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "tiles"; } catch { return "tiles"; } })());
  $effect(() => { try { localStorage.setItem(VIEW_KEY, view); } catch { /* private mode */ } });

  let libraryId = $derived(work.libraryId);
  let workId = $derived(work.id);
  const children = new WorkChildrenStore();
  const episodes = new SeasonEpisodesStore();
  $effect(() => { fetchQuery(children, { library: libraryId, parent: workId }); });

  // Rows are ordered by index here, not by the server, so every page is needed.
  function loadAll(result, store, state) {
    const { hasNextPage, endCursor } = result.pageInfo;
    if (!hasNextPage || !endCursor || state.requested === endCursor) return;
    state.requested = endCursor;
    nextPage(store).catch(() => { state.requested = ""; });
  }
  const childPages = { requested: "" };
  const episodePages = { requested: "" };
  $effect(() => loadAll($children, children, childPages));
  $effect(() => loadAll($episodes, episodes, episodePages));

  const nodes = (result, parent) => (result.variables?.parent === parent ? (result.data?.works.edges ?? []).map((edge) => edge.node).sort(byIndex) : []);
  let items = $derived(nodes($children, workId));
  let seasons = $derived(work.kind === "SERIES" ? items.filter((item) => item.kind === "SEASON") : []);
  let season = $state("");
  $effect(() => {
    if (!seasons.length) return;
    if (!seasons.some((item) => item.id === season)) {
      // Land on the first season with something left to watch.
      season = (seasons.find((item) => !item.userData?.played) ?? seasons[0]).id;
    }
  });
  $effect(() => { if (season) fetchQuery(episodes, { library: libraryId, parent: season }); });
  let rows = $derived(seasons.length ? nodes($episodes, season) : items.filter((item) => item.kind !== "SEASON"));
  const pending = (result, parent) => result.variables?.parent !== parent || (result.fetching && !result.data);
  let listLoading = $derived(seasons.length ? pending($episodes, season) : pending($children, workId));
  let listError = $derived(failure($children) || (seasons.length ? failure($episodes) : ""));
  let asGrid = $derived(work.kind === "ARTIST");
  let isEpisodes = $derived(!asGrid && work.kind !== "ALBUM");
  let current = $derived(seasons.find((item) => item.id === season));
  const seasonName = (item) => (item.indexNumber === 0 ? "Specials" : item.indexNumber != null ? `Season ${item.indexNumber}` : item.title);
  let isTracks = $derived(work.kind === "ALBUM");

  function progress(item) {
    const data = item.userData;
    if (!data || data.played || !data.positionMs || !item.runtimeMs) return 0;
    return Math.min(1, data.positionMs / item.runtimeMs);
  }
</script>

<section class="section children">
  <div class="section-head">
    <h2>{work.kind === "ARTIST" ? "Albums" : work.kind === "ALBUM" ? "Tracks" : seasons.length > 1 ? "Seasons" : "Episodes"}</h2>
    {#if isEpisodes && rows.length}
      <Segmented size="sm" bind:value={view} label="Episode layout" items={[{ value: "tiles", icon: SquaresFour, hint: "Tiles" }, { value: "list", icon: ListBullets, hint: "List" }]} />
    {/if}
  </div>

  {#if seasons.length > 1}
    <div class="seasons" role="radiogroup" aria-label="Season">
      {#each seasons as item, index (item.id)}
        {@const stats = seasonStats[item.id]}
        <button class="season" role="radio" aria-checked={item.id === season} class:on={item.id === season} style:--i={index} onclick={() => (season = item.id)}>
          <span class="season-art"><Artwork work={item} width={132} fallback={work} />{#if stats && stats.total && stats.watched === stats.total}<span class="done" title="Watched"><Check size={13} weight="bold" /></span>{/if}</span>
          <span class="season-name">{seasonName(item)}</span>
          <small class="tabular">{stats ? `${stats.total} ${stats.total === 1 ? "episode" : "episodes"}` : (item.year ?? "")}</small>
          {#if stats?.watched && stats.watched < stats.total}<span class="season-bar" aria-label="{stats.watched} of {stats.total} watched"><span style:width="{(stats.watched / stats.total) * 100}%"></span></span>{/if}
        </button>
      {/each}
    </div>
    {#if current}<h3 class="season-title">{seasonName(current)}{#if seasonStats[current.id]}<small class="tabular">{seasonStats[current.id].watched} of {seasonStats[current.id].total} watched</small>{/if}</h3>{/if}
  {/if}

  {#if listLoading && !rows.length}
    <div class={view === "tiles" && isEpisodes ? "tiles" : "episodes"} aria-hidden="true">{#each Array(view === "tiles" ? 8 : 4) as _}<div class="skeleton" style:aspect-ratio={view === "tiles" ? "16 / 12" : null} style:height={view === "tiles" ? null : "118px"}></div>{/each}</div>
  {:else if listError && !rows.length}
    <p class="empty-note">Couldn’t load this {work.kind.toLowerCase()}: {listError}</p>
  {:else if !rows.length}
    <p class="empty-note">Nothing has been scanned under this {work.kind.toLowerCase()} yet.</p>
  {:else if asGrid}
    <div class="poster-grid stagger">{#each rows as item, index (item.id)}<div style:--i={index}><WorkCard work={item} /></div>{/each}</div>
  {:else if isTracks}
    <ol class="tracks stagger">
      {#each rows as item, index (item.id)}
        <li style:--i={index}>
          <button class="track" onclick={() => playWork(item)} disabled={player.preparing === item.id}>
            <span class="num">{#if player.preparing === item.id}<span class="spinner sm"></span>{:else}<span class="n">{item.indexNumber ?? index + 1}</span><Play size={14} weight="fill" />{/if}</span>
            <span class="t">{item.title}</span>
            <span class="d tabular">{item.runtimeMs ? timecode(item.runtimeMs) : ""}</span>
          </button>
        </li>
      {/each}
    </ol>
  {:else if view === "tiles"}
    {#key season}
      <ol class="tiles stagger">
        {#each rows as item, index (item.id)}
          <li style:--i={index}>
            <article class="tile" class:watched={item.userData?.played} class:next={item.id === upNextId}>
              <button class="thumb" onclick={() => playWork(item)} aria-label="Play {item.title}" disabled={player.preparing === item.id}>
                <Artwork work={item} kind="thumb" width={300} fallback={art} />
                <span class="play-overlay">{#if player.preparing === item.id}<span class="spinner"></span>{:else}<span class="play-disc"><Play size={18} weight="fill" /></span>{/if}</span>
                <span class="ep-badge tabular">E{item.indexNumber ?? index + 1}</span>
                {#if item.id === upNextId}<span class="next-badge">Up next</span>{/if}
                {#if item.userData?.played}<span class="seen-badge" title="Watched"><Check size={12} weight="bold" /></span>{/if}
                {#if item.runtimeMs}<span class="dur tabular">{runtime(item.runtimeMs)}</span>{/if}
                {#if progress(item)}<span class="bar"><span style:width="{progress(item) * 100}%"></span></span>{/if}
              </button>
              <a href="/works/{item.id}" class="tile-title" title={item.overview ?? item.title}>{item.title}</a>
            </article>
          </li>
        {/each}
      </ol>
    {/key}
  {:else}
    <ol class="episodes stagger">
      {#each rows as item, index (item.id)}
        <li style:--i={index}>
          <article class="episode" class:watched={item.userData?.played}>
            <button class="thumb" onclick={() => playWork(item)} aria-label="Play {item.title}" disabled={player.preparing === item.id}>
              <Artwork work={item} kind="thumb" width={260} fallback={art} />
              <span class="play-overlay">{#if player.preparing === item.id}<span class="spinner"></span>{:else}<Play size={22} weight="fill" />{/if}</span>
              {#if progress(item)}<span class="bar"><span style:width="{progress(item) * 100}%"></span></span>{/if}
            </button>
            <div class="text">
              <a href="/works/{item.id}" class="title"><span class="ep tabular">{item.indexNumber ?? index + 1}</span>{item.title}</a>
              <p class="sub">{[runtime(item.runtimeMs), item.year].filter(Boolean).join(" · ")}{#if item.userData?.played}<span class="seen"><Check size={13} weight="bold" />Watched</span>{/if}</p>
              {#if item.overview}<p class="clamp-2 desc">{item.overview}</p>{/if}
            </div>
          </article>
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .children { margin-top: 56px; }
  .section-head { flex-wrap: wrap; align-items: center; }

  /* Season rail: poster tiles, the chosen one lifted and ringed. The rail
     scrolls, so it clips: its padding has to clear the 5px ring plus the
     3px hover lift. */
  .seasons { display: grid; grid-auto-flow: column; grid-auto-columns: clamp(104px, 11vw, 132px); gap: 14px; overflow-x: auto; padding: 10px 8px 14px; margin: -10px -8px 6px; scroll-snap-type: x proximity; scrollbar-width: none; }
  .season { display: grid; gap: 3px; text-align: left; scroll-snap-align: start; animation: fade-up var(--t-slow) var(--ease-out) both; animation-delay: calc(var(--i) * 40ms); }
  .season-art { position: relative; display: block; border-radius: var(--r); overflow: hidden; outline: 2px solid transparent; outline-offset: 3px; transition: transform var(--t) var(--ease-out), outline-color var(--t), box-shadow var(--t), filter var(--t); filter: saturate(0.75) brightness(0.8); }
  .season:hover .season-art { transform: translateY(-3px); filter: none; }
  .season.on .season-art { outline-color: var(--accent); filter: none; box-shadow: 0 14px 30px -14px rgb(212 255 90 / 0.4); }
  .season:active .season-art { transform: scale(0.97); }
  .season-name { margin-top: 7px; font-size: 0.88rem; font-weight: 650; color: var(--text-2); transition: color var(--t); }
  .season.on .season-name, .season:hover .season-name { color: var(--text); }
  .season small { color: var(--text-3); font-size: 0.76rem; }
  .season-bar { height: 3px; border-radius: 9px; background: var(--surface-3); overflow: hidden; margin-top: 3px; }
  .season-bar span { display: block; height: 100%; background: var(--accent); }
  .done { position: absolute; top: 7px; right: 7px; display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; background: var(--accent); color: var(--accent-ink); }
  .season-title { display: flex; align-items: baseline; gap: 12px; font-size: 1.04rem; margin: 10px 0 14px; }
  .season-title small { color: var(--text-3); font-size: 0.82rem; font-weight: 500; }

  /* Dense episode tiles. */
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(clamp(180px, 17vw, 236px), 1fr)); gap: 18px 14px; }
  .tile { display: grid; gap: 8px; }
  .tile .thumb { box-shadow: var(--shadow-1); transition: transform var(--t) var(--ease-out), box-shadow var(--t); }
  .tile .thumb:hover { transform: translateY(-3px); box-shadow: var(--shadow-2); }
  .tile .thumb:active { transform: scale(0.98); }
  .tile.next .thumb { outline: 2px solid var(--accent); outline-offset: 2px; }
  .play-disc { display: grid; place-items: center; width: 44px; height: 44px; border-radius: 50%; background: rgb(255 255 255 / 0.92); color: #0b0c0f; box-shadow: var(--shadow-2); }
  .play-disc :global(svg) { margin-left: 2px; }
  .ep-badge, .dur, .next-badge { position: absolute; top: 7px; height: 21px; padding: 0 7px; display: inline-flex; align-items: center; border-radius: 6px; font-size: 0.72rem; font-weight: 700; background: rgb(7 8 11 / 0.72); backdrop-filter: blur(6px); color: var(--text); }
  .ep-badge { left: 7px; }
  .next-badge { left: auto; right: 7px; background: var(--accent); color: var(--accent-ink); }
  .dur { top: auto; bottom: 8px; right: 7px; font-weight: 600; color: var(--text-2); transition: opacity var(--t); }
  .tile .thumb:hover .dur { opacity: 0; }
  .seen-badge { position: absolute; top: 7px; right: 7px; display: grid; place-items: center; width: 21px; height: 21px; border-radius: 50%; background: var(--accent); color: var(--accent-ink); }
  .tile.next .seen-badge { display: none; }
  .tile .bar { bottom: 0; left: 0; right: 0; border-radius: 0; height: 3px; }
  .tile-title { font-size: 0.88rem; font-weight: 600; line-height: 1.3; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); transition: color var(--t-fast); }
  .tile-title:hover { color: var(--text); }
  .tile.watched .tile-title { color: var(--text-3); }
  .tile.watched .thumb :global(img) { filter: saturate(0.6) brightness(0.8); }
  .episodes { display: grid; gap: 10px; }
  .episode { display: grid; grid-template-columns: clamp(150px, 18vw, 240px) minmax(0, 1fr); gap: 20px; align-items: center; padding: 10px; border-radius: var(--r-l); transition: background var(--t) var(--ease-out); }
  .episode:hover { background: var(--surface); }
  .thumb { position: relative; display: block; border-radius: var(--r); overflow: hidden; }
  .play-overlay { position: absolute; inset: 0; display: grid; place-items: center; background: rgb(0 0 0 / 0.35); color: #fff; opacity: 0; transition: opacity var(--t) var(--ease-out); }
  .play-overlay :global(svg) { transform: scale(0.8); transition: transform 360ms var(--ease-spring); }
  .thumb:hover .play-overlay, .thumb:focus-visible .play-overlay { opacity: 1; }
  .thumb:hover .play-overlay :global(svg) { transform: none; }
  .bar { position: absolute; left: 8px; right: 8px; bottom: 7px; height: 3px; border-radius: 9px; background: rgb(255 255 255 / 0.25); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); }
  .text { display: grid; gap: 4px; min-width: 0; }
  .title { font-weight: 650; font-size: 1rem; display: flex; gap: 10px; align-items: baseline; }
  .title:hover { text-decoration: underline; text-underline-offset: 3px; text-decoration-color: var(--text-3); }
  .ep { color: var(--text-3); font-weight: 600; min-width: 1.4em; }
  .sub { color: var(--text-3); font-size: 0.84rem; display: flex; gap: 12px; align-items: center; }
  .seen { display: inline-flex; align-items: center; gap: 4px; color: var(--accent); font-weight: 600; }
  .desc { color: var(--text-2); font-size: 0.9rem; }
  .watched .thumb :global(img) { filter: saturate(0.6) brightness(0.8); }
  .tracks { display: grid; max-width: 880px; }
  .track { display: grid; grid-template-columns: 40px 1fr auto; align-items: center; gap: 14px; width: 100%; padding: 10px 14px; border-radius: 12px; text-align: left; transition: background var(--t); }
  .track:hover { background: var(--surface-2); }
  .num { display: grid; place-items: center; color: var(--text-3); font-variant-numeric: tabular-nums; }
  .num :global(svg) { display: none; color: var(--text); }
  .track:hover .n { display: none; }
  .track:hover .num :global(svg) { display: block; }
  .t { font-weight: 550; }
  .d { color: var(--text-3); font-size: 0.86rem; }
  @media (max-width: 600px) {
    .episode { grid-template-columns: 1fr; }
    .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px 10px; }
  }
</style>
