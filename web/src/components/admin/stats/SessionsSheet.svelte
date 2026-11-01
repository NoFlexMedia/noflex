<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
  import { untrack } from "svelte";
  import { AdminPlaySessionsStore } from "$houdini";
  import { failure, fetchQuery, nextPage, refetch } from "$lib/graphql.js";
  import { attempt } from "$lib/ui.svelte.js";
  import Artwork from "$components/Artwork.svelte";
  import State from "$components/State.svelte";
  import Dialog from "$components/ui/Dialog.svelte";
  import { ago } from "$components/account/relative.js";
  import { RANGES, hours, playedTitle, utcOffset } from "./fmt.js";

  /** `scope`: { user: { id, name }, library: { id, name } | null, range } or null when closed. */
  let { scope = $bindable(null) } = $props();

  const sessions = new AdminPlaySessionsStore();
  let open = $derived(scope != null);
  let loadingMore = $state(false);

  $effect(() => {
    if (!scope) return;
    const vars = { range: scope.range, user: scope.user.id, library: scope.library?.id ?? null, offset: utcOffset() };
    untrack(() => fetchQuery(sessions, vars));
  });

  let rows = $derived(($sessions.data?.playSessions.edges ?? []).map((edge) => edge.node));
  let rangeName = $derived(RANGES.find((range) => range.value === scope?.range)?.long ?? "");

  async function more() {
    loadingMore = true;
    await attempt(() => nextPage(sessions));
    loadingMore = false;
  }

  const progress = (row) => (row.work?.runtimeMs ? Math.min(100, (row.positionMs / row.work.runtimeMs) * 100) : null);
  const when = (iso) => new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(iso));
</script>

<Dialog
  open={open}
  variant="sheet"
  width={560}
  title={scope ? `${scope.user.name}${scope.library ? ` · ${scope.library.name}` : ""}` : ""}
  description={scope ? `Viewing sessions over ${rangeName}, newest first.` : null}
  onclose={() => (scope = null)}
>
  {#if failure($sessions) && !rows.length}
    <State compact kind="error" title="Couldn’t load the sessions" message={failure($sessions)} action={() => refetch(sessions)} />
  {:else if $sessions.fetching && !rows.length}
    <div class="list" aria-busy="true">{#each Array(6) as _}<div class="skeleton row-skel"></div>{/each}</div>
  {:else if !rows.length}
    <State compact icon={ClockCounterClockwise} title="No sessions" message="Nothing was played here in this period." />
  {:else}
    <ol class="sessions stagger">
      {#each rows as row, index (row.id)}
        <li style:--i={index}>
          <a class="thumb" href={row.work ? `/works/${row.work.id}` : undefined} aria-label={playedTitle(row.work)}>
            <Artwork work={row.work ?? { title: row.rootWork?.title ?? "?" }} kind="thumb" width={112} />
          </a>
          <div class="info">
            <strong class="clamp-2">{row.rootWork && row.rootWork.id !== row.work?.id ? `${row.rootWork.title} — ` : ""}{playedTitle(row.work)}</strong>
            <small title={when(row.startedAt)}>{ago(row.startedAt)} · {hours(row.watchedMs)} watched{row.device ? ` · ${row.device}` : ""}</small>
            {#if progress(row) != null}<div class="progress-bar" aria-label="Stopped at {Math.round(progress(row))}%"><span style:width="{progress(row)}%"></span></div>{/if}
          </div>
          {#if !row.ended && Date.now() - Date.parse(row.lastSeenAt) < 120_000}<span class="state accent"><span class="dot live"></span>Live</span>{/if}
        </li>
      {/each}
    </ol>
    {#if $sessions.pageInfo?.hasNextPage}
      <div class="more"><button class="btn sm" onclick={more} disabled={loadingMore}>{#if loadingMore}<span class="spinner sm"></span>{/if}Load older sessions</button></div>
    {/if}
  {/if}
</Dialog>

<style>
  .sessions { display: grid; gap: 4px; }
  .sessions li { display: flex; align-items: center; gap: 14px; padding: 10px; margin: 0 -10px; border-radius: var(--r); transition: background var(--t-fast); }
  .sessions li:hover { background: var(--surface); }
  .thumb { width: 112px; flex: none; border-radius: var(--r-s); overflow: hidden; transition: transform var(--t) var(--ease-out); }
  .thumb:hover { transform: scale(1.03); }
  .info { flex: 1; min-width: 0; display: grid; gap: 4px; }
  .info strong { font-size: 0.92rem; font-weight: 600; }
  .info small { color: var(--text-3); font-size: 0.8rem; font-variant-numeric: tabular-nums; }
  .info .progress-bar { margin-top: 4px; max-width: 220px; }
  .row-skel { height: 66px; margin-bottom: 8px; }
  .more { display: flex; justify-content: center; margin-top: 14px; }
</style>
