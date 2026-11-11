<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script module>
  /** What each work kind can pick, as image kinds with a `*_IMAGE` lock. */
  export const PICKABLE = {
    MOVIE: ["PRIMARY", "BACKDROP", "LOGO"],
    SERIES: ["PRIMARY", "BACKDROP", "LOGO"],
    SEASON: ["PRIMARY"],
    EPISODE: ["THUMB"],
  };
  const LABEL = { PRIMARY: "Poster", BACKDROP: "Backdrop", LOGO: "Logo", THUMB: "Still" };
  /** Height over width for each kind's tiles; a logo keeps its own shape. */
  const RATIO = { PRIMARY: 1.5, BACKDROP: 0.5625, THUMB: 0.5625, LOGO: 0.4 };
</script>

<script>
  import Check from "phosphor-svelte/lib/Check";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import { graphql } from "$houdini";
  import { request } from "$lib/graphql.js";
  import { mediaUrl } from "$lib/tokens.js";
  import { attempt } from "$lib/ui.svelte.js";
  import ProviderLogo from "$components/ProviderLogo.svelte";
  import State from "$components/State.svelte";
  import Segmented from "$components/ui/Segmented.svelte";

  let { work } = $props();

  const candidatesQuery = graphql(`
    query ArtworkCandidates($work: ID!, $kind: ImageKind!) @cache(policy: NetworkOnly) {
      artworkCandidates(workId: $work, kind: $kind) { id provider kind language width height score previewUrl }
      providerAttributions { provider name logoUrl }
    }
  `);
  const setArtwork = graphql(`
    mutation SetWorkArtwork($work: ID!, $candidate: ID!) {
      setWorkArtwork(workId: $work, candidateId: $candidate) {
        ...WorkFull
        attributions { provider name text url logoUrl }
      }
    }
  `);

  let kinds = $derived(PICKABLE[work.kind] ?? ["PRIMARY"]);
  let kind = $state("");
  $effect(() => { if (!kinds.includes(kind)) kind = kinds[0]; });
  let items = $derived(kinds.map((value) => ({ value, label: LABEL[value] })));

  /** `null` until asked, then the answer for `kind`, or `"failed"`. */
  let result = $state(null);
  let loading = $state(false);
  let picking = $state("");
  let picked = $state("");
  let loaded = $state(new Set());
  let broken = $state(new Set());

  async function load() {
    loading = true;
    loaded = new Set();
    broken = new Set();
    const asked = kind;
    const data = await attempt(() => request(candidatesQuery, { work: work.id, kind: asked }));
    if (asked !== kind) return;
    loading = false;
    result = data ?? "failed";
  }

  function switchKind(next) {
    kind = next;
    picked = "";
    if (result || loading) load();
  }

  let groups = $derived.by(() => {
    if (!result || result === "failed") return [];
    const credits = new Map(result.providerAttributions.map((credit) => [credit.provider, credit]));
    const byProvider = new Map();
    for (const candidate of result.artworkCandidates) {
      if (!byProvider.has(candidate.provider)) byProvider.set(candidate.provider, { provider: candidate.provider, credit: credits.get(candidate.provider), candidates: [] });
      byProvider.get(candidate.provider).candidates.push(candidate);
    }
    return [...byProvider.values()];
  });

  async function pick(candidate) {
    picking = candidate.id;
    const done = await attempt(() => setArtwork.mutate({ work: work.id, candidate: candidate.id }), `${LABEL[kind]} updated and locked against refreshes`);
    picking = "";
    if (done) picked = candidate.id;
  }

  const mark = (set, id) => new Set(set).add(id);
  const size = (candidate) => (candidate.width && candidate.height ? `${candidate.width}×${candidate.height}` : null);
</script>

