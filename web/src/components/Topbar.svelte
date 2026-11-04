<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import CaretDown from "phosphor-svelte/lib/CaretDown";
  import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
  import Devices from "phosphor-svelte/lib/Devices";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import SidebarSimple from "phosphor-svelte/lib/SidebarSimple";
  import SignOut from "phosphor-svelte/lib/SignOut";
  import User from "phosphor-svelte/lib/User";
  import { app, logout } from "$lib/session.svelte.js";
  import { palette, sidebar, toggleSidebar } from "$lib/ui.svelte.js";
  import { titleCase } from "$lib/format.js";
  import BrandMark from "$components/BrandMark.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Tip from "$components/ui/Tip.svelte";

  let scrolled = $state(false);
  const mac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
</script>

<svelte:window onscroll={() => (scrolled = scrollY > 8)} />

<header class="topbar" class:scrolled>
  <a class="mobile-brand" href="/" aria-label="NoFlex home"><BrandMark size={28} /></a>
  <span class="sidebar-toggle">
    <Tip text={sidebar.collapsed ? "Expand sidebar" : "Collapse sidebar"} hint="[">
      {#snippet children(props)}
        <button {...props} type="button" class="icon-btn" onclick={toggleSidebar} aria-expanded={!sidebar.collapsed} aria-label={sidebar.collapsed ? "Expand sidebar" : "Collapse sidebar"}><SidebarSimple size={19} /></button>
      {/snippet}
    </Tip>
  </span>
  <button class="search" type="button" onclick={() => (palette.open = true)}>
    <MagnifyingGlass size={17} />
    <span>Search or jump to…</span>
    <kbd>{mac ? "⌘" : "Ctrl"} K</kbd>
  </button>
  <div class="spacer"></div>
  <Menu width={240}>
    {#snippet trigger(props)}
      <button {...props} class="account" aria-label="Account menu for {app.viewer.name}">
        <span class="avatar">{app.viewer.name.slice(0, 2).toUpperCase()}</span>
        <span class="name">{app.viewer.name}</span>
        <CaretDown size={15} />
      </button>
    {/snippet}
    <div class="nf-menu-label">{app.viewer.name} · {titleCase(app.viewer.role)}</div>
    <MenuItem icon={User} href="/account">Account</MenuItem>
    <MenuItem icon={Devices} href="/account?tab=devices">Devices & keys</MenuItem>
    <MenuItem icon={ClockCounterClockwise} href="/account?tab=history">Watch history</MenuItem>
    {#if app.viewer.isAdmin}<MenuItem icon={ShieldCheck} href="/admin">Administration</MenuItem>{/if}
    <div class="nf-menu-sep"></div>
    <MenuItem icon={SignOut} onSelect={logout}>Sign out</MenuItem>
  </Menu>
</header>

<style>
  .topbar { position: fixed; z-index: 30; top: 0; right: 0; left: var(--nav-w); height: var(--topbar-h); display: flex; align-items: center; gap: 14px; padding: 0 var(--gutter); transition: background 300ms var(--ease-out), backdrop-filter 300ms, border-color 300ms; border-bottom: 1px solid transparent; view-transition-name: chrome-top; }
  /* A single flex row is cheap to lay out per frame, and unlike a transform it keeps the right-hand controls on screen. */
  :global(:root[data-sidebar-motion]) .topbar { transition: left var(--nav-t) var(--ease-out), background 300ms var(--ease-out), backdrop-filter 300ms, border-color 300ms; }
  .topbar.scrolled { background: rgb(7 8 11 / 0.72); backdrop-filter: blur(22px) saturate(1.5); border-bottom-color: var(--line); }
  .mobile-brand { display: none; }
  /* Only the wide layout has a sidebar to collapse. */
  .sidebar-toggle { display: none; }
  @media (min-width: 1180px) { .sidebar-toggle { display: contents; } }
  .search { display: flex; align-items: center; gap: 10px; width: min(420px, 100%); height: 40px; padding: 0 8px 0 14px; border-radius: 999px; background: rgb(255 255 255 / 0.07); border: 1px solid var(--line); color: var(--text-3); font-size: 0.9rem; backdrop-filter: blur(16px); transition: background var(--t), border-color var(--t), color var(--t); }
  .search:hover { background: rgb(255 255 255 / 0.1); border-color: var(--line-strong); color: var(--text-2); }
  .search span { flex: 1; min-width: 0; text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  kbd { padding: 2px 7px; border-radius: 6px; background: var(--surface-2); border: 1px solid var(--line); font-size: 0.72rem; }
  .account { display: flex; align-items: center; gap: 9px; height: 42px; padding: 0 10px 0 4px; border-radius: 999px; color: var(--text-2); transition: background var(--t); }
  .account:hover, .account[data-state="open"] { background: var(--surface-2); color: var(--text); }
  .name { font-weight: 600; font-size: 0.9rem; }
  @media (max-width: 759px) {
    .topbar { padding: 0 12px; gap: 10px; }
    .mobile-brand { display: grid; }
    .search kbd, .name, .account > :global(svg) { display: none; }
    .search { flex: 1; width: auto; }
    .spacer { display: none; }
  }
</style>
