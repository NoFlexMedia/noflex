<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CardsThree from "phosphor-svelte/lib/CardsThree";
  import DotsThree from "phosphor-svelte/lib/DotsThree";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import PencilSimple from "phosphor-svelte/lib/PencilSimple";
  import Play from "phosphor-svelte/lib/Play";
  import Plus from "phosphor-svelte/lib/Plus";
  import Trash from "phosphor-svelte/lib/Trash";
  import X from "phosphor-svelte/lib/X";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Tabs as BitsTabs } from "bits-ui";
  import { cache, graphql } from "$houdini";
  import { removeFromCollection, removeFromPlaylist } from "$lib/collections.js";
  import { failure, fetchQuery, refetch } from "$lib/graphql.js";
  import { playWork } from "$lib/playback.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt, confirmAction } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import WorkCard from "$components/WorkCard.svelte";
  import Dialog from "$components/ui/Dialog.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Select from "$components/ui/Select.svelte";
  import Tabs from "$components/ui/Tabs.svelte";

  let tab = $derived(page.url.searchParams.get("tab") === "playlists" ? "playlists" : "collections");
  let libraryId = $state(app.libraries[0]?.id ?? "");
  const collections = graphql(`
    query Collections($library: ID!) {
      collections(libraryId: $library, first: 12) @list(name: "Collections") {
        edges { node { ...CollectionShelf } }
      }
    }
  `);
  $effect(() => { if (libraryId) fetchQuery(collections, { library: libraryId }); });
  const playlists = graphql(`
    query Playlists {
      playlists(first: 12) @list(name: "Playlists") {
        edges { node { ...PlaylistShelf } }
      }
    }
  `);
  $effect(() => { fetchQuery(playlists); });

  const createCollection = graphql(`
    mutation CreateCollection($library: ID!, $title: String!, $overview: String) {
      createCollection(libraryId: $library, title: $title, overview: $overview) { ...CollectionShelf }
    }
  `);
  const updateCollection = graphql(`
    mutation UpdateCollection($id: ID!, $title: String, $overview: String) {
      updateCollection(id: $id, title: $title, overview: $overview) { id title overview }
    }
  `);
  const deleteCollection = graphql(`
    mutation DeleteCollection($id: ID!) { deleteCollection(id: $id) { id @Collection_delete } }
  `);
  const createPlaylist = graphql(`
    mutation CreatePlaylist($title: String!, $overview: String) {
      createPlaylist(title: $title, overview: $overview) { ...Playlists_insert }
    }
  `);
  const updatePlaylist = graphql(`
    mutation UpdatePlaylist($id: ID!, $title: String, $overview: String) {
      updatePlaylist(id: $id, title: $title, overview: $overview) { id title overview }
    }
  `);
  const deletePlaylist = graphql(`
    mutation DeletePlaylist($id: ID!) { deletePlaylist(id: $id) { id @Playlist_delete } }
  `);

  const shelf = (node, entries) => ({ ...node, more: node.items.pageInfo.hasNextPage, entries });
  let collectionItems = $derived($collections.variables?.library === libraryId ? ($collections.data?.collections.edges ?? []).map(({ node }) => shelf(node, node.items.nodes.map((work) => ({ work })))) : []);
  let playlistItems = $derived(($playlists.data?.playlists.edges ?? []).map(({ node }) => shelf(node, node.items.nodes)));
  let collectionsState = $derived({ loading: $collections.fetching && !collectionItems.length, error: failure($collections), data: $collections.data, retry: () => refetch(collections) });
  let playlistsState = $derived({ loading: $playlists.fetching && !$playlists.data, error: failure($playlists), data: $playlists.data, retry: () => refetch(playlists) });
  let canEdit = $derived(tab === "playlists" || app.viewer.isAdmin);

  let editor = $state({ open: false, item: null, title: "", overview: "" });
  let saving = $state(false);

  function setTab(next) { goto(next === "playlists" ? "?tab=playlists" : "?", { replaceState: true, noScroll: true, keepFocus: true }); }
  function openEditor(item = null) { editor = { open: true, item, title: item?.title ?? "", overview: item?.overview ?? "" }; }

  async function save(event) {
    event.preventDefault();
    saving = true;
    const isCollection = tab === "collections";
    const { item, title, overview } = editor;
    const library = libraryId;
    const task = item
      ? () => (isCollection ? updateCollection : updatePlaylist).mutate({ id: item.id, title, overview: overview || null })
      : isCollection
        ? async () => {
            const { data } = await createCollection.mutate({ library, title, overview: overview || null });
            // Collection lists are per library; only that library's take the new one.
            const record = cache.get("Collection", { id: data.createCollection.id });
            for (const name of ["Collections", "CollectionMenu"]) cache.list(name).when({ must: { libraryId: library } }).append(record);
          }
        : () => createPlaylist.mutate({ title, overview: overview || null });
    const result = await attempt(task, item ? "Saved" : `${isCollection ? "Collection" : "Playlist"} created`);
    saving = false;
    if (result !== null) editor.open = false;
  }

  async function remove(item) {
    if (!(await confirmAction({ title: `Delete “${item.title}”?`, message: "The titles inside stay in your library.", confirmLabel: "Delete", danger: true }))) return;
    const isCollection = tab === "collections";
    await attempt(() => (isCollection ? deleteCollection : deletePlaylist).mutate({ id: item.id }), "Deleted");
  }

  async function removeEntry(item, index) {
    const removed = item.entries[index];
    const task = tab === "collections"
      ? () => removeFromCollection(item.id, removed.work.id)
      : () => removeFromPlaylist(item.id, removed.work?.id ?? removed.edition?.id);
    await attempt(task, "Removed");
  }
