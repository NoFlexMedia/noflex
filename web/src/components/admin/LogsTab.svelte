<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import CaretDown from "phosphor-svelte/lib/CaretDown";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import { untrack } from "svelte";
  import { AdminLogsStore } from "$houdini";
  import { failure, fetchQuery, nextPage, refetch } from "$lib/graphql.js";
  import { attempt, poll } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import Select from "$components/ui/Select.svelte";
  import { clock, stamp } from "./status/fmt.js";

  const logs = new AdminLogsStore();

  let level = $state("WARN");
  let target = $state("");
  let contains = $state("");
  let follow = $state(true);
  let open = $state({});
  let loadingMore = $state(false);

  // Text filters settle before they refetch, so typing doesn't spam the server.
  let applied = $state({ target: "", contains: "" });
  $effect(() => {
    const next = { target: target.trim(), contains: contains.trim() };
    const timer = setTimeout(() => (applied = next), 300);
    return () => clearTimeout(timer);
  });

  let variables = $derived({ minLevel: level === "ALL" ? null : level, target: applied.target || null, contains: applied.contains || null });
  $effect(() => {
    const vars = variables;
    untrack(() => fetchQuery(logs, vars));
  });

  let edges = $derived($logs.data?.recentLogs.edges ?? []);
  // A failing background task logs the same line every few seconds;
  // consecutive repeats fold into one row with a count.
  let rows = $derived.by(() => {
    const out = [];
    for (const { node } of edges) {
      const last = out.at(-1);
      if (last && last.node.level === node.level && last.node.target === node.target && last.node.message === node.message) {
        last.count += 1;
        last.oldest = node.at;
      } else {
        out.push({ node, count: 1, oldest: node.at });
      }
    }
    return out;
  });
  const ERROR_FIELDS = ["error", "err", "reason"];
  const errorOf = (node) => node.fields.find((field) => ERROR_FIELDS.includes(field.name))?.value ?? "";
  let hasMore = $derived($logs.pageInfo?.hasNextPage ?? false);
  // Following refetches the first page, which would drop older pages the admin loaded.
  let paged = $derived(edges.length > 200);
  $effect(() => {
    if (!follow || paged) return;
    return poll(() => refetch(logs), 5000);
  });

  async function more() {
    loadingMore = true;
    await attempt(() => nextPage(logs));
    loadingMore = false;
  }

  const LEVELS = [
    { value: "ALL", label: "All levels" },
    { value: "INFO", label: "Info and up" },
    { value: "WARN", label: "Warnings and errors" },
    { value: "ERROR", label: "Errors only" },
  ];
  const short = (t) => t.replace(/^noflex_/, "").replace(/::/g, "·");
</script>

