<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import { titleCase } from "$lib/format.js";
  import { servers } from "$lib/servers.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";
  import WorkBrowser from "$components/WorkBrowser.svelte";

  let { data: route } = $props();

  const KINDS = ["MOVIE", "SERIES", "SEASON", "EPISODE", "ARTIST", "ALBUM", "TRACK", "AUDIO_BOOK", "PHOTO"];
  let library = $derived((route.serverId ? servers.libraries[route.serverId]?.items ?? [] : app.libraries).find((item) => item.id === route.id));
  let serverName = $derived(route.serverId ? servers.linked.find((server) => server.id === route.serverId)?.name : null);
</script>

<svelte:head><title>{library?.name ?? "Library"} · NoFlex</title></svelte:head>

{#snippet notFound()}
  <State icon={Books} title="Library not found" message="It may have been removed, or you no longer have access."><a class="btn sm" href="/libraries">All libraries</a></State>
{/snippet}

<WorkBrowser works={route.works} title={library?.name ?? "Library"} eyebrow={library ? [serverName, titleCase(library.kind)].filter(Boolean).join(" · ") : "Library"} kinds={KINDS} emptyTitle="This library is empty" emptyMessage={app.viewer.isAdmin && !route.serverId ? "Run a scan from Administration once media is in place." : "Check back after the next scan."} missing={library ? undefined : notFound} />