</script>

<svelte:head><title>Collections · NoFlex</title></svelte:head>

{#snippet shelves(list, query, emptyTitle, emptyMessage)}
  {#if query.loading}
    {#each [0, 1] as _}<div class="shelf" aria-hidden="true"><div class="skeleton" style:height="22px" style:width="220px"></div><div class="row-cards">{#each Array(6) as _}<div class="skeleton" style:aspect-ratio="2 / 3"></div>{/each}</div></div>{/each}
  {:else if query.error && !query.data}
    <State kind="error" title="Couldn’t load these lists" message={query.error} action={query.retry} />
  {:else if !list.length}
    <State icon={CardsThree} title={emptyTitle} message={emptyMessage}>{#if canEdit}<button class="btn primary sm" onclick={() => openEditor()}><Plus size={15} />Create one</button>{/if}</State>
  {:else}
    <div class="stagger">
      {#each list as item, index (item.id)}
        <section class="shelf" style:--i={index}>
          <header>
            <div>
              <h2>{item.title}</h2>
              <p>{item.overview || `${item.entries.length}${item.more ? "+" : ""} ${item.entries.length === 1 && !item.more ? "title" : "titles"}`}</p>
            </div>
            <div class="row">
              {#if item.entries[0]?.work}<button class="btn sm" onclick={() => playWork(item.entries[0].work)}><Play size={14} weight="fill" />Play</button>{/if}
              {#if canEdit}
                <Menu>
                  {#snippet trigger(props)}<button {...props} class="icon-btn sm" aria-label="Manage {item.title}"><DotsThree size={18} weight="bold" /></button>{/snippet}
                  <MenuItem icon={PencilSimple} onSelect={() => openEditor(item)}>Rename or describe</MenuItem>
                  <MenuItem icon={Trash} danger onSelect={() => remove(item)}>Delete</MenuItem>
                </Menu>
              {/if}
            </div>
          </header>
          {#if item.entries.length}
            <div class="row-cards">
              {#each item.entries as entry, position (position)}
                <div class="entry">
                  {#if entry.work}<WorkCard work={entry.work} key="{item.id}:{position}" />
                  {:else if entry.edition}<a class="edition" href="/works/{entry.edition.workId}"><FilmStrip size={22} /><strong>{entry.edition.name}</strong><small>Edition</small></a>{/if}
                  {#if canEdit}<button class="remove" aria-label="Remove from {item.title}" onclick={() => removeEntry(item, position)}><X size={14} /></button>{/if}
                </div>
              {/each}
            </div>
          {:else}
            <p class="empty-note">Empty for now — add titles from their detail page’s ••• menu.</p>
          {/if}
        </section>
      {/each}
    </div>
  {/if}
{/snippet}

<div class="page">
  <header class="page-head">
    <div><p class="eyebrow">Curated by people, not feeds</p><h1>Collections</h1></div>
    <div class="actions">
      {#if tab === "collections" && app.libraries.length > 1}<Select pill bind:value={libraryId} items={app.libraries.map((library) => ({ value: library.id, label: library.name }))} label="Library" />{/if}
      {#if canEdit}<button class="btn primary" onclick={() => openEditor()}><Plus size={16} />New {tab === "collections" ? "collection" : "playlist"}</button>{/if}
    </div>
  </header>

  <Tabs value={tab} onchange={setTab} label="List type" items={[{ value: "collections", label: "Library collections" }, { value: "playlists", label: "My playlists" }]}>
    <BitsTabs.Content value="collections" class="panel-body">
      {@render shelves(collectionItems, collectionsState, "No collections in this library", app.viewer.isAdmin ? "Group related titles — a franchise, a director, a mood — into a shelf everyone can browse." : "An administrator hasn’t curated any collections here yet.")}
    </BitsTabs.Content>
    <BitsTabs.Content value="playlists" class="panel-body">
      {@render shelves(playlistItems, playlistsState, "No playlists yet", "Make a personal queue — then add titles from their detail page.")}
    </BitsTabs.Content>
  </Tabs>
</div>

<Dialog bind:open={editor.open} title={editor.item ? `Edit “${editor.item.title}”` : `New ${tab === "collections" ? "collection" : "playlist"}`} width={480}>
  <form id="list-editor" class="stack" onsubmit={save}>
    <label class="field">Title<input bind:value={editor.title} required placeholder={tab === "collections" ? "The Criterion shelf" : "Sunday night"} /></label>
    <label class="field">Description<textarea bind:value={editor.overview} rows="3" placeholder="Optional"></textarea></label>
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (editor.open = false)}>Cancel</button>
    <button class="btn primary" form="list-editor" disabled={saving || !editor.title.trim()}>{#if saving}<span class="spinner sm"></span>{/if}{editor.item ? "Save" : "Create"}</button>
  {/snippet}
</Dialog>

<style>
  .page :global(.panel-body) { margin-top: 28px; }
  .shelf { margin-bottom: 44px; }
  .shelf header { display: flex; justify-content: space-between; align-items: flex-end; gap: 16px; margin-bottom: 14px; }
  .shelf h2 { font-size: 1.2rem; }
  .shelf p { color: var(--text-3); font-size: 0.88rem; margin-top: 2px; }
  .row-cards { display: grid; grid-auto-flow: column; grid-auto-columns: clamp(130px, 13vw, 170px); gap: 16px; overflow-x: auto; padding: 6px 2px 12px; scrollbar-width: none; }
  .entry { position: relative; }
  .remove { position: absolute; top: -6px; right: -6px; z-index: 2; display: grid; place-items: center; width: 26px; height: 26px; border-radius: 50%; background: #23262d; border: 1px solid var(--line-strong); color: var(--text-2); opacity: 0; transform: scale(0.7); transition: opacity var(--t), transform 300ms var(--ease-spring), background var(--t); }
  .entry:hover .remove, .remove:focus-visible { opacity: 1; transform: none; }
  .remove:hover { background: var(--danger); color: #fff; }
  .edition { display: grid; place-content: center; justify-items: center; gap: 6px; aspect-ratio: 2 / 3; border-radius: var(--r); background: var(--surface-2); text-align: center; padding: 12px; color: var(--text-2); }
  .edition small { color: var(--text-3); }
  @media (hover: none) { .remove { opacity: 1; transform: none; } }
</style>