<div class="diag">
  <div class="toolbar">
    <div class="level"><Select bind:value={level} items={LEVELS} label="Minimum level" /></div>
    <input class="input search-input" type="search" placeholder="Module, e.g. noflex_scanner" bind:value={target} aria-label="Filter by module" spellcheck="false" />
    <input class="input search-input" type="search" placeholder="Search messages and fields" bind:value={contains} aria-label="Search log text" />
    <span class="spacer"></span>
    <label class="check" title={paged ? "Paused while older entries are loaded" : "Refresh the newest entries every 5 s"}><input type="checkbox" bind:checked={follow} disabled={paged} />Follow</label>
    <button class="icon-btn sm" aria-label="Refresh" onclick={() => refetch(logs)}><ArrowsClockwise size={16} /></button>
  </div>
  <p class="hint">The server keeps the last 2,000 events at info level and above, and its last 1,000 warnings and errors, in memory. They reset when it restarts.</p>

  {#if $logs.fetching && !$logs.data}
    <div class="panel dense">{#each Array(8) as _}<div class="skeleton line"></div>{/each}</div>
  {:else if failure($logs) && !$logs.data}
    <State kind="error" title="Couldn’t read the log" message={failure($logs)} action={() => refetch(logs)} />
  {:else if !edges.length}
    <div class="panel empty-panel"><State compact icon={CheckCircle} title="Nothing logged" message={level === "ALL" || level === "INFO" ? "No events match these filters." : "No warnings or errors since the server started."} /></div>
  {:else}
    <ol class="panel dense log" aria-busy={$logs.fetching}>
      {#each rows as { node, count, oldest } (node.id)}
        {@const expandable = node.fields.length || node.spans.length}
        {@const error = errorOf(node)}
        <li class="entry {node.level.toLowerCase()}">
          <button class="row" onclick={() => expandable && (open[node.id] = !open[node.id])} aria-expanded={expandable ? !!open[node.id] : undefined} disabled={!expandable}>
            <time class="tabular" datetime={node.at} title={stamp(node.at)}>{clock(node.at)}</time>
            <span class="lvl">{node.level}</span>
            <span class="target mono" title={node.target}>{short(node.target)}</span>
            <span class="msg">
              {#if count > 1}<span class="repeat" title="Repeated {count} times since {stamp(oldest)}">×{count}</span>{/if}
              {node.message || node.fields.map((field) => `${field.name}=${field.value}`).join(" ")}
              {#if error && node.message}<span class="err-preview">{error}</span>{/if}
            </span>
            {#if expandable}<span class="more" class:on={open[node.id]}><CaretDown size={14} /></span>{/if}
          </button>
          {#if open[node.id]}
            <div class="detail">
              {#if node.spans.length}<p class="spans mono">{node.spans.join(" › ")}</p>{/if}
              {#if node.fields.length}
                <dl>{#each node.fields as field, i (field.name + i)}<dt class="mono">{field.name}</dt><dd class="mono">{field.value}</dd>{/each}</dl>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ol>
    {#if hasMore}
      <div class="more-row"><button class="btn sm" onclick={more} disabled={loadingMore}>{#if loadingMore}<span class="spinner sm"></span>{/if}Load older</button></div>
    {/if}
  {/if}
</div>

<style>
  .toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .level { width: 210px; }
  .level :global(.nf-select) { min-height: 36px; border-radius: 999px; font-size: 0.86rem; }
  .hint { color: var(--text-3); font-size: 0.8rem; margin-top: -8px; }
  .skeleton.line { height: 18px; margin: 8px 0; border-radius: 6px; }
  .log { padding: 6px; display: grid; }
  .entry + .entry { border-top: 1px solid var(--line); }
  .row { display: grid; grid-template-columns: 8.4ch 6ch minmax(0, 13rem) minmax(0, 1fr) 18px; align-items: baseline; gap: 12px; width: 100%; padding: 7px 10px; border-radius: 8px; text-align: left; font-size: 0.84rem; color: var(--text-2); }
  .row:disabled { cursor: default; }
  .row:not(:disabled):hover { background: var(--surface-2); }
  time { color: var(--text-3); font-size: 0.78rem; }
  .lvl { font-size: 0.66rem; font-weight: 700; letter-spacing: 0.06em; color: var(--text-3); }
  .warn .lvl { color: var(--warning); }
  .error .lvl { color: var(--danger); }
  .error .msg { color: var(--text); }
  .target { color: var(--text-3); font-size: 0.76rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .msg { overflow-wrap: anywhere; min-width: 0; }
  .repeat { display: inline-block; margin-right: 6px; padding: 0 6px; border-radius: 5px; background: var(--surface-3); color: var(--text-2); font-size: 0.72rem; font-weight: 700; font-variant-numeric: tabular-nums; }
  .err-preview { display: block; margin-top: 2px; color: var(--text-3); font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 0.74rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .entry.error .err-preview { color: rgb(255 107 107 / 0.8); }
  .more { color: var(--text-3); transition: transform var(--t) var(--ease-out); align-self: center; }
  .more.on { transform: rotate(180deg); }
  .detail { margin: 0 10px 10px calc(14.4ch + 24px); padding: 10px 12px; border-radius: var(--r-s); background: rgb(0 0 0 / 0.22); font-size: 0.78rem; }
  .spans { color: var(--focus); margin-bottom: 8px; overflow-wrap: anywhere; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 4px 14px; }
  dt { color: var(--text-3); }
  dd { color: var(--text-2); overflow-wrap: anywhere; white-space: pre-wrap; }
  .more-row { display: flex; justify-content: center; }
  @media (max-width: 760px) {
    .row { grid-template-columns: 8.4ch 5.5ch minmax(0, 1fr) 18px; }
    .target { grid-column: 1 / 4; grid-row: 2; }
    .msg { grid-column: 1 / 4; grid-row: 3; }
    .more { grid-column: 4; grid-row: 1; }
    .detail { margin-left: 10px; }
    .level { width: 100%; }
  }
</style>
