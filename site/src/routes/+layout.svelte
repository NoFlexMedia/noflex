<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import "../app.css";
  import { page } from "$app/state";
  import Footer from "$components/Footer.svelte";
  import Nav from "$components/Nav.svelte";
  import { trackPress } from "$lib/actions";
  import { SITE } from "$lib/site";

  let { children } = $props();
</script>

<svelte:document onpointerdown={trackPress} />

<svelte:head>
  <link rel="canonical" href="{SITE}{page.url.pathname}" />
  <meta property="og:site_name" content="NoFlex" />
  <meta property="og:type" content="website" />
  <meta property="og:url" content="{SITE}{page.url.pathname}" />
  <meta property="og:image" content="{SITE}/og.jpg" />
  <meta property="og:image:alt" content="NoFlex: Your media, your server." />
  <meta name="twitter:card" content="summary_large_image" />
</svelte:head>

<a class="skip" href="#main">Skip to content</a>
<Nav />
<main id="main">{@render children()}</main>
<Footer />

<style>
  .skip { position: fixed; top: 12px; left: 12px; z-index: 100; padding: 8px 14px; border-radius: 999px; background: var(--accent); color: var(--accent-ink); font-weight: 600; transform: translateY(-200%); transition: transform var(--t) var(--ease-out); }
  .skip:focus { transform: none; }
</style>
