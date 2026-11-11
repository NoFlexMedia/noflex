<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import DownloadSimple from "phosphor-svelte/lib/DownloadSimple";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { graphql } from "$houdini";
  import { languageName } from "$lib/format.js";
  import { request } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import Dialog from "$components/ui/Dialog.svelte";
  import Segmented from "$components/ui/Segmented.svelte";

  /**
   * Search online for a media file's subtitles and download one. `onpick`
   * gets the new track's index and the file's tracks as they now are.
   */
  let { open = $bindable(false), mediaFileId, portal = undefined, onpick } = $props();

  const search = graphql(`
    query SubtitleSearch($mediaFileId: ID!, $languages: [String!]) @cache(policy: NetworkOnly) {
      searchSubtitles(mediaFileId: $mediaFileId, languages: $languages) {
        offers { provider providerName handle language locale format release hearingImpaired forced hashMatch downloadCount }
        failures { provider providerName message }
      }
    }
  `);
  const download = graphql(`
    mutation DownloadSubtitle($mediaFileId: ID!, $provider: String!, $handle: String!) {
      downloadSubtitle(mediaFileId: $mediaFileId, provider: $provider, handle: $handle) {
        index
        mediaFile { ...MediaFileFull }
      }
    }
  `);

  let preferred = $derived(app.playbackPrefs.subtitleLanguages ?? []);
  let scope = $state("preferred");
  let result = $state(null);
  let error = $state("");
  let loading = $state(false);
  // Held back so a quick answer shows no spinner.
  let slow = $state(false);
  let picking = $state(null);
  let run = 0;

  $effect(() => {
    if (!open || !mediaFileId) return;
    const languages = scope === "all" || !preferred.length ? [] : null;
    const mine = ++run;
    loading = true; slow = false; error = "";
    const timer = setTimeout(() => { if (mine === run) slow = true; }, 250);
    request(search, { mediaFileId, languages }).then(
      (data) => { if (mine === run) result = data.searchSubtitles; },
      (cause) => { if (mine === run) error = cause instanceof Error ? cause.message : String(cause); },
    ).finally(() => { clearTimeout(timer); if (mine === run) loading = false; });
  });

  let groups = $derived.by(() => {
    const byLanguage = new Map();
    for (const offer of result?.offers ?? []) {
      const key = offer.locale ?? offer.language;
      if (!byLanguage.has(key)) byLanguage.set(key, { key, label: languageLabel(offer), offers: [] });
      byLanguage.get(key).offers.push(offer);
    }
    return [...byLanguage.values()];
  });

  function languageLabel(offer) {
    const name = languageName(offer.language) || offer.language;
    const region = offer.locale?.split("-")[1];
    return region ? `${name} (${region})` : name;
  }

  async function pick(offer) {
    if (picking) return;
    picking = offer.handle;
    const added = await attempt(async () => (await request(download, { mediaFileId, provider: offer.provider, handle: offer.handle })).downloadSubtitle, "Subtitles added");
    picking = null;
    if (!added) return;
    open = false;
    onpick?.(added.index, added.mediaFile.subtitleTracks);
  }

  const count = new Intl.NumberFormat(undefined, { notation: "compact" });
</script>

<Dialog bind:open title="Search online for subtitles" description="Matches for this exact file come first." width={560} {portal}>
  {#if preferred.length}
    <div class="scope"><Segmented bind:value={scope} label="Languages" size="sm" items={[{ value: "preferred", label: "My languages" }, { value: "all", label: "All languages" }]} /></div>
  {/if}
  {#if loading && !result}
    {#if slow}
      <div class="rows" aria-busy="true">{#each [0, 1, 2, 3] as row (row)}<div class="skeleton row-skeleton"></div>{/each}</div>
    {/if}
  {:else if error}
    <State kind="error" compact title="Couldn't search for subtitles" message={error} />
  {:else if result}
    {#each result.failures as failure (failure.provider)}
      <p class="failure" role="status"><WarningCircle size={18} />{failure.message}</p>
    {/each}
    {#if !groups.length}
      <State compact icon={MagnifyingGlass} title="No subtitles found" message={scope === "preferred" && preferred.length ? "Nothing in your languages. Try all languages." : "No provider has subtitles for this file."}>
        {#if scope === "preferred" && preferred.length}<button class="btn sm" type="button" onclick={() => (scope = "all")}>Search all languages</button>{/if}
      </State>
    {/if}
    <div class="groups" class:dim={loading}>
      {#each groups as group (group.key)}
        <section>
          <h3 class="eyebrow">{group.label}</h3>
          <ul>
            {#each group.offers as offer (offer.provider + offer.handle)}
              <li>
                <button type="button" class="offer" disabled={picking != null} onclick={() => pick(offer)}>
                  <span class="text">
                    <span class="release">{offer.release ?? "Unnamed release"}</span>
                    <span class="meta">
                      {#if offer.hashMatch}<span class="chip accent"><CheckCircle size={14} weight="fill" />Matches this file</span>{/if}
                      {#if offer.hearingImpaired}<span class="chip">SDH</span>{/if}
                      {#if offer.forced}<span class="chip">Forced</span>{/if}
                      <span class="faint">{offer.providerName}{#if offer.downloadCount != null} · {count.format(offer.downloadCount)} downloads{/if}</span>
                    </span>
                  </span>
                  <span class="go" class:busy={picking === offer.handle}>{#if picking === offer.handle}<span class="spinner sm"></span>{:else}<DownloadSimple size={18} />{/if}</span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    </div>
  {/if}
</Dialog>

<style>
  .scope { margin-bottom: 14px; }
  .rows { display: grid; gap: 8px; }
  .row-skeleton { height: 58px; }
  .failure { display: flex; align-items: center; gap: 8px; margin: 0 0 12px; padding: 10px 12px; border-radius: var(--r); background: var(--surface-2); color: var(--text-2); font-size: 0.86rem; }
  .failure :global(svg) { flex: none; color: var(--warning); }
  .groups { display: grid; gap: 18px; transition: opacity 160ms var(--ease-out); }
  .groups.dim { opacity: 0.55; }
  h3 { margin: 0 0 8px; }
  ul { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
  .offer { display: flex; align-items: center; gap: 12px; width: 100%; padding: 10px 12px; border: 1px solid var(--line); border-radius: var(--r); background: transparent; color: inherit; text-align: left; cursor: pointer; transition: background 140ms var(--ease-out), transform 140ms var(--ease-out); }
  .offer:hover:not(:disabled) { background: var(--surface-2); }
  .offer:active:not(:disabled) { transform: scale(0.985); }
  .offer:disabled { cursor: default; }
  .text { display: grid; gap: 6px; flex: 1; min-width: 0; }
  .release { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 550; }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; font-size: 0.8rem; }
  .go { display: grid; place-items: center; flex: none; width: 34px; height: 34px; border-radius: 999px; background: var(--surface-2); color: var(--text-2); }
  .offer:hover:not(:disabled) .go { color: var(--accent); }
  @media (prefers-reduced-motion: reduce) { .offer, .groups { transition: none; } }
</style>
