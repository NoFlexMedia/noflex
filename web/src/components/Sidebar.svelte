<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import BookOpen from "phosphor-svelte/lib/BookOpen";
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import CardsThree from "phosphor-svelte/lib/CardsThree";
  import FilmStrip from "phosphor-svelte/lib/FilmStrip";
  import House from "phosphor-svelte/lib/House";
  import ImageIcon from "phosphor-svelte/lib/ImageIcon";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import MusicNotes from "phosphor-svelte/lib/MusicNotes";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import Stack from "phosphor-svelte/lib/Stack";
  import Television from "phosphor-svelte/lib/Television";
  import { page } from "$app/state";
  import { availableCategories } from "$lib/categories.js";
  import { connectedServers, loadServerLibraries, servers } from "$lib/servers.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { poll } from "$lib/ui.svelte.js";
  import BrandMark from "$components/BrandMark.svelte";
  import ServerStatus from "$components/ServerStatus.svelte";
  import Tip from "$components/ui/Tip.svelte";

  const LIBRARY_ICON = { MOVIES: FilmStrip, SHOWS: Television, MUSIC: MusicNotes, AUDIOBOOKS: BookOpen, PHOTOS: ImageIcon, MIXED: Stack };
  const primary = [
    { href: "/", label: "Home", icon: House },
    { href: "/search", label: "Search", icon: MagnifyingGlass },
  ];
  const secondary = [
    { href: "/collections", label: "Collections", icon: CardsThree },
    { href: "/live-tv", label: "Live TV", icon: Television },
    { href: "/watch-rooms", label: "Watch together", icon: Broadcast },
  ];
  let categories = $derived(availableCategories(app.categories).map((category) => ({ href: `/browse/${category.slug}`, label: category.label, icon: category.icon })));

  const LINKED_LIBRARIES_MS = 5 * 60_000;
  // Not only for the list below: the palette and the libraries page read these too.
  $effect(() => {
    const connected = connectedServers();
    if (!connected.length) return;
    loadServerLibraries(connected);
    return poll(() => loadServerLibraries(connected), LINKED_LIBRARIES_MS);
  });
  const libraryEntry = (library, server) => ({
    href: server ? `/libraries/${library.id}?server=${server.id}` : `/libraries/${library.id}`,
    label: library.name,
    tip: server ? `${library.name} · ${server.name}` : library.name,
    icon: LIBRARY_ICON[library.kind] ?? Books,
    // Loading a library switches to its server, which hovering mustn't do.
    noPreload: !!server || !!servers.active,
  });
  let libraryGroups = $derived.by(() => {
    const linked = connectedServers().flatMap((server) => {
      const answer = servers.libraries[server.id];
      return answer ? [{ key: server.id, heading: server.name, failed: answer.failed, entries: answer.items.map((library) => libraryEntry(library, server)) }] : [];
    });
    const own = app.libraries.length ? [{ key: "own", heading: linked.length ? app.server?.name ?? "This server" : "Libraries", failed: false, entries: app.libraries.map((library) => libraryEntry(library, null)) }] : [];
    return [...own, ...linked];
  });
  const isActive = (href) => (href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href.split("?")[0]));
</script>

