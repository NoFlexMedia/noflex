<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import Plus from "phosphor-svelte/lib/Plus";
  import Trash from "phosphor-svelte/lib/Trash";
  import { cache, graphql, LiveTvSourcesStore } from "$houdini";
  import { date } from "$lib/format.js";
  import { failure, fetchQuery, refetch } from "$lib/graphql.js";
  import { attempt, confirmAction } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import Dialog from "$components/ui/Dialog.svelte";
  import { ago } from "$components/account/relative.js";

  let { onchanged } = $props();

  const sources = new LiveTvSourcesStore();
  $effect(() => { fetchQuery(sources); });
  let list = $derived([...($sources.data?.liveTvSources ?? [])].sort((a, b) => a.name.localeCompare(b.name)));
  let busy = $state({});

  const createSource = graphql(`
    mutation CreateLiveTvSource($input: CreateLiveTvSourceInput!) {
      createLiveTvSource(input: $input) { ...LiveTvSourceFields ...LiveTvSources_insert }
    }
  `);
  const updateSource = graphql(`
    mutation UpdateLiveTvSource($id: ID!, $input: UpdateLiveTvSourceInput!) {
      updateLiveTvSource(id: $id, input: $input) { ...LiveTvSourceFields }
    }
  `);
  const refreshSource = graphql(`
    mutation RefreshLiveTvSource($id: ID!) { refreshLiveTvSource(id: $id) { ...LiveTvSourceFields } }
  `);
  const deleteSource = graphql(`
    mutation DeleteLiveTvSource($id: ID!) { deleteLiveTvSource(id: $id) }
  `);

  async function refresh(source) {
    busy[source.id] = "refresh";
    const result = await attempt(() => refreshSource.mutate({ id: source.id }));
    busy[source.id] = "";
    if (!result || result.data.refreshLiveTvSource.lastError) return;
    onchanged();
  }

  async function toggle(source, input) {
    const enabled = input.checked;
    const result = await attempt(() => updateSource.mutate({ id: source.id, input: { enabled } }), enabled ? `${source.name} enabled` : `${source.name} disabled`);
    if (!result) { input.checked = !enabled; return; }
    onchanged();
  }

  async function remove(source) {
    if (!(await confirmAction({ title: `Delete “${source.name}”?`, message: "Its channels, guide data and schedules on those channels go with it.", confirmLabel: "Delete source", danger: true }))) return;
    if (!(await attempt(() => deleteSource.mutate({ id: source.id }), `${source.name} deleted`))) return;
    cache.get("LiveTvSource", { id: source.id }).delete();
    onchanged();
  }

  let draft = $state({ open: false, name: "", playlistUrl: "", epgUrl: "" });
  let saving = $state(false);

  async function create(event) {
    event.preventDefault();
    saving = true;
    const result = await attempt(() => createSource.mutate({ input: { name: draft.name.trim(), playlistUrl: draft.playlistUrl.trim(), epgUrl: draft.epgUrl.trim() || null, enabled: true } }), `${draft.name.trim()} added`);
    saving = false;
    if (!result) return;
    draft.open = false;
    // A fresh source has no channels until its playlist is fetched.
    refresh(result.data.createLiveTvSource);
  }

  const openDraft = () => (draft = { open: true, name: "", playlistUrl: "", epgUrl: "" });
  const tone = (source) => (!source.enabled ? "" : source.lastError ? "bad" : source.lastRefreshAt ? "on" : "warn");
</script>

{#if $sources.fetching && !$sources.data}
  <div class="panel" aria-busy="true">{#each Array(3) as _}<div class="skeleton line"></div>{/each}</div>
{:else if failure($sources) && !$sources.data}
  <State kind="error" title="Couldn’t load tuner sources" message={failure($sources)} action={() => refetch(sources)} />
{:else if !list.length}
  <State icon={Broadcast} title="No sources yet" message="A source is an M3U playlist from an IPTV provider or a network tuner like HDHomeRun, optionally with an XMLTV guide.">
    <button class="btn primary sm" onclick={openDraft}><Plus size={15} />Add a source</button>
  </State>
{:else}
  <section class="panel fade-up">
    <header class="panel-head">
      <span class="panel-icon"><Broadcast size={19} /></span>
      <div><h2>Tuner sources</h2><p>Playlists are re-read on refresh; guide data comes from each source’s XMLTV feed</p></div>
      <span class="spacer"></span>
      <button class="btn primary sm" onclick={openDraft}><Plus size={15} />Add</button>
    </header>
    <ul class="list">
      {#each list as source (source.id)}
        <li class:off={!source.enabled}>
          <span class="dot {tone(source)}"></span>
          <span class="what">
            <strong>{source.name}</strong>
            {#if source.lastError}
              <small class="error">{source.lastError}</small>
            {:else}
              <small title={source.lastRefreshAt ? date(source.lastRefreshAt, { dateStyle: "medium", timeStyle: "short" }) : undefined}>{source.lastRefreshAt ? `Refreshed ${ago(source.lastRefreshAt)}` : "Never refreshed"}{source.enabled ? "" : " · disabled"}</small>
            {/if}
          </span>
          <button class="btn ghost sm" onclick={() => refresh(source)} disabled={!!busy[source.id] || !source.enabled}>{#if busy[source.id] === "refresh"}<span class="spinner sm"></span>{:else}<ArrowsClockwise size={15} />{/if}<span class="label">Refresh</span></button>
          <input type="checkbox" class="switch" checked={source.enabled} onchange={(event) => toggle(source, event.currentTarget)} aria-label="{source.enabled ? 'Disable' : 'Enable'} {source.name}" />
          <button class="icon-btn sm" aria-label="Delete {source.name}" onclick={() => remove(source)}><Trash size={16} /></button>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<Dialog bind:open={draft.open} title="Add a tuner source" description="NoFlex fetches the playlist right away and lists its channels." width={520}>
  <form id="source-form" class="stack" onsubmit={create}>
    <label class="field">Name<input bind:value={draft.name} required placeholder="Living room HDHomeRun" autocomplete="off" /></label>
    <label class="field">M3U playlist URL<input type="url" bind:value={draft.playlistUrl} required placeholder="http://192.168.1.20/lineup.m3u" spellcheck="false" /></label>
    <label class="field">XMLTV guide URL<input type="url" bind:value={draft.epgUrl} placeholder="Optional — https://…/guide.xml" spellcheck="false" /><small>Without a guide you can still watch and record by time slot.</small></label>
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (draft.open = false)}>Cancel</button>
    <button class="btn primary" form="source-form" disabled={saving || !draft.name.trim() || !draft.playlistUrl.trim()}>{#if saving}<span class="spinner sm"></span>{/if}Add source</button>
  {/snippet}
</Dialog>

<style>
  .panel { max-width: 920px; }
  .skeleton.line { height: 44px; margin-top: 12px; border-radius: 10px; }
  .skeleton.line:first-child { margin-top: 0; }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .what small.error { color: var(--danger); white-space: normal; overflow-wrap: anywhere; }
  .off strong { color: var(--text-2); }
  @media (max-width: 520px) { .label { display: none; } }
</style>
