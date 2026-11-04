<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import X from "phosphor-svelte/lib/X";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { untrack } from "svelte";
  import { ToggleGroup } from "bits-ui";
  import { SearchStore } from "$houdini";
  import { inview } from "$lib/actions/index.js";
  import { mergeRanked } from "$lib/merge.js";
  import { askServers, connectedServers } from "$lib/servers.svelte.js";
  import { titleCase } from "$lib/format.js";
  import { failure, fetchQuery, nextPage } from "$lib/graphql.js";
  import { attempt } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import WorkCard from "$components/WorkCard.svelte";

  const MODES = [["HYBRID", "Balanced"], ["LEXICAL", "Exact words"], ["VECTOR", "Meaning"]];
  const KINDS = ["MOVIE", "SERIES", "EPISODE", "ALBUM", "TRACK", "ARTIST", "AUDIO_BOOK", "PHOTO"];
  const EXAMPLES = ["a rainy detective story", "heist gone wrong", "feel-good 90s comedy", "space survival", "jazz for a late night"];

  let text = $state(page.url.searchParams.get("q") ?? "");
  let mode = $state(page.url.searchParams.get("mode") ?? "HYBRID");
  let kinds = $state(page.url.searchParams.getAll("kind"));
  let input = $state();

  const search = new SearchStore();
  let query = $derived(text.trim());
  let wanted = $derived({ text: query, mode, kinds: kinds.length ? kinds : null });
  let current = $derived($search.variables?.text === query && $search.variables?.mode === mode && JSON.stringify($search.variables?.kinds ?? null) === JSON.stringify(wanted.kinds));
  let ownResults = $derived(query.length >= 2 ? ($search.data?.search.edges ?? []).map((edge) => edge.node).filter((hit) => hit.work) : []);

  // Other servers' hits join as they answer. Merged by rank, since scores from
  // different servers mean different things.
  let elsewhere = $state([]);
  $effect(() => {
    const variables = wanted;
    elsewhere = [];
    if (query.length < 2 || !connectedServers().length) return;
    let stale = false;
    const timer = setTimeout(() => {
      askServers(new SearchStore(), variables, (data) => data.search.edges.map((edge) => edge.node.work).filter(Boolean)).then((asked) => {
        if (!stale) elsewhere = asked.answers.map((answer) => answer.works);
      });
    }, 220);
    return () => { stale = true; clearTimeout(timer); };
  });
  let results = $derived.by(() => {
    if (!elsewhere.length) return ownResults;
    const hits = new Map(ownResults.map((hit) => [hit.work.id, hit]));
    return mergeRanked([ownResults.map((hit) => hit.work), ...elsewhere]).map((work) => hits.get(work.id) ?? { work, workId: work.id, matchedFields: [] });
  });
  let searched = $derived(query.length >= 2 && $search.data ? $search.variables?.text ?? "" : "");
  let loading = $derived(query.length >= 2 && ($search.fetching || !current));
  let error = $derived(query.length >= 2 && current ? failure($search) : "");
  let more = $state(false);

  async function loadMore() {
    if (more || loading || !$search.pageInfo.hasNextPage) return;
    more = true;
    await attempt(() => nextPage(search));
    more = false;
  }

  $effect(() => {
    const params = new URLSearchParams();
    if (query) params.set("q", query);
    if (mode !== "HYBRID") params.set("mode", mode);
    for (const kind of kinds) params.append("kind", kind);
    const suffix = params.size ? `?${params}` : "";
    if (suffix !== untrack(() => page.url.search)) goto(`/search${suffix}`, { replaceState: true, noScroll: true, keepFocus: true });

    if (query.length < 2) return;
    const variables = wanted;
    const timer = setTimeout(() => { fetchQuery(search, variables); }, 220);
    return () => clearTimeout(timer);
  });

  $effect(() => { input?.focus(); });

  const toggleKind = (kind) => (kinds = kinds.includes(kind) ? kinds.filter((item) => item !== kind) : [...kinds, kind]);
</script>

<svelte:head><title>{text ? `“${text}”` : "Search"} · NoFlex</title></svelte:head>

