<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import "../app.css";
  import { onNavigate } from "$app/navigation";
  import { Tooltip } from "bits-ui";
  import IconContext from "phosphor-svelte/lib/IconContext";
  import { cast } from "$lib/cast.svelte.js";
  import { connect } from "$lib/connect.svelte.js";
  import { player } from "$lib/playback.svelte.js";
  import { useReceiver } from "$lib/receiver.svelte.js";
  import { app, start } from "$lib/session.svelte.js";
  import { morph } from "$lib/transition.svelte.js";
  import { ambient, palette, prefersReducedMotion, toggleSidebar } from "$lib/ui.svelte.js";
  import AuthScreen from "$components/AuthScreen.svelte";
  import BrandMark from "$components/BrandMark.svelte";
  import CommandPalette from "$components/CommandPalette.svelte";
  import ConfirmHost from "$components/ConfirmHost.svelte";
  import MobileNav from "$components/MobileNav.svelte";
  import NavProgress from "$components/NavProgress.svelte";
  import RemoteBar from "$components/RemoteBar.svelte";
  import Player from "$components/Player.svelte";
  import Sidebar from "$components/Sidebar.svelte";
  import State from "$components/State.svelte";
  import { navigating, page } from "$app/state";
  import { deactivateServer, servers } from "$lib/servers.svelte.js";
  import Toaster from "$components/Toaster.svelte";
  import Topbar from "$components/Topbar.svelte";

  let { children } = $props();

  // Leaving a linked server's title or library returns to home, unless it's still playing.
  // A title's loader activates its server mid-navigation, while `page` still
  // says where we came from, so go by where we're headed.
  $effect(() => {
    const url = navigating.to?.url ?? page.url;
    const onServer = url.pathname.startsWith("/works/") || (url.pathname.startsWith("/libraries/") && url.searchParams.has("server"));
    if (servers.active && !player.current && !onServer) deactivateServer();
  });

  start();

  // The viewer's other devices, for as long as they are signed in.
  $effect(() => { if (app.phase === "app") return connect.run(); });
  $effect(() => { if (app.phase === "app") return useReceiver(); });
  $effect(() => { if (app.phase === "app") return cast.run(); });

  onNavigate((navigation) => {
    const from = navigation.from?.url.pathname ?? "";
    const to = navigation.to?.url.pathname ?? "";
    if (!from.startsWith("/works/") && !to.startsWith("/works/")) morph.key = null;
    if (!document.startViewTransition || prefersReducedMotion() || from === to) return;
    return new Promise((resolve) => {
      document.startViewTransition(async () => {
        resolve();
        await navigation.complete;
      });
    });
  });

  function shortcuts(event) {
    if (app.phase !== "app" || player.current) return;
    const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName ?? "") || document.activeElement?.isContentEditable;
    if ((event.key === "k" && (event.metaKey || event.ctrlKey)) || (event.key === "/" && !typing)) {
      event.preventDefault();
      palette.open = !palette.open;
    } else if (event.key === "[" && !typing && !event.metaKey && !event.ctrlKey && !event.altKey && matchMedia("(min-width: 760px)").matches) {
      event.preventDefault();
      toggleSidebar();
    }
  }
</script>

<svelte:window onkeydown={shortcuts} />
<svelte:head><meta name="theme-color" content="#07080b" /></svelte:head>

<!-- Icons are decorative unless a call site names them. -->
<IconContext values={{ size: 20, "aria-hidden": "true" }}>
<Tooltip.Provider delayDuration={350} disableHoverableContent>
  {#if app.phase === "boot"}
    <main class="boot" id="main" aria-busy="true">
      <BrandMark size={52} intro />
      <span class="sr-only">Opening NoFlex</span>
    </main>
  {:else if app.phase === "error"}
    <main class="boot" id="main"><State kind="error" title="NoFlex couldn’t start" message={app.error} action={start} /></main>
  {:else if app.phase === "auth" && page.url.pathname === "/invite"}
    {@render children()}
  {:else if app.phase === "auth" || app.phase === "setup"}
    <AuthScreen />
  {:else if app.viewer}
    <div class="ambient" style:--ambient={ambient.color ?? undefined} aria-hidden="true"></div>
    <a class="skip" href="#main">Skip to content</a>
    <Sidebar />
    <Topbar />
    <main id="main" class="main nav-follow" tabindex="-1">{@render children()}</main>
    <MobileNav />
    {#if !player.current}<RemoteBar />{/if}
    <CommandPalette bind:open={palette.open} />
    {#if player.current}<Player playback={player.current} />{/if}
  {/if}
  <NavProgress />
  <Toaster />
  <ConfirmHost />
</Tooltip.Provider>
</IconContext>

<style>
  .boot { min-height: 100dvh; display: grid; place-items: center; }
  .boot :global(svg) { animation: breathe 1.6s var(--ease-in-out) infinite; }
  @keyframes breathe { 50% { transform: scale(0.94); opacity: 0.85; } }
  .main { margin-left: var(--nav-w); min-height: 100dvh; outline: none; }
  .ambient { position: fixed; inset: 0 0 auto 0; height: 90vh; z-index: -1; pointer-events: none; background-color: rgb(var(--ambient) / 0.34); mask-image: radial-gradient(120% 70% at 60% -10%, #000 0%, transparent 70%); transition: background-color 1.2s var(--ease-out); }
  .skip { position: fixed; z-index: 200; left: 12px; top: -60px; padding: 10px 14px; border-radius: 10px; background: var(--accent); color: var(--accent-ink); font-weight: 650; transition: top var(--t); }
  .skip:focus { top: 12px; }
  @media (max-width: 759px) { .main { padding-bottom: 72px; } }
</style>