{#snippet item(entry)}
  <Tip text={entry.tip ?? entry.label} side="right" contentClass="rail-tip">
    {#snippet children(props)}
      <a {...props} href={entry.href} class="item" data-sveltekit-preload-data={entry.noPreload ? "off" : undefined} aria-current={isActive(entry.href) ? "page" : undefined}>
        <span class="glyph"><entry.icon size={19} /></span>
        <span class="label">{entry.label}</span>
      </a>
    {/snippet}
  </Tip>
{/snippet}

<aside class="sidebar" aria-label="Primary">
  <a class="brand" href="/" aria-label="NoFlex home">
    <span class="mark"><BrandMark /></span>
    <span class="wordmark">NoFlex</span>
  </a>
  <nav>
    {#each [...primary, ...categories, ...secondary] as entry (entry.href)}{@render item(entry)}{/each}
    {#if app.showLibraries}
      {#each libraryGroups as group (group.key)}
        <p class="heading"><span class="label" title={group.heading}>{group.heading}</span></p>
        {#if group.failed}<p class="away"><span class="label">Can’t reach this server</span></p>{/if}
        {#each group.entries as entry (entry.href)}{@render item(entry)}{/each}
      {/each}
    {/if}
  </nav>
  <div class="foot">
    {#if app.viewer?.isAdmin}{@render item({ href: "/admin", label: "Administration", icon: ShieldCheck })}{/if}
    <ServerStatus />
  </div>
</aside>

<style>
  /* Insets that center the brand mark, icons, and status dot on the
     rail's axis, so collapsing only clips and fades — nothing moves. */
  .sidebar { --inset: calc((var(--rail-w) - 24px - 19px) / 2); position: fixed; z-index: 40; inset: 0 auto 0 0; width: var(--nav-w); display: flex; flex-direction: column; padding: 18px 12px 14px; overflow: hidden; contain: layout style; background: linear-gradient(180deg, rgb(255 255 255 / 0.018), transparent 40%), rgb(8 9 12 / 0.82); backdrop-filter: blur(24px) saturate(1.3); border-right: 1px solid var(--line); view-transition-name: chrome; }
  :global(:root[data-sidebar-motion]) .sidebar { transition: width var(--nav-t) var(--ease-out); }
  .brand { display: flex; align-items: center; gap: 11px; padding: 4px 10px 18px calc((var(--rail-w) - 24px - 30px) / 2); color: var(--accent); }
  .mark { display: grid; flex: none; transition: transform 500ms var(--ease-spring); }
  .brand:hover .mark { transform: rotate(-8deg) scale(1.06); }
  .wordmark { font-weight: 750; font-size: 1.16rem; letter-spacing: -0.03em; color: var(--text); white-space: nowrap; }
  nav { flex: 1; display: grid; align-content: start; gap: 2px; overflow-y: auto; margin: 0 -4px; padding: 0 4px; scrollbar-width: none; }
  .heading { position: relative; padding: 18px 12px 6px; font-size: 0.7rem; font-weight: 650; letter-spacing: 0.13em; text-transform: uppercase; color: var(--text-3); white-space: nowrap; }
  .heading::after { content: ""; position: absolute; left: 10px; right: 10px; top: calc(50% + 6px); height: 1px; background: var(--line); opacity: 0; transition: opacity 160ms var(--ease-out); }
  .away { padding: 2px 12px 6px; font-size: 0.78rem; color: var(--text-3); }
  .item { position: relative; display: flex; align-items: center; gap: 12px; width: 100%; height: 40px; padding: 0 12px 0 var(--inset); border-radius: 11px; color: var(--text-2); font-weight: 550; font-size: 0.92rem; transition: background var(--t) var(--ease-out), color var(--t); }
  .item:hover { background: var(--surface-2); color: var(--text); }
  .item[aria-current="page"] { background: var(--surface-3); color: var(--text); }
  .item[aria-current="page"]::before { content: ""; position: absolute; left: -12px; top: 10px; bottom: 10px; width: 3px; border-radius: 0 3px 3px 0; background: var(--accent); animation: grow 300ms var(--ease-spring); }
  @keyframes grow { from { transform: scaleY(0); } }
  .item[aria-current="page"] .glyph { color: var(--accent); }
  .glyph { display: grid; flex: none; transition: transform var(--t) var(--ease-spring); }
  .item:active .glyph { transform: scale(0.88); }
  .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Labels return once the sidebar has opened enough to hold them, and leave at once. */
  .label, .wordmark { transition: opacity 180ms var(--ease-out) 90ms; }
  .foot { display: grid; gap: 2px; padding-top: 10px; border-top: 1px solid var(--line); }

  :global(:root[data-sidebar="collapsed"]) .label, :global(:root[data-sidebar="collapsed"]) .wordmark { opacity: 0; transition-duration: 110ms; transition-delay: 0ms; }
  :global(:root[data-sidebar="collapsed"]) .heading::after { opacity: 1; }
  @media (max-width: 759px) { .sidebar { display: none; } }
  @media (prefers-reduced-motion: reduce) { .label, .wordmark, .heading::after { transition: none; } }
</style>
