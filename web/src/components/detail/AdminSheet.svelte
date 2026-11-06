<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import Lock from "phosphor-svelte/lib/Lock";
  import LockOpen from "phosphor-svelte/lib/LockOpen";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import PencilSimple from "phosphor-svelte/lib/PencilSimple";
  import Trash from "phosphor-svelte/lib/Trash";
  import { goto } from "$app/navigation";
  import { cache, graphql } from "$houdini";
  import { titleCase } from "$lib/format.js";
  import { request } from "$lib/graphql.js";
  import { ask, attempt } from "$lib/ui.svelte.js";
  import ProviderLogo from "$components/ProviderLogo.svelte";
  import Dialog from "$components/ui/Dialog.svelte";

  /** Edits land in the cache by themselves; `onremoved` follows an edition or file removal. */
  let { open = $bindable(false), work, edition = null, media = null, onremoved } = $props();

  const metadataSearch = graphql(`
    query MetadataSearch($query: String!, $kind: WorkKind!, $year: Int) @cache(policy: NetworkOnly) {
      searchMetadata(query: $query, kind: $kind, year: $year) { externalId { kind value } title year confidence provider }
      providerAttributions { provider name logoUrl }
    }
  `);
  const identifyWork = graphql(`
    mutation IdentifyWork($id: ID!, $provider: String!, $kind: ExternalIdKind!, $value: String!) {
      identifyWork(workId: $id, provider: $provider, externalId: { kind: $kind, value: $value }) {
        ...WorkFull
        attributions { provider name text url logoUrl }
      }
    }
  `);
  const refreshMetadata = graphql(`
    mutation RefreshMetadata($id: ID!, $force: Boolean!) {
      refreshMetadata(workId: $id, force: $force) {
        ...WorkFull
        attributions { provider name text url logoUrl }
      }
    }
  `);
  const updateWork = graphql(`
    mutation UpdateWorkFields($id: ID!, $fields: WorkFieldsInput!) {
      updateWorkFields(workId: $id, fields: $fields) { ...WorkFull }
    }
  `);
  const lockField = graphql(`
    mutation LockWorkField($id: ID!, $field: WorkField!, $locked: Boolean!) {
      setWorkFieldLock(workId: $id, field: $field, locked: $locked) { ...WorkFull }
    }
  `);
  const deleteWork = graphql(`
    mutation DeleteWork($id: ID!, $files: Boolean!) { deleteWork(id: $id, deleteFile: $files) }
  `);
  const deleteEdition = graphql(`
    mutation DeleteEdition($id: ID!, $files: Boolean!) { deleteEdition(id: $id, deleteFile: $files) }
  `);
  const deleteMedia = graphql(`
    mutation DeleteMediaFile($id: ID!, $files: Boolean!) { deleteMediaFile(id: $id, deleteFile: $files) }
  `);

  const FIELDS = [["title", "Title"], ["sortTitle", "Sort title"], ["originalTitle", "Original title"], ["year", "Year"], ["releaseDate", "Release date"], ["communityRating", "Rating"], ["tagline", "Tagline"]];
  const LOCKS = ["TITLE", "SORT_TITLE", "ORIGINAL_TITLE", "YEAR", "RELEASE_DATE", "OVERVIEW", "TAGLINE", "COMMUNITY_RATING", "GENRES", "STUDIOS", "PEOPLE", "PARENTAL_RATING"];

  let query = $state("");
  let year = $state("");
  let matches = $state([]);
  let credits = $state(new Map());
  let searching = $state(false);
  let busy = $state("");

  async function search(event) {
    event.preventDefault();
    searching = true;
    const result = await attempt(() => request(metadataSearch, { query: query || work.title, kind: work.kind, year: year ? Number(year) : work.year }));
    matches = result?.searchMetadata ?? [];
    credits = new Map((result?.providerAttributions ?? []).map((credit) => [credit.provider, credit]));
    searching = false;
  }

  async function identify(match) {
    busy = "identify";
    const result = await attempt(() => identifyWork.mutate({ id: work.id, provider: match.provider, kind: match.externalId.kind, value: match.externalId.value }), `Matched to “${match.title}”`);
    busy = "";
    if (result) matches = [];
  }

  async function refresh(force) {
    busy = force ? "force" : "refresh";
    await attempt(() => refreshMetadata.mutate({ id: work.id, force }), "Metadata refreshed");
    busy = "";
  }

  async function save(event) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const fields = {};
    for (const [key, value] of form.entries()) {
      const original = key === "overview" ? work.overview : work[key];
      if (value === "" || String(original ?? "") === value) continue;
      fields[key] = key === "year" || key === "communityRating" ? Number(value) : value;
    }
    if (!Object.keys(fields).length) return;
    busy = "save";
    await attempt(() => updateWork.mutate({ id: work.id, fields }), "Saved — edited fields are now locked");
    busy = "";
  }

  async function toggleLock(field) {
    await attempt(() => lockField.mutate({ id: work.id, field, locked: !work.lockedFields.includes(field) }));
  }

  async function remove(kind) {
    const label = { work: `“${work.title}”`, edition: `the “${edition?.name}” edition`, media: `part ${(media?.partIndex ?? 0) + 1} of this edition` }[kind];
    const answer = await ask({ title: `Delete ${label}?`, message: "Database-only removals reappear on the next scan while the files stay on disk.", option: "Also delete the files from disk", confirmLabel: "Delete", danger: true });
    if (!answer.confirmed) return;
    const [mutation, id] = { work: [deleteWork, work.id], edition: [deleteEdition, edition?.id], media: [deleteMedia, media?.id] }[kind];
    const result = await attempt(() => mutation.mutate({ id, files: answer.option }), "Deleted");
    if (!result) return;
    open = false;
    if (kind === "work") {
      await goto(work.parentId ? `/works/${work.parentId}` : `/libraries/${work.libraryId}`, { replaceState: true });
      cache.get("Work", { id }).delete();
    } else onremoved();
  }
