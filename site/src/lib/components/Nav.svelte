<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { page } from "$app/state";
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import GithubLogo from "phosphor-svelte/lib/GithubLogo";
  import List from "phosphor-svelte/lib/List";
  import X from "phosphor-svelte/lib/X";
  import BrandMark from "$components/BrandMark.svelte";
  import { links } from "$lib/site";

  const items = [
    { href: "/#features", label: "Features" },
    { href: "/#apps", label: "Apps" },
    { href: "/#open-source", label: "Open source" },
    { href: "/download", label: "Get NoFlex" },
    { href: "/support", label: "Support" },
  ];

  let scrolled = $state(false);
  let open = $state(false);
  $effect(() => { page.url; open = false; });
</script>

<svelte:window onscroll={() => (scrolled = scrollY > 8)} onkeydown={(event) => { if (event.key === "Escape") open = false; }} />

<header class="bar" class:scrolled={scrolled || open} class:open>
  <nav class="wrap inner" aria-label="Main">
    <a class="brand" href="/" aria-label="NoFlex home"><BrandMark size={28} intro /><span>NoFlex</span></a>
    <ul class="links">
      {#each items as item}
        <li><a href={item.href} aria-current={page.url.pathname === item.href ? "page" : undefined}>{item.label}</a></li>
      {/each}
    </ul>
    <div class="actions">
      <a class="icon-btn" href={links.repo} aria-label="NoFlex on GitHub"><GithubLogo size={21} weight="fill" /></a>
      <a class="btn primary sm demo" href="/demo">Try the demo<ArrowRight class="arrow" size={14} weight="bold" /></a>
      <button class="icon-btn menu" aria-expanded={open} aria-controls="mobile-menu" aria-label={open ? "Close menu" : "Open menu"} onclick={() => (open = !open)}>
        {#if open}<X size={22} />{:else}<List size={22} />{/if}
      </button>
    </div>
  </nav>
  {#if open}
    <ul class="sheet" id="mobile-menu">
      {#each items as item, i}
        <li style:--i={i}><a href={item.href}>{item.label}<ArrowRight size={16} /></a></li>
      {/each}
      <li style:--i={items.length}><a href={links.repo}>GitHub<GithubLogo size={18} weight="fill" /></a></li>
    </ul>
  {/if}
</header>

<style>
  .bar { position: fixed; inset: 0 0 auto; z-index: 50; transition: background var(--t) var(--ease-out), border-color var(--t), backdrop-filter var(--t); border-bottom: 1px solid transparent; }
  .bar.scrolled { background: var(--bg-overlay); backdrop-filter: blur(20px) saturate(1.5); border-bottom-color: var(--line); }
  .inner { display: flex; align-items: center; gap: 24px; height: var(--topbar-h); }
  .brand { display: inline-flex; align-items: center; gap: 10px; font-weight: 700; font-size: 1.08rem; letter-spacing: -0.03em; }
  .brand :global(svg) { transition: transform var(--t-slow) var(--ease-spring); }
  .brand:hover :global(svg) { transform: rotate(-8deg) scale(1.06); }
  .links { display: flex; gap: 2px; margin-right: auto; }
  .links a { display: inline-flex; align-items: center; height: 34px; padding: 0 12px; border-radius: 999px; font-size: 0.9rem; font-weight: 550; color: var(--text-2); transition: color var(--t), background var(--t); }
  .links a:hover, .links a[aria-current="page"] { color: var(--text); background: var(--surface-2); }
  .actions { display: flex; align-items: center; gap: 6px; margin-left: auto; }
  .menu { display: none; }
  .sheet { display: grid; gap: 2px; padding: 8px var(--gutter) 20px; }
  .sheet li { animation: fade-up 420ms var(--ease-out) both; animation-delay: calc(var(--i) * 35ms); }
  .sheet a { display: flex; align-items: center; justify-content: space-between; height: 52px; padding: 0 4px; border-bottom: 1px solid var(--line); font-size: 1.1rem; font-weight: 600; }
  .sheet a :global(svg) { color: var(--text-3); }
  @media (max-width: 899px) {
    .links { display: none; }
    .menu { display: inline-grid; }
  }
  @media (max-width: 420px) { .demo { display: none; } }
</style>
