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
  import SidebarSimple from "phosphor-svelte/lib/SidebarSimple";
  import Stack from "phosphor-svelte/lib/Stack";
  import Television from "phosphor-svelte/lib/Television";
  import { page } from "$app/state";
  import { app } from "$lib/session.svelte.js";
  import { sidebar, toggleSidebar } from "$lib/ui.svelte.js";
  import BrandMark from "$components/BrandMark.svelte";
  import Tip from "$components/ui/Tip.svelte";

  const LIBRARY_ICON = { MOVIES: FilmStrip, SHOWS: Television, MUSIC: MusicNotes, AUDIOBOOKS: BookOpen, PHOTOS: ImageIcon, MIXED: Stack };
  const primary = [
    { href: "/", label: "Home", icon: House },
    { href: "/search", label: "Search", icon: MagnifyingGlass },
    { href: "/collections", label: "Collections", icon: CardsThree },
    { href: "/live-tv", label: "Live TV", icon: Television },
    { href: "/watch-rooms", label: "Watch together", icon: Broadcast },
  ];
  const isActive = (href) => (href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href));
</script>

{#snippet item(entry)}
  <Tip text={entry.label} side="right" contentClass="rail-tip">
    {#snippet children(props)}
      <a {...props} href={entry.href} class="item" aria-current={isActive(entry.href) ? "page" : undefined}>
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
    {#each primary as entry}{@render item(entry)}{/each}
    {#if app.libraries.length}
      <p class="heading"><span class="label">Libraries</span></p>
      {#each app.libraries as library (library.id)}
        {@render item({ href: `/libraries/${library.id}`, label: library.name, icon: LIBRARY_ICON[library.kind] ?? Books })}
      {/each}
    {/if}
  </nav>
  <div class="foot">
    {#if app.viewer?.isAdmin}{@render item({ href: "/admin", label: "Administration", icon: ShieldCheck })}{/if}
    <Tip text={sidebar.collapsed ? "Expand sidebar" : "Collapse sidebar"} hint="[" side="right">
      {#snippet children(props)}
        <button {...props} type="button" class="item toggle" onclick={toggleSidebar} aria-expanded={!sidebar.collapsed} aria-label={sidebar.collapsed ? "Expand sidebar" : "Collapse sidebar"}>
          <span class="glyph"><SidebarSimple size={19} /></span>
          <span class="label">Collapse</span>
        </button>
      {/snippet}
    </Tip>
    <div class="server" title="{app.server?.name ?? 'NoFlex'} · v{app.server?.version}">
      <span class="dot on"></span>
      <span class="label"><strong>{app.server?.name ?? "NoFlex"}</strong><small>v{app.server?.version} · {app.server?.backend}</small></span>
    </div>
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
  .item { position: relative; display: flex; align-items: center; gap: 12px; width: 100%; height: 40px; padding: 0 12px 0 var(--inset); border: 0; border-radius: 11px; background: none; font: inherit; text-align: left; cursor: pointer; color: var(--text-2); font-weight: 550; font-size: 0.92rem; transition: background var(--t) var(--ease-out), color var(--t); }
  .item:hover { background: var(--surface-2); color: var(--text); }
  .item[aria-current="page"] { background: var(--surface-3); color: var(--text); }
  .item[aria-current="page"]::before { content: ""; position: absolute; left: -12px; top: 10px; bottom: 10px; width: 3px; border-radius: 0 3px 3px 0; background: var(--accent); animation: grow 300ms var(--ease-spring); }
  @keyframes grow { from { transform: scaleY(0); } }
  .item[aria-current="page"] .glyph { color: var(--accent); }
  .glyph { display: grid; flex: none; transition: transform var(--t) var(--ease-spring); }
  .item:active .glyph { transform: scale(0.88); }
  .toggle .glyph { color: var(--text-3); }
  .toggle:hover .glyph { color: var(--text); }
  .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Labels return once the sidebar has opened enough to hold them, and leave at once. */
  .label, .wordmark { transition: opacity 180ms var(--ease-out) 90ms; }
  .foot { display: grid; gap: 2px; padding-top: 10px; border-top: 1px solid var(--line); }
  .server { display: flex; align-items: center; gap: 12px; padding: 10px 14px 4px calc((var(--rail-w) - 24px - 8px) / 2); }
  .server .label { display: grid; }
  .server strong { font-size: 0.84rem; font-weight: 600; }
  .server small { font-size: 0.74rem; color: var(--text-3); }

  @media (min-width: 1180px) {
    :global(:root[data-sidebar="collapsed"]) .label, :global(:root[data-sidebar="collapsed"]) .wordmark { opacity: 0; transition-duration: 110ms; transition-delay: 0ms; }
    :global(:root[data-sidebar="collapsed"]) .heading::after { opacity: 1; }
  }
  @media (max-width: 1179px) {
    .sidebar { padding-inline: 10px; }
    .wordmark, .label, .server .label, .heading, .toggle { display: none; }
    .brand { justify-content: center; padding-inline: 0; }
    .item { justify-content: center; padding: 0; height: 46px; }
    .item[aria-current="page"]::before { left: -10px; }
    .heading { display: block; height: 1px; padding: 0; margin: 12px 10px; background: var(--line); font-size: 0; }
    .heading::after { display: none; }
    .server { justify-content: center; padding-inline: 0; }
  }
  @media (max-width: 759px) { .sidebar { display: none; } }
  @media (prefers-reduced-motion: reduce) { .label, .wordmark, .heading::after { transition: none; } }
</style>