<div class="picker">
  <div class="controls">
    {#if kinds.length > 1}<Segmented value={kind} {items} label="Artwork kind" size="sm" onchange={switchKind} />{/if}
    {#if !result}
      <button class="btn outline sm" onclick={load} disabled={loading}>{#if loading}<span class="spinner sm"></span>{:else}<ImageIcon size={15} />{/if}Browse {LABEL[kind].toLowerCase()}s</button>
    {/if}
  </div>

  {#if loading}
    <div class="grid" class:wide={kind !== "PRIMARY"} class:logo={kind === "LOGO"} aria-busy="true" style:--ratio={RATIO[kind]}>
      {#each Array(6) as _}<div class="tile skeleton"></div>{/each}
    </div>
  {:else if result === "failed"}
    <State kind="error" compact title="Couldn’t ask the providers" message="Something went wrong while listing artwork." action={load} />
  {:else if result && !groups.length}
    <State compact icon={ImageIcon} title="No {LABEL[kind].toLowerCase()}s on offer" message="None of this server’s image providers has one for this title. If it isn’t matched yet, identify it first." />
  {:else if result}
    {#each groups as group (group.provider)}
      <section class="provider">
        <h4>{#if group.credit?.logoUrl}<ProviderLogo credit={group.credit} height={14} />{:else}{group.credit?.name ?? group.provider}{/if}<small>{group.candidates.length}</small></h4>
        <ul class="grid stagger" class:wide={kind !== "PRIMARY"} class:logo={kind === "LOGO"} style:--ratio={RATIO[kind]}>
          {#each group.candidates as candidate, index (candidate.id)}
            {#if !broken.has(candidate.id)}
              <li style:--i={Math.min(index, 12)}>
                <button class="tile" class:loaded={loaded.has(candidate.id)} class:on={picked === candidate.id} onclick={() => pick(candidate)} disabled={!!picking} aria-label="Use this {LABEL[kind].toLowerCase()} from {group.credit?.name ?? group.provider}">
                  <img src={mediaUrl(candidate.previewUrl)} alt="" loading="lazy" decoding="async" onload={() => (loaded = mark(loaded, candidate.id))} onerror={() => (broken = mark(broken, candidate.id))} />
                  {#if picking === candidate.id}<span class="busy"><span class="spinner sm"></span></span>{:else if picked === candidate.id}<span class="busy"><Check size={20} weight="bold" /></span>{/if}
                </button>
                <small>{candidate.language ? candidate.language.toUpperCase() : "No text"}{#if size(candidate)} · {size(candidate)}{/if}</small>
              </li>
            {/if}
          {/each}
        </ul>
      </section>
    {/each}
  {/if}
</div>

<style>
  .picker { display: grid; gap: 14px; }
  .controls { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
  .provider { display: grid; gap: 10px; }
  h4 { display: flex; align-items: center; gap: 8px; font-size: 0.86rem; font-weight: 600; color: var(--text-2); }
  h4 small { color: var(--text-3); font-weight: 500; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(var(--min, 96px), 1fr)); gap: 10px; }
  .grid.wide { --min: 150px; }
  li { display: grid; gap: 4px; min-width: 0; }
  li small { color: var(--text-3); font-size: 0.74rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tile { position: relative; display: block; width: 100%; aspect-ratio: 1 / var(--ratio); border-radius: var(--r-s); overflow: hidden; background: var(--surface-2); outline: 2px solid transparent; outline-offset: 2px; transition: transform var(--t-fast) var(--ease-out), outline-color var(--t-fast); }
  .tile:hover:not(:disabled) { transform: translateY(-2px); }
  .tile:active:not(:disabled) { transform: scale(0.97); }
  .tile.on { outline-color: var(--accent); }
  .tile img { width: 100%; height: 100%; object-fit: cover; opacity: 0; transition: opacity 260ms var(--ease-out); }
  .grid.logo .tile img { object-fit: contain; padding: 8px; }
  .tile.loaded img { opacity: 1; }
  .busy { position: absolute; inset: 0; display: grid; place-items: center; background: rgb(0 0 0 / 0.45); color: var(--text); }
  @media (prefers-reduced-motion: reduce) { .tile, .tile img { transition: none; } }
</style>
