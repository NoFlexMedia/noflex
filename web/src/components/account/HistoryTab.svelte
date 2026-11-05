<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
  import { graphql } from "$houdini";
  import { date, titleCase } from "$lib/format.js";
  import { failure, fetchQuery, nextPage, refetch } from "$lib/graphql.js";
  import { subtitleFor } from "$lib/playback.svelte.js";
  import { attempt } from "$lib/ui.svelte.js";
  import Artwork from "$components/Artwork.svelte";
  import State from "$components/State.svelte";

  const history = graphql(`
    query PlayHistory {
      playHistory(first: 30) @paginate {
        edges {
          node {
            id
            targetKind
            data { ...UserDataFields }
            work { id title kind year parentIndex indexNumber images { kind url } }
          }
        }
      }
    }
  `);
  $effect(() => { fetchQuery(history); });
  let rows = $derived(($history.data?.playHistory.edges ?? []).map((edge) => edge.node));
  let loading = $derived($history.fetching && !$history.data);
  let error = $derived(failure($history));
  let more = $state(false);

  async function loadMore() {
    if (more || !$history.pageInfo.hasNextPage) return;
    more = true;
    await attempt(() => nextPage(history));
    more = false;
  }

  const dayKey = (value) => (value ? new Date(value).toDateString() : "");
  function dayLabel(value) {
    if (!value) return "Undated";
    const day = new Date(value);
    const today = new Date();
    const yesterday = new Date(Date.now() - 86_400_000);
    if (day.toDateString() === today.toDateString()) return "Today";
    if (day.toDateString() === yesterday.toDateString()) return "Yesterday";
    return date(value, { weekday: "long", month: "long", day: "numeric", ...(day.getFullYear() !== today.getFullYear() ? { year: "numeric" } : {}) });
  }
  let groups = $derived.by(() => {
    const result = [];
    for (const row of rows) {
      const key = dayKey(row.data.lastPlayed);
      const last = result.at(-1);
      if (last?.key === key) last.rows.push(row); else result.push({ key, label: dayLabel(row.data.lastPlayed), rows: [row] });
    }
    return result;
  });

  function status(row) {
    const parts = [];
    if (row.data.played) parts.push("Finished");
    else if (row.data.positionMs) parts.push(`Stopped at ${Math.max(1, Math.round(row.data.positionMs / 60_000))} min`);
    if (row.data.playCount > 1) parts.push(`played ${row.data.playCount}×`);
    return parts.join(" · ") || "Started";
  }
  const meta = (work) => [work.kind === "EPISODE" ? subtitleFor(work) : work.year, titleCase(work.kind)].filter(Boolean).join(" · ");
</script>

{#if loading && !rows.length}
  <div class="history" aria-busy="true">
    <div class="skeleton day-skel"></div>
    {#each Array(6) as _}<div class="row-skel"><div class="skeleton thumb"></div><div class="lines"><div class="skeleton"></div><div class="skeleton short"></div></div></div>{/each}
  </div>
{:else if error && !rows.length}
  <State kind="error" title="History didn’t load" message={error} action={() => refetch(history)} />
{:else if !rows.length}
  <State icon={ClockCounterClockwise} title="Nothing watched yet" message="Everything you start or finish shows up here, newest first.">
    <a class="btn sm" href="/">Find something to watch</a>
  </State>
{:else}
  <div class="history">
    {#each groups as group (group.key)}
      <section>
        <h2 class="eyebrow">{group.label}</h2>
        <ol class="stagger">
          {#each group.rows as row, index (row.id)}
            <li style:--i={index}>
              {#if row.work}
                <a class="item" href="/works/{row.work.id}">
                  <span class="poster"><Artwork work={row.work} kind="poster" width={48} /></span>
                  <span class="what"><strong>{row.work.title}</strong><small>{meta(row.work)}</small></span>
                  <span class="state"><small class:done={row.data.played}>{#if row.data.played}<Check size={13} weight="bold" />{/if}{status(row)}</small><time datetime={row.data.lastPlayed}>{date(row.data.lastPlayed, { timeStyle: "short" })}</time></span>
                </a>
              {:else}
                <div class="item gone">
                  <span class="poster missing"><ClockCounterClockwise size={18} /></span>
                  <span class="what"><strong>Unavailable {titleCase(row.targetKind).toLowerCase()}</strong><small>Removed, or no longer shared with you</small></span>
                  <span class="state"><small>{status(row)}</small><time datetime={row.data.lastPlayed}>{date(row.data.lastPlayed, { timeStyle: "short" })}</time></span>
                </div>
              {/if}
            </li>
          {/each}
        </ol>
      </section>
    {/each}
    {#if $history.pageInfo.hasNextPage}
      <div class="more"><button class="btn outline" onclick={loadMore} disabled={more}>{#if more}<span class="spinner sm"></span>{/if}Load older plays</button></div>
    {:else if rows.length > 12}
      <p class="end">That’s the beginning of your history.</p>
    {/if}
  </div>
{/if}

<style>
  .history { display: grid; gap: 28px; max-width: 860px; }
  .day-skel { height: 14px; width: 120px; border-radius: 5px; }
  .row-skel { display: flex; gap: 14px; align-items: center; }
  .row-skel .thumb { width: 48px; height: 72px; border-radius: 8px; flex: none; }
  .row-skel .lines { flex: 1; display: grid; gap: 8px; }
  .row-skel .lines .skeleton { height: 14px; width: 50%; border-radius: 5px; }
  .row-skel .lines .short { width: 28%; }
  h2.eyebrow { margin-bottom: 8px; padding-left: 10px; }
  ol { display: grid; gap: 2px; }
  .item { display: flex; align-items: center; gap: 14px; padding: 8px 10px; border-radius: 14px; transition: background var(--t); }
  a.item:hover { background: var(--surface); }
  a.item:hover .poster { transform: scale(1.04); }
  .poster { width: 48px; flex: none; border-radius: 8px; overflow: hidden; box-shadow: 0 0 0 1px var(--line); transition: transform var(--t) var(--ease-out); }
  .poster.missing { display: grid; place-items: center; height: 72px; background: var(--surface-2); color: var(--text-3); }
  .what { flex: 1; display: grid; gap: 2px; min-width: 0; }
  .what strong { font-weight: 600; font-size: 0.95rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .what small, .state small { color: var(--text-3); font-size: 0.82rem; }
  .gone .what strong { color: var(--text-2); }
  .state { display: grid; justify-items: end; gap: 2px; text-align: right; }
  .state small { display: inline-flex; align-items: center; gap: 4px; }
  .state small.done { color: var(--success); }
  time { color: var(--text-3); font-size: 0.78rem; font-variant-numeric: tabular-nums; }
  .more { display: flex; justify-content: center; }
  .end { text-align: center; color: var(--text-3); font-size: 0.86rem; }
  @media (max-width: 520px) { .state small { display: none; } }
</style>
