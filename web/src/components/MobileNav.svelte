<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import House from "phosphor-svelte/lib/House";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import Television from "phosphor-svelte/lib/Television";
  import User from "phosphor-svelte/lib/User";
  import { page } from "$app/state";

  const items = [
    { href: "/", label: "Home", icon: House },
    { href: "/browse", label: "Browse", icon: Books },
    { href: "/search", label: "Search", icon: MagnifyingGlass },
    { href: "/live-tv", label: "Live", icon: Television },
    { href: "/account", label: "You", icon: User },
  ];
  const isActive = (href) => (href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href));
</script>

<nav class="mobile-nav" aria-label="Primary">
  {#each items as entry}
    <a href={entry.href} aria-current={isActive(entry.href) ? "page" : undefined}><entry.icon size={21} /><span>{entry.label}</span></a>
  {/each}
</nav>

<style>
  .mobile-nav { display: none; }
  @media (max-width: 759px) {
    .mobile-nav { position: fixed; z-index: 40; left: 0; right: 0; bottom: 0; display: grid; grid-template-columns: repeat(5, 1fr); padding: 6px 6px max(6px, env(safe-area-inset-bottom)); background: rgb(8 9 12 / 0.86); backdrop-filter: blur(22px) saturate(1.4); border-top: 1px solid var(--line); view-transition-name: chrome; }
    a { display: grid; justify-items: center; gap: 3px; padding: 6px 0; border-radius: 12px; color: var(--text-3); font-size: 0.68rem; font-weight: 600; transition: color var(--t); }
    a :global(svg) { transition: transform 360ms var(--ease-spring); }
    a[aria-current="page"] { color: var(--accent); }
    a[aria-current="page"] :global(svg) { transform: translateY(-1px) scale(1.08); }
  }
</style>
