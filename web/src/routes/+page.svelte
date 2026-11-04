<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Plus from "phosphor-svelte/lib/Plus";
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import Play from "phosphor-svelte/lib/Play";
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import Trash from "phosphor-svelte/lib/Trash";
  import { graphql, HomeStore } from "$houdini";
  import { mergeContinueWatching, mergeRanked } from "$lib/merge.js";
  import { askServersForLists, connectedServers } from "$lib/servers.svelte.js";
  import { failure, fetchQuery, refetch } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import { date, titleCase } from "$lib/format.js";
  import { pickImage } from "$lib/images.js";
  import Hero from "$components/Hero.svelte";
  import Rail from "$components/Rail.svelte";
  import State from "$components/State.svelte";

  let { data: route } = $props();
  let home = $derived(route.home);
  // The navigation follows, so a first scan's content appears without a reload.
  $effect(() => { if ($home.data) app.categories = [...$home.data.contentCategories]; });
  const activity = graphql(`
    query Activity {
      activity(first: 12) { nodes { id kind actor subject detail occurredAt } }
    }
  `);
  $effect(() => { if (app.viewer.isAdmin) fetchQuery(activity); });
  const TOP_LEVEL = new Set(["MOVIE", "SERIES", "ALBUM", "AUDIO_BOOK"]);

  // Every shelf is a connection; the rails only need its nodes.
  const SHELVES = ["continueWatching", "recentlyAdded", "recentlyPlayed", "recommendations"];
  const MERGED = { continueWatching: mergeContinueWatching, recentlyAdded: mergeRanked, recommendations: mergeRanked };
  let ownShelves = $derived($home.data && Object.fromEntries(SHELVES.map((shelf) => [shelf, $home.data[shelf].nodes])));

  // Other servers fill in as they answer; this server's rails never wait on them.
  let elsewhere = $state([]);
  let unreachable = $state([]);
  $effect(() => {
    if (!connectedServers().length) { elsewhere = []; unreachable = []; return; }
    askAllShelves();
  });
  async function askAllShelves() {
    const asked = await askServersForLists(new HomeStore(), {}, (result) => Object.fromEntries(Object.keys(MERGED).map((shelf) => [shelf, result[shelf].nodes])));
    elsewhere = asked.answers;
    unreachable = asked.missing;
  }
  let data = $derived.by(() => {
    if (!ownShelves) return ownShelves;
    const merged = { ...ownShelves };
    if (elsewhere.length) for (const [shelf, merge] of Object.entries(MERGED)) merged[shelf] = merge([ownShelves[shelf], ...elsewhere.map((answer) => answer.lists[shelf])]);
    return merged;
  });
  let featured = $derived.by(() => {
    // This server's own: the hero links and plays without any server context.
    const pool = (ownShelves?.recentlyAdded ?? []).filter((work) => TOP_LEVEL.has(work.kind));
    const withArt = pool.filter((work) => pickImage(work.images, "backdrop"));
    return (withArt.length ? withArt : pool).slice(0, 6);
  });
  let empty = $derived(data && !data.recentlyAdded.length && !data.continueWatching.length);
  const ACTIVITY_ICON = { PLAY: Play, SCAN: ArrowsClockwise, DELETE: Trash };
  const activityIcon = (kind) => ACTIVITY_ICON[Object.keys(ACTIVITY_ICON).find((prefix) => kind.startsWith(prefix))] ?? Plus;
</script>

<svelte:head><title>Home · NoFlex</title></svelte:head>

