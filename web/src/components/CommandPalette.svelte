<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import Books from "phosphor-svelte/lib/Books";
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import CardsThree from "phosphor-svelte/lib/CardsThree";
  import House from "phosphor-svelte/lib/House";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import Television from "phosphor-svelte/lib/Television";
  import User from "phosphor-svelte/lib/User";
  import { goto } from "$app/navigation";
  import { Dialog } from "bits-ui";
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { graphql } from "$houdini";
  import { fetchQuery } from "$lib/graphql.js";
  import { subtitleFor } from "$lib/playback.svelte.js";
  import { connectedServers, servers } from "$lib/servers.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { titleCase } from "$lib/format.js";
  import Artwork from "$components/Artwork.svelte";

  let { open = $bindable(false) } = $props();
  let text = $state("");
  let active = $state(0);
  let list = $state();

  // A newer keystroke supersedes the search in flight.
  const quickSearch = graphql(`
    query QuickSearch($text: String!) @dedupe(cancelFirst: true, match: Operation) {
      search(text: $text, mode: HYBRID, first: 8) { nodes { id work { ...WorkCard } } }
    }
  `);
  let query = $derived(text.trim());
  let results = $derived(query.length >= 2 ? ($quickSearch.data?.search.nodes ?? []).filter((hit) => hit.work).slice(0, 7) : []);
  let loading = $derived(query.length >= 2 && ($quickSearch.fetching || $quickSearch.variables?.text !== query));

  let pages = $derived([
    { label: "Home", href: "/", icon: House },
    { label: "Search everything", href: "/search", icon: MagnifyingGlass },
    { label: "Collections & playlists", href: "/collections", icon: CardsThree },
    { label: "Live TV", href: "/live-tv", icon: Television },
    { label: "Watch together", href: "/watch-rooms", icon: Broadcast },
    { label: "Account", href: "/account", icon: User },
    ...(app.viewer?.isAdmin ? [{ label: "Administration", href: "/admin", icon: ShieldCheck }] : []),
    ...app.libraries.map((library) => ({ label: library.name, href: `/libraries/${library.id}`, icon: Books, hint: titleCase(library.kind) })),
    ...connectedServers().flatMap((server) => (servers.libraries[server.id]?.items ?? []).map((library) => ({ label: library.name, href: `/libraries/${library.id}?server=${server.id}`, icon: Books, hint: `${titleCase(library.kind)} · ${server.name}` }))),
  ]);
  let commands = $derived(text.trim() ? pages.filter((page) => page.label.toLowerCase().includes(text.trim().toLowerCase())).slice(0, 4) : pages.slice(0, 6));
  let entries = $derived([
    ...results.map((hit) => ({ kind: "work", work: hit.work, href: `/works/${hit.work.id}` })),
    ...commands.map((page) => ({ kind: "page", ...page })),
  ]);

  $effect(() => {
    const next = query;
    active = 0;
    if (next.length < 2) return;
    const timer = setTimeout(() => { fetchQuery(quickSearch, { text: next }); }, 160);
    return () => clearTimeout(timer);
  });

  $effect(() => { if (!open) text = ""; });

  function choose(entry) {
    if (!entry) {
      if (text.trim()) { open = false; goto(`/search?q=${encodeURIComponent(text.trim())}`); }
      return;
    }
    open = false;
    goto(entry.href);
  }

  function key(event) {
    if (event.key === "ArrowDown") { event.preventDefault(); active = (active + 1) % Math.max(1, entries.length); scrollActive(); }
    else if (event.key === "ArrowUp") { event.preventDefault(); active = (active - 1 + entries.length) % Math.max(1, entries.length); scrollActive(); }
    else if (event.key === "Enter") { event.preventDefault(); choose(event.metaKey || event.ctrlKey ? null : entries[active]); }
  }
  function scrollActive() {
    requestAnimationFrame(() => list?.querySelector('[data-active="true"]')?.scrollIntoView({ block: "nearest" }));
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Portal>
    <Dialog.Overlay forceMount>
      {#snippet child({ props, open })}{#if open}<div {...props} class="overlay" transition:fade={{ duration: 160 }}></div>{/if}{/snippet}
    </Dialog.Overlay>
    <Dialog.Content forceMount>
      {#snippet child({ props, open })}
        {#if open}
          <div {...props} class="palette" transition:scale={{ start: 0.97, duration: 200, easing: cubicOut }}>
            <Dialog.Title class="sr-only">Search and jump</Dialog.Title>
            <div class="input">
              <MagnifyingGlass size={20} />
              <!-- svelte-ignore a11y_autofocus -->
              <input bind:value={text} onkeydown={key} placeholder="Search titles, people, moods — or jump anywhere" autocomplete="off" spellcheck="false" autofocus aria-controls="palette-list" aria-activedescendant="palette-{active}" role="combobox" aria-expanded="true" />
              {#if loading}<span class="spinner sm"></span>{:else}<kbd>esc</kbd>{/if}
            </div>
            <ul id="palette-list" role="listbox" bind:this={list}>
              {#if results.length}<li class="group" role="presentation">Titles</li>{/if}
              {#each entries as entry, index (entry.href + index)}
                {#if entry.kind === "page" && index === results.length}<li class="group" role="presentation">{text.trim() ? "Go to" : "Jump to"}</li>{/if}
                <li id="palette-{index}" role="option" aria-selected={index === active} data-active={index === active}>
                  <a href={entry.href} onclick={(event) => { event.preventDefault(); choose(entry); }} onpointermove={() => (active = index)}>
                    {#if entry.kind === "work"}
                      <span class="thumb"><Artwork work={entry.work} width={36} /></span>
                      <span class="text"><strong>{entry.work.title}</strong><small>{[subtitleFor(entry.work), titleCase(entry.work.kind)].filter(Boolean).join(" · ")}</small></span>
                    {:else}
                      <span class="icon"><entry.icon size={18} /></span>
                      <span class="text"><strong>{entry.label}</strong>{#if entry.hint}<small>{entry.hint}</small>{/if}</span>
                    {/if}
                    <ArrowRight size={16} />
                  </a>
                </li>
              {/each}
              {#if text.trim().length >= 2 && !loading && !results.length}<li class="none" role="presentation">No titles match “{text.trim()}”.</li>{/if}
            </ul>
            <footer><span><kbd>↑</kbd><kbd>↓</kbd> navigate</span><span><kbd>↵</kbd> open</span><span><kbd>ctrl</kbd><kbd>↵</kbd> all results</span></footer>
          </div>
        {/if}
      {/snippet}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  .overlay { position: fixed; inset: 0; z-index: 85; background: rgb(3 4 6 / 0.6); backdrop-filter: blur(10px); }
  .palette { position: fixed; z-index: 86; left: 50%; top: min(14vh, 120px); translate: -50% 0; width: min(640px, calc(100vw - 24px)); border-radius: 22px; background: rgb(20 22 28 / 0.94); backdrop-filter: blur(24px) saturate(1.6); border: 1px solid var(--line-strong); box-shadow: var(--shadow-3), 0 0 0 1px rgb(0 0 0 / 0.4); overflow: hidden; outline: none; }
  .input { display: flex; align-items: center; gap: 12px; padding: 0 18px; height: 62px; border-bottom: 1px solid var(--line); color: var(--text-3); }
  .input input { flex: 1; height: 100%; border: 0; background: none; outline: none; font-size: 1.06rem; color: var(--text); }
  kbd { display: inline-grid; place-items: center; min-width: 22px; height: 22px; padding: 0 6px; border-radius: 6px; background: var(--surface-2); border: 1px solid var(--line); font-size: 0.72rem; color: var(--text-3); }
  ul { max-height: min(56vh, 460px); overflow-y: auto; padding: 8px; }
  .group { padding: 10px 12px 6px; font-size: 0.7rem; font-weight: 650; letter-spacing: 0.12em; text-transform: uppercase; color: var(--text-3); }
  li a { display: flex; align-items: center; gap: 13px; padding: 8px 12px; border-radius: 12px; color: var(--text-2); }
  li a > :global(svg:last-child) { opacity: 0; transform: translateX(-4px); transition: opacity var(--t-fast), transform var(--t) var(--ease-out); }
  [data-active="true"] a { background: var(--surface-3); color: var(--text); }
  [data-active="true"] a > :global(svg:last-child) { opacity: 0.7; transform: none; }
  .thumb { width: 34px; flex: none; }
  .thumb :global(.art) { border-radius: 6px; }
  .icon { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 10px; background: var(--surface-2); flex: none; }
  .text { flex: 1; min-width: 0; display: grid; }
  .text strong { font-weight: 600; font-size: 0.94rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .text small { color: var(--text-3); font-size: 0.8rem; }
  .none { padding: 14px 12px; color: var(--text-3); font-size: 0.9rem; }
  footer { display: flex; gap: 18px; padding: 10px 18px; border-top: 1px solid var(--line); font-size: 0.76rem; color: var(--text-3); }
  footer span { display: inline-flex; gap: 4px; align-items: center; }
  @media (max-width: 600px) { footer { display: none; } .palette { top: 10px; } }
</style>
