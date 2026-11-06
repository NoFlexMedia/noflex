<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Avatar from "$components/Avatar.svelte";
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import Clock from "phosphor-svelte/lib/Clock";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import Folder from "phosphor-svelte/lib/Folder";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
  import Play from "phosphor-svelte/lib/Play";
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import Television from "phosphor-svelte/lib/Television";
  import TrendUp from "phosphor-svelte/lib/TrendUp";
  import Users from "phosphor-svelte/lib/Users";
  import { untrack } from "svelte";
  import { AdminPlayChartStore, AdminTopPlayedStore, AdminTopUsersStore } from "$houdini";
  import { titleCase } from "$lib/format.js";
  import { failure, fetchQuery, refetch } from "$lib/graphql.js";
  import Artwork from "$components/Artwork.svelte";
  import State from "$components/State.svelte";
  import Segmented from "$components/ui/Segmented.svelte";
  import Select from "$components/ui/Select.svelte";
  import { ago } from "$components/account/relative.js";
  import BarChart from "./stats/BarChart.svelte";
  import SessionsSheet from "./stats/SessionsSheet.svelte";
  import { RANGES, compact, hours, utcOffset } from "./stats/fmt.js";

  let { admin } = $props();

  const chart = new AdminPlayChartStore();
  const topUsers = new AdminTopUsersStore();
  const topPlayed = new AdminTopPlayedStore();

  let range = $state("LAST_30_DAYS");
  let library = $state("");
  let user = $state("");
  let metric = $state("plays");
  let sheet = $state(null);

  const offset = utcOffset();
  $effect(() => {
    const vars = { range, offset };
    untrack(() => { fetchQuery(topUsers, vars); fetchQuery(topPlayed, vars); });
  });
  $effect(() => {
    const vars = { range, offset, library: library || null, user: user || null };
    untrack(() => fetchQuery(chart, vars));
  });

  let libraries = $derived($admin.data?.libraries ?? []);
  let users = $derived($admin.data?.users ?? []);
  let points = $derived($chart.data?.playHistoryChart.points ?? []);
  let bucket = $derived($chart.data?.playHistoryChart.bucket ?? "DAY");
  let leaders = $derived($topUsers.data?.topUsers ?? []);
  let played = $derived($topPlayed.data?.topPlayed ?? []);
  let rangeInfo = $derived(RANGES.find((item) => item.value === range));
  let totals = $derived({
    plays: leaders.reduce((sum, row) => sum + row.plays, 0),
    watched: leaders.reduce((sum, row) => sum + row.watchedMs, 0),
    users: leaders.length,
  });
  let topWatched = $derived(Math.max(1, ...leaders.map((row) => row.watchedMs)));
  let filtered = $derived(Boolean(library || user));

  const libraryIcon = { MOVIES: FilmStrip, SHOWS: Television, MUSIC: MusicNotes, AUDIOBOOKS: BookOpen, PHOTOS: ImageIcon, MIXED: Folder };
  const sub = (item) => `${item.plays} ${item.plays === 1 ? "play" : "plays"} · ${item.users} ${item.users === 1 ? "viewer" : "viewers"}`;

  function openSessions(row, entry = null) {
    sheet = { user: row.user, library: entry?.library ?? null, range };
  }
</script>