<div class="page home">
  {#if $home.fetching && !$home.data}
    <div class="hero-skeleton skeleton" aria-hidden="true"></div>
    {#each [0, 1] as row}
      <div class="rail-skeleton" aria-hidden="true"><div class="skeleton title"></div><div class="cards">{#each Array(7) as _}<div class="skeleton card"></div>{/each}</div></div>
    {/each}
    <span class="sr-only" role="status">Loading your home screen</span>
  {:else if failure($home) && !data}
    <State kind="error" title="Home is out of reach" message={failure($home)} action={() => refetch(home)} />
  {:else if empty}
    <section class="welcome fade-up">
      <span class="glyph"><Sparkle size={28} /></span>
      <h1>Welcome, {app.viewer.name}.</h1>
      {#if app.viewer.isAdmin}
        <p>{app.libraries.length ? "Your libraries are empty so far — run a scan once media is in place." : "Point NoFlex at a folder of films, shows, music or photos to get started."}</p>
        <a class="btn primary lg" href="/admin">{app.libraries.length ? "Manage libraries" : "Add your first library"}<ArrowRight size={18} /></a>
      {:else}
        <p>Nothing has been shared with you yet. Ask the server’s administrator to grant you a library.</p>
      {/if}
    </section>
  {:else if data}
    {#if featured.length}<Hero items={featured} />{/if}
    <div class="rails" class:flush={!featured.length}>
      {#if unreachable.length}<p class="away" role="status">{unreachable.join(", ")} can’t be reached right now.</p>{/if}
      {#if data.continueWatching.length}<Rail title="Continue watching" items={data.continueWatching} wide />{/if}
      {#if data.recentlyAdded.length}<Rail title="Recently added" items={data.recentlyAdded} />{/if}
      {#if data.recommendations.length}<Rail title="Recommended" eyebrow="Because of what you watch" items={data.recommendations} />{/if}
      {#if data.recentlyPlayed.length}<Rail title="Watch again" items={data.recentlyPlayed} />{/if}

      {#if $activity.data?.activity.nodes.length}
        <section class="section activity">
          <div class="section-head"><h2>On this server</h2></div>
          <ol class="stagger">
            {#each $activity.data.activity.nodes.slice(0, 8) as item, index (item.id)}
              {@const Glyph = activityIcon(item.kind)}
              <li style:--i={index}>
                <span class="mark"><Glyph size={15} /></span>
                <span class="what"><strong>{titleCase(item.kind)}</strong><small>{item.subject || item.detail || "Server activity"}</small></span>
                <time datetime={item.occurredAt}>{date(item.occurredAt, { dateStyle: "medium", timeStyle: "short" })}</time>
              </li>
            {/each}
          </ol>
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  .away { margin: 0; color: var(--text-3); font-size: 0.86rem; }
  .hero-skeleton { height: clamp(440px, 74vh, 760px); margin: calc(var(--topbar-h) * -1 - 20px) calc(var(--gutter) * -1) 0; border-radius: 0; }
  .rail-skeleton { margin-top: 38px; }
  .rail-skeleton .title { width: 200px; height: 22px; border-radius: 8px; margin-bottom: 16px; }
  .rail-skeleton .cards { display: grid; grid-auto-flow: column; grid-auto-columns: clamp(132px, 14vw, 184px); gap: 16px; overflow: hidden; }
  .rail-skeleton .card { aspect-ratio: 2 / 3; }
  .rails { position: relative; margin-top: -40px; }
  .rails.flush { margin-top: 0; }
  @media (max-width: 759px) { .rails { margin-top: 8px; } }
  .rails > :global(:first-child) { margin-top: 0; }
  .welcome { display: grid; justify-items: start; gap: 16px; max-width: 640px; padding: clamp(48px, 14vh, 140px) 0; }
  .welcome .glyph { display: grid; place-items: center; width: 64px; height: 64px; border-radius: 20px; background: var(--accent-soft); color: var(--accent); }
  .welcome h1 { font-size: clamp(2.2rem, 4.6vw, 3.6rem); letter-spacing: -0.04em; }
  .welcome p { font-size: 1.06rem; color: var(--text-2); }
  .activity ol { display: grid; gap: 2px; max-width: 760px; }
  .activity li { display: flex; align-items: center; gap: 14px; padding: 10px 12px; border-radius: 12px; transition: background var(--t); }
  .activity li:hover { background: var(--surface); }
  .mark { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 10px; background: var(--surface-2); color: var(--text-2); flex: none; }
  .what { flex: 1; display: grid; min-width: 0; }
  .what strong { font-size: 0.9rem; font-weight: 600; }
  .what small { color: var(--text-3); font-size: 0.82rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  time { color: var(--text-3); font-size: 0.8rem; white-space: nowrap; }
</style>
