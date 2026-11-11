<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import GithubLogo from "phosphor-svelte/lib/GithubLogo";
  import BrandMark from "$components/BrandMark.svelte";
  import { links } from "$lib/site";

  const groups = [
    { title: "Product", items: [["Features", "/#features"], ["Apps", "/#apps"], ["Demo", "/demo"], ["Get NoFlex", "/download"]] },
    { title: "Project", items: [["Source code", links.repo], ["Documentation", links.gettingStarted], ["Writing plugins", links.plugins], ["Contributing", links.contributing], ["Releases", links.releases]] },
    { title: "Help", items: [["Support", "/support"], ["Report a bug", links.newIssue], ["Report a vulnerability", "/support#security"], ["Delete your account", "/delete-account"]] },
    { title: "Legal", items: [["Privacy policy", "/privacy"], ["Terms of use", "/terms"], ["Credits & licenses", "/credits"]] },
  ];
</script>

<footer>
  <div class="wrap">
    <div class="top">
      <div class="intro">
        <a class="brand" href="/"><BrandMark size={30} /><span>NoFlex</span></a>
        <p>Your media, your server. Free software for the screens you already own.</p>
        <a class="btn sm" href={links.repo}><GithubLogo size={16} weight="fill" />NoFlexMedia/noflex</a>
      </div>
      {#each groups as group}
        <nav aria-label={group.title}>
          <h2>{group.title}</h2>
          <ul>{#each group.items as [label, href]}<li><a {href}>{label}</a></li>{/each}</ul>
        </nav>
      {/each}
    </div>
    <div class="bottom">
      <p>© {new Date().getFullYear()} Christopher Tam. NoFlex is free software under the <a href={links.license}>GNU AGPL v3</a>.</p>
      <p>Artwork from <a href="/credits#artwork">Blender Studio's open movies</a>, CC BY.</p>
    </div>
  </div>
</footer>

<style>
  footer { border-top: 1px solid var(--line); padding: 64px 0 40px; background: linear-gradient(180deg, var(--bg), var(--bg-raised)); }
  .top { display: grid; grid-template-columns: minmax(220px, 1.6fr) repeat(4, minmax(0, 1fr)); gap: 40px 28px; }
  .intro { display: grid; align-content: start; justify-items: start; gap: 16px; }
  .intro p { color: var(--text-3); font-size: 0.92rem; max-width: 30ch; }
  .brand { display: inline-flex; align-items: center; gap: 10px; font-weight: 700; font-size: 1.1rem; letter-spacing: -0.03em; }
  h2 { font-size: 0.72rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-3); margin-bottom: 14px; }
  ul { display: grid; gap: 9px; }
  nav a { color: var(--text-2); font-size: 0.9rem; transition: color var(--t); }
  nav a:hover { color: var(--text); }
  .bottom { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 8px 24px; margin-top: 56px; padding-top: 24px; border-top: 1px solid var(--line); color: var(--text-3); font-size: 0.82rem; }
  .bottom a { color: var(--text-2); text-decoration: underline; text-decoration-color: var(--line-strong); text-underline-offset: 3px; }
  .bottom a:hover { color: var(--text); }
  @media (max-width: 899px) { .top { grid-template-columns: repeat(2, minmax(0, 1fr)); } .intro { grid-column: 1 / -1; } }
</style>