<div class="diag activity">
  <div class="filters">
    <Segmented bind:value={range} items={RANGES.map(({ value, label, hint }) => ({ value, label, hint }))} label="Time range" />
    <span class="spacer"></span>
    <span class="meta">Times shown in your timezone</span>
  </div>

  <div class="tiles small stagger">
    <div class="tile small" style:--i={0}><span class="tile-label"><Play size={13} weight="fill" />Plays</span><span class="tile-value">{compact(totals.plays)}</span><span class="tile-sub">over {rangeInfo?.long}</span></div>
    <div class="tile small" style:--i={1}><span class="tile-label"><Clock size={13} />Watched</span><span class="tile-value">{hours(totals.watched)}</span><span class="tile-sub">wall-clock playing time</span></div>
    <div class="tile small" style:--i={2}><span class="tile-label"><Users size={13} />Viewers</span><span class="tile-value">{totals.users}</span><span class="tile-sub">of {users.length} {users.length === 1 ? "account" : "accounts"}</span></div>
    <div class="tile small" style:--i={3}><span class="tile-label"><Sparkle size={13} />Busiest library</span><span class="tile-value">{played[0]?.library?.name ?? "—"}</span><span class="tile-sub">{played[0] ? `${played[0].plays} plays` : "no plays yet"}</span></div>
  </div>

  <section class="panel dense">
    <header class="panel-head chart-head">
      <div><h3>Play history</h3><p>{metric === "plays" ? "Plays" : "Hours watched"} per {bucket.toLowerCase()}{filtered ? " · filtered" : ""}</p></div>
      <span class="spacer"></span>
      <div class="chart-filters">
        <div class="pick"><Select pill bind:value={library} label="Library" items={[{ value: "", label: "All libraries" }, ...libraries.map((item) => ({ value: item.id, label: item.name }))]} /></div>
        <div class="pick"><Select pill bind:value={user} label="User" items={[{ value: "", label: "All users" }, ...users.map((item) => ({ value: item.id, label: item.name }))]} /></div>
        <Segmented size="sm" bind:value={metric} label="Measure" items={[{ value: "plays", label: "Plays" }, { value: "hours", label: "Hours" }]} />
      </div>
    </header>
    {#if failure($chart) && !points.length}
      <State compact kind="error" title="Couldn’t draw the chart" message={failure($chart)} action={() => refetch(chart)} />
    {:else if !points.length}
      <div class="skeleton chart-skel" aria-busy="true"></div>
    {:else}
      <div class="chart-wrap" class:dim={$chart.fetching}>
        <BarChart {points} {bucket} {metric} label="Play history over {rangeInfo?.long}" />
        {#if !$chart.data?.playHistoryChart.totalPlays}<p class="overlay-note">Nothing played {filtered ? "matching these filters " : ""}over {rangeInfo?.long}.</p>{/if}
      </div>
    {/if}
  </section>

  <div class="diag-cols">
    <section class="panel dense">
      <header class="panel-head"><span class="panel-icon"><Users size={18} /></span><div><h3>Top users</h3><p>Ranked by time watched. Select a library to see that user’s sessions.</p></div></header>
      {#if failure($topUsers) && !leaders.length}
        <State compact kind="error" title="Couldn’t rank users" message={failure($topUsers)} action={() => refetch(topUsers)} />
      {:else if $topUsers.fetching && !$topUsers.data}
        {#each Array(3) as _}<div class="skeleton user-skel"></div>{/each}
      {:else if !leaders.length}
        <State compact icon={Users} title="No viewers yet" message="Users appear here once they play something over {rangeInfo?.long}." />
      {:else}
        <ol class="leaders stagger">
          {#each leaders as row, index (row.user?.id ?? index)}
            <li style:--i={index}>
              <div class="leader-head">
                <span class="rank tabular">{index + 1}</span>
                <Avatar user={row.user} />
                <div class="who">
                  <button class="name" onclick={() => openSessions(row)}>{row.user?.name ?? "Deleted user"}</button>
                  <small>last played {ago(row.lastPlayedAt)}</small>
                </div>
                <div class="totals tabular"><strong>{hours(row.watchedMs)}</strong><small>{row.plays} {row.plays === 1 ? "play" : "plays"}</small></div>
              </div>
              <div class="meter" aria-hidden="true"><span style:width="{(row.watchedMs / topWatched) * 100}%"></span></div>
              <ul class="libs">
                {#each row.libraries as entry (entry.library?.id)}
                  {@const Glyph = libraryIcon[entry.library?.kind] ?? Folder}
                  <li>
                    <button class="lib" onclick={() => openSessions(row, entry)} aria-label="{row.user?.name}: {hours(entry.watchedMs)} in {entry.library?.name}, {entry.plays} plays. Show sessions">
                      <Glyph size={14} />
                      <span class="lib-name">{entry.library?.name ?? "Removed library"}</span>
                      <span class="lib-bar" aria-hidden="true"><span style:width="{(entry.watchedMs / Math.max(1, row.watchedMs)) * 100}%"></span></span>
                      <span class="lib-num tabular">{hours(entry.watchedMs)} · {entry.plays}</span>
                      <CaretRight size={14} />
                    </button>
                  </li>
                {/each}
              </ul>
            </li>
          {/each}
        </ol>
      {/if}
    </section>

    <section class="panel dense">
      <header class="panel-head"><span class="panel-icon"><TrendUp size={18} /></span><div><h3>Most played</h3><p>Per library. Episodes count toward their series, tracks toward their artist.</p></div></header>
      {#if failure($topPlayed) && !played.length}
        <State compact kind="error" title="Couldn’t rank titles" message={failure($topPlayed)} action={() => refetch(topPlayed)} />
      {:else if $topPlayed.fetching && !$topPlayed.data}
        {#each Array(2) as _}<div class="skeleton user-skel"></div>{/each}
      {:else if !played.length}
        <State compact icon={FilmStrip} title="Nothing played yet" message="The most played titles in each library show up here." />
      {:else}
        <div class="libraries">
          {#each played as group (group.library?.id)}
            {@const Glyph = libraryIcon[group.library?.kind] ?? Folder}
            <div class="group">
              <div class="sub-head"><Glyph size={15} /><h3>{group.library?.name ?? "Removed library"}</h3><small>{titleCase(group.library?.kind ?? "")} · {group.plays} plays</small></div>
              <ol class="top-row stagger">
                {#each group.items as item, index (item.work?.id ?? index)}
                  <li style:--i={index}>
                    <a class="top-item" href={item.work ? `/works/${item.work.id}` : undefined} title="{item.work?.title}: {sub(item)}">
                      <span class="poster"><Artwork work={item.work ?? { title: "?" }} width={96} /><span class="badge-rank tabular">{index + 1}</span></span>
                      <strong class="clamp-2">{item.work?.title ?? "Removed title"}</strong>
                      <small class="tabular">{sub(item)}</small>
                    </a>
                  </li>
                {/each}
              </ol>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  </div>
</div>

<SessionsSheet bind:scope={sheet} />

<style>
  .filters { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .meta { color: var(--text-3); font-size: 0.8rem; }
  .chart-head { flex-wrap: wrap; }
  .chart-filters { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; max-width: 100%; }
  .pick { min-width: 150px; }
  .chart-wrap { position: relative; padding-top: 34px; transition: opacity var(--t); }
  .chart-wrap.dim { opacity: 0.6; }
  .chart-skel { height: 262px; }
  .overlay-note { position: absolute; inset: 34px 0 30px 40px; display: grid; place-items: center; color: var(--text-3); font-size: 0.9rem; pointer-events: none; }

  .leaders { display: grid; gap: 6px; }
  .leaders > li { display: grid; gap: 8px; padding: 12px; margin: 0 -12px; border-radius: var(--r); transition: background var(--t-fast); }
  .leaders > li:hover { background: var(--surface); }
  .leader-head { display: flex; align-items: center; gap: 12px; min-width: 0; }
  .rank { width: 18px; color: var(--text-3); font-weight: 700; font-size: 0.86rem; text-align: center; }
  .who { flex: 1; min-width: 0; display: grid; }
  .name { justify-self: start; font-weight: 650; font-size: 0.96rem; text-align: left; border-radius: 4px; transition: color var(--t-fast); }
  .name:hover { color: var(--accent); }
  .who small, .totals small { color: var(--text-3); font-size: 0.78rem; }
  .totals { display: grid; justify-items: end; }
  .totals strong { font-size: 1.02rem; }
  .meter { height: 4px; border-radius: 99px; background: var(--surface-2); overflow: hidden; margin-left: 30px; }
  .meter span { display: block; height: 100%; border-radius: inherit; background: var(--accent); transition: width var(--t-slow) var(--ease-out); animation: meter-in 800ms var(--ease-out) both; transform-origin: left; }
  @keyframes meter-in { from { transform: scaleX(0); } }
  .libs { display: grid; gap: 2px; margin-left: 30px; }
  .lib { width: 100%; display: grid; grid-template-columns: auto minmax(0, 7.5rem) minmax(40px, 1fr) auto auto; align-items: center; gap: 10px; padding: 6px 8px; margin: 0 -8px; border-radius: var(--r-s); color: var(--text-2); font-size: 0.84rem; text-align: left; transition: background var(--t-fast), color var(--t-fast); }
  .lib:hover { background: var(--surface-2); color: var(--text); }
  .lib :global(svg:last-child) { color: var(--text-3); opacity: 0; transform: translateX(-4px); transition: opacity var(--t-fast), transform var(--t) var(--ease-out); }
  .lib:hover :global(svg:last-child), .lib:focus-visible :global(svg:last-child) { opacity: 1; transform: none; }
  .lib-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lib-bar { height: 6px; border-radius: 99px; background: var(--surface-2); overflow: hidden; }
  .lib-bar span { display: block; height: 100%; border-radius: inherit; background: var(--accent); opacity: 0.7; }
  .lib-num { color: var(--text-3); font-size: 0.78rem; white-space: nowrap; }
  .user-skel { height: 92px; margin-bottom: 10px; }

  .libraries { display: grid; gap: 6px; min-width: 0; }
  .libraries > *, .top-row { min-width: 0; }
  .group .sub-head { align-items: center; color: var(--text-2); margin-top: 6px; }
  .top-row { display: grid; grid-auto-flow: column; grid-auto-columns: 96px; gap: 14px; overflow-x: auto; padding: 4px 2px 10px; scroll-snap-type: x proximity; }
  .top-item { display: grid; gap: 6px; scroll-snap-align: start; }
  .poster { position: relative; display: block; border-radius: var(--r-s); overflow: hidden; transition: transform var(--t) var(--ease-out), box-shadow var(--t); }
  .top-item:hover .poster { transform: translateY(-3px); box-shadow: var(--shadow-2); }
  .badge-rank { position: absolute; left: 6px; bottom: 6px; min-width: 22px; height: 22px; padding: 0 6px; display: grid; place-items: center; border-radius: 7px; background: rgb(7 8 11 / 0.78); backdrop-filter: blur(6px); font-size: 0.74rem; font-weight: 750; }
  .top-item strong { font-size: 0.82rem; font-weight: 600; line-height: 1.25; }
  .top-item small { color: var(--text-3); font-size: 0.72rem; margin-top: -3px; }
  @media (max-width: 560px) {
    .lib { grid-template-columns: auto minmax(0, 1fr) auto auto; }
    .lib-bar { display: none; }
    .chart-filters { flex-basis: 100%; }
    .pick { min-width: 0; flex: 1 1 120px; }
  }
</style>