</script>

<Dialog bind:open title="Manage “{work.title}”" description="Metadata, matching and files. Changes apply for everyone on this server." variant="sheet" width={560}>
  <section>
    <h3><MagnifyingGlass size={16} />Identify</h3>
    <form class="identify" onsubmit={search}>
      <label class="field">Title<input bind:value={query} placeholder={work.title} /></label>
      <label class="field">Year<input bind:value={year} type="number" placeholder={work.year ?? "Any"} /></label>
      <button class="btn outline" disabled={searching}>{#if searching}<span class="spinner sm"></span>{/if}Search</button>
    </form>
    {#if matches.length}
      <ul class="matches stagger">
        {#each matches as match, index}
          {@const credit = credits.get(match.provider)}
          <li style:--i={index}>
            <button onclick={() => identify(match)} disabled={busy === "identify"}>
              <span class="match"><strong>{match.title}</strong><small>{#if credit?.logoUrl}<ProviderLogo {credit} height={16} />{:else}{credit?.name ?? match.provider} ·{/if} {match.externalId.value}</small></span>
              <span class="conf"><span>{match.year ?? "—"}</span><span class="pct" style:--p={match.confidence}>{Math.round(match.confidence * 100)}%</span></span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <div class="row">
      <button class="btn sm" onclick={() => refresh(false)} disabled={!!busy}>{#if busy === "refresh"}<span class="spinner sm"></span>{:else}<ArrowsClockwise size={15} />{/if}Refresh metadata</button>
      <button class="btn sm ghost" onclick={() => refresh(true)} disabled={!!busy}>{#if busy === "force"}<span class="spinner sm"></span>{/if}Force full refresh</button>
    </div>
  </section>

  <section>
    <h3><PencilSimple size={16} />Edit details</h3>
    {#key work}
      <form class="fields" onsubmit={save}>
        {#each FIELDS as [key, label]}
          <label class="field">{label}{#if work.lockedFields.includes(key.replace(/[A-Z]/g, (c) => `_${c}`).toUpperCase())}<small><Lock size={11} /> locked</small>{/if}<input name={key} value={work[key] ?? ""} inputmode={key === "year" || key === "communityRating" ? "decimal" : undefined} /></label>
        {/each}
        <label class="field full">Overview<textarea name="overview" rows="5">{work.overview ?? ""}</textarea></label>
        <div class="form-actions full"><button class="btn primary sm" disabled={busy === "save"}>{#if busy === "save"}<span class="spinner sm"></span>{/if}Save changes</button></div>
      </form>
    {/key}
  </section>

  <section>
    <h3><Lock size={16} />Field locks</h3>
    <p class="hint">Locked fields are never overwritten by provider refreshes.</p>
    <div class="locks">
      {#each LOCKS as field}
        <button class:on={work.lockedFields.includes(field)} aria-pressed={work.lockedFields.includes(field)} onclick={() => toggleLock(field)}>
          {#if work.lockedFields.includes(field)}<Lock size={13} />{:else}<LockOpen size={13} />{/if}{titleCase(field)}
        </button>
      {/each}
    </div>
  </section>

  <section class="danger">
    <h3><Trash size={16} />Remove</h3>
    <div class="row">
      <button class="btn danger sm" onclick={() => remove("work")}>Delete title</button>
      {#if edition}<button class="btn danger sm" onclick={() => remove("edition")}>Delete edition</button>{/if}
      {#if media}<button class="btn danger sm" onclick={() => remove("media")}>Delete media file</button>{/if}
    </div>
  </section>
</Dialog>

<style>
  section { display: grid; gap: 14px; padding: 22px 0; border-top: 1px solid var(--line); }
  section:first-child { border-top: 0; padding-top: 4px; }
  h3 { display: flex; align-items: center; gap: 9px; font-size: 0.95rem; }
  h3 :global(svg) { color: var(--text-3); }
  .identify { display: grid; grid-template-columns: 1fr 96px auto; gap: 10px; align-items: end; }
  .matches { display: grid; gap: 4px; }
  .matches button { display: flex; justify-content: space-between; align-items: center; gap: 12px; width: 100%; padding: 11px 13px; border-radius: 12px; background: var(--surface); text-align: left; transition: background var(--t); }
  .matches button:hover { background: var(--surface-3); }
  .matches span:first-child { display: grid; }
  .matches small { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 8px; margin-top: 2px; color: var(--text-3); font-size: 0.8rem; }
  .conf { display: flex; gap: 12px; align-items: center; color: var(--text-2); font-size: 0.86rem; }
  .pct { padding: 2px 8px; border-radius: 99px; font-weight: 650; background: color-mix(in srgb, var(--accent) calc(var(--p) * 30%), transparent); color: var(--text); }
  .fields { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .fields .full { grid-column: 1 / -1; }
  .field small { display: inline-flex; align-items: center; gap: 4px; color: var(--warning); margin-left: 8px; }
  .hint { color: var(--text-3); font-size: 0.86rem; margin-top: -6px; }
  .locks { display: flex; flex-wrap: wrap; gap: 6px; }
  .locks button { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 11px; border-radius: 99px; border: 1px solid var(--line-strong); font-size: 0.8rem; font-weight: 550; color: var(--text-2); transition: all var(--t); }
  .locks button.on { background: var(--accent-soft); color: var(--accent); border-color: transparent; }
  @media (max-width: 520px) { .identify, .fields { grid-template-columns: 1fr; } }
</style>