<div class="page">
  <header class="head">
    <label class="box" class:busy={loading}>
      <MagnifyingGlass size={24} />
      <span class="sr-only">Search your libraries</span>
      <input bind:this={input} bind:value={text} placeholder="Titles, people, moods — or describe what you remember" autocomplete="off" spellcheck="false" enterkeyhint="search" />
      {#if loading}<span class="spinner"></span>{:else if text}<button class="icon-btn sm" aria-label="Clear search" onclick={() => { text = ""; input.focus(); }}><X size={16} /></button>{/if}
    </label>
    <div class="options">
      <ToggleGroup.Root type="single" bind:value={() => mode, (next) => { if (next) mode = next; }} class="modes" aria-label="Search method">
        {#each MODES as [value, label]}<ToggleGroup.Item {value} class="mode">{label}</ToggleGroup.Item>{/each}
      </ToggleGroup.Root>
      <div class="kinds" role="group" aria-label="Media types">
        {#each KINDS as kind}<button class="chip" class:on={kinds.includes(kind)} aria-pressed={kinds.includes(kind)} onclick={() => toggleKind(kind)}>{titleCase(kind)}</button>{/each}
      </div>
    </div>
  </header>

  {#if error}
    <State kind="error" title="Search couldn’t finish" message={error} />
  {:else if text.trim().length < 2}
    <section class="prompt fade-up">
      <span class="glyph"><Sparkle size={26} /></span>
      <h2>Search the way you remember things.</h2>
      <p>Balanced search blends exact metadata with meaning, so a half-remembered plot still finds its film.</p>
      <div class="examples">{#each EXAMPLES as example}<button class="chip outline" onclick={() => (text = example)}>{example}</button>{/each}</div>
    </section>
  {:else if loading && !results.length}
    <div class="poster-grid" aria-busy="true">{#each Array(12) as _}<div class="skeleton" style:aspect-ratio="2 / 3"></div>{/each}</div>
  {:else if searched && !results.length}
    <State icon={MagnifyingGlass} title="No close matches" message="Try fewer words, switch to “Meaning”, or clear the type filters." />
  {:else if results.length}
    <div class="summary"><h2>{results.length} {results.length === 1 ? "result" : "results"}</h2><span class="faint">for “{searched}”</span></div>
    <div class="poster-grid stagger" class:stale={loading}>
      {#each results as hit, index (hit.workId)}
        <div class="hit" style:--i={index % 48}>
          <WorkCard work={hit.work} key="search:{hit.workId}" />
          {#if hit.matchedFields.length}<span class="why" title="Matched on {hit.matchedFields.join(', ')}">{hit.matchedFields.slice(0, 2).map(titleCase).join(" · ")}</span>{/if}
        </div>
      {/each}
    </div>
    {#if $search.pageInfo.hasNextPage && current}
      {#key results.length}<div class="sentinel" use:inview={{ callback: loadMore }}>{#if more}<span class="spinner"></span>{/if}</div>{/key}
    {/if}
  {/if}
</div>

<style>
  .sentinel { display: grid; place-items: center; height: 120px; }
  .head { display: grid; gap: 16px; margin: 12px 0 36px; }
  .box { display: flex; align-items: center; gap: 14px; height: 66px; padding: 0 18px 0 22px; border-radius: 20px; background: rgb(255 255 255 / 0.05); border: 1px solid var(--line); color: var(--text-3); transition: border-color var(--t), background var(--t), box-shadow var(--t); }
  .box:focus-within { border-color: rgb(165 220 255 / 0.5); background: rgb(255 255 255 / 0.07); box-shadow: 0 0 0 5px rgb(165 220 255 / 0.1), var(--shadow-2); }
  .box input { flex: 1; min-width: 0; height: 100%; border: 0; outline: 0; background: none; font-size: clamp(1.05rem, 2vw, 1.3rem); color: var(--text); }
  .options { display: flex; flex-wrap: wrap; gap: 12px 18px; align-items: center; }
  .options :global(.modes) { display: inline-flex; padding: 4px; border-radius: 99px; background: var(--surface); border: 1px solid var(--line); }
  .options :global(.mode) { height: 32px; padding: 0 14px; border-radius: 99px; font-size: 0.86rem; font-weight: 600; color: var(--text-3); transition: background var(--t), color var(--t); }
  .options :global(.mode[data-state="on"]) { background: var(--surface-3); color: var(--text); }
  .kinds { display: flex; flex-wrap: wrap; gap: 6px; }
  .kinds .chip { cursor: pointer; transition: background var(--t), color var(--t); }
  .kinds .chip:hover { color: var(--text); }
  .kinds .chip.on { background: var(--accent-soft); color: var(--accent); }
  .prompt { display: grid; justify-items: center; text-align: center; gap: 12px; padding: 7vh 0; max-width: 560px; margin: 0 auto; }
  .glyph { display: grid; place-items: center; width: 60px; height: 60px; border-radius: 20px; background: var(--accent-soft); color: var(--accent); margin-bottom: 4px; }
  .prompt h2 { font-size: 1.5rem; }
  .prompt p { color: var(--text-2); }
  .examples { display: flex; flex-wrap: wrap; justify-content: center; gap: 8px; margin-top: 8px; }
  .examples .chip { height: 32px; cursor: pointer; transition: border-color var(--t), color var(--t); }
  .examples .chip:hover { border-color: var(--text-3); color: var(--text); }
  .summary { display: flex; align-items: baseline; gap: 10px; margin-bottom: 18px; }
  .summary h2 { font-size: 1.1rem; }
  .stale { opacity: 0.6; transition: opacity var(--t); }
  .hit { display: grid; gap: 4px; }
  .why { padding: 0 2px; font-size: 0.74rem; color: var(--accent); opacity: 0.8; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
