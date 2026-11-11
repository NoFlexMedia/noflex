<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { onMount } from "svelte";

  /** `toc`: [[id, label]] for the page's h2s, which carry those ids. */
  let { eyebrow, title, lede = null, updated = null, toc = [], children } = $props();
  let current = $state("");

  onMount(() => {
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) if (entry.isIntersecting) current = entry.target.id;
    }, { rootMargin: "-20% 0px -70% 0px" });
    for (const [id] of toc) { const el = document.getElementById(id); if (el) observer.observe(el); }
    return () => observer.disconnect();
  });
</script>

<article class="doc wrap">
  <header>
    <p class="eyebrow">{eyebrow}</p>
    <h1>{title}</h1>
    {#if lede}<p class="lede">{lede}</p>{/if}
    {#if updated}<p class="updated">Last updated {updated}</p>{/if}
  </header>
  <div class="body" class:has-toc={toc.length}>
    {#if toc.length}
      <nav aria-label="On this page">
        <p class="eyebrow">On this page</p>
        <ol>{#each toc as [id, label]}<li><a href="#{id}" aria-current={current === id ? "true" : undefined}>{label}</a></li>{/each}</ol>
      </nav>
    {/if}
    <div class="prose">{@render children()}</div>
  </div>
</article>

<style>
  .doc { padding-top: calc(var(--topbar-h) + clamp(48px, 9vh, 96px)); padding-bottom: 120px; }
  header { display: grid; gap: 14px; max-width: 760px; margin-bottom: clamp(40px, 7vh, 72px); animation: fade-up 700ms var(--ease-out) both; }
  h1 { font-size: clamp(2.4rem, 5.6vw, 4.2rem); font-weight: 740; letter-spacing: -0.05em; line-height: 1; }
  .lede { color: var(--text-2); font-size: clamp(1.04rem, 1.4vw, 1.2rem); }
  .updated { color: var(--text-3); font-size: 0.86rem; }
  .body.has-toc { display: grid; grid-template-columns: 220px minmax(0, 1fr); gap: clamp(32px, 6vw, 88px); align-items: start; }
  nav { position: sticky; top: calc(var(--topbar-h) + 28px); display: grid; gap: 12px; animation: fade-up 700ms var(--ease-out) 120ms both; }
  nav ol { display: grid; gap: 2px; border-left: 1px solid var(--line); }
  nav a { display: block; padding: 5px 0 5px 14px; margin-left: -1px; border-left: 1px solid transparent; color: var(--text-3); font-size: 0.86rem; transition: color var(--t), border-color var(--t); }
  nav a:hover { color: var(--text-2); }
  nav a[aria-current="true"] { color: var(--text); border-left-color: var(--accent); }
  .prose { animation: fade-up 700ms var(--ease-out) 80ms both; }
  .prose > :global(:first-child) { margin-top: 0; }
  @media (max-width: 899px) { .body.has-toc { grid-template-columns: 1fr; } nav { display: none; } }
</style>
