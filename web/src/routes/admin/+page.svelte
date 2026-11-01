<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Plus from "phosphor-svelte/lib/Plus";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Tabs as BitsTabs } from "bits-ui";
  import { failure, refetch } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";
  import ActivityTab from "$components/admin/ActivityTab.svelte";
  import IndexingTab from "$components/admin/IndexingTab.svelte";
  import LibrariesTab from "$components/admin/LibrariesTab.svelte";
  import LogsTab from "$components/admin/LogsTab.svelte";
  import OverviewTab from "$components/admin/OverviewTab.svelte";
  import PlaybackTab from "$components/admin/PlaybackTab.svelte";
  import PluginsTab from "$components/admin/PluginsTab.svelte";
  import ResourcesTab from "$components/admin/ResourcesTab.svelte";
  import ServerTab from "$components/admin/ServerTab.svelte";
  import TranscodingTab from "$components/admin/TranscodingTab.svelte";
  import UsersTab from "$components/admin/UsersTab.svelte";
  import "$components/admin/status/diag.css";
  import Tabs from "$components/ui/Tabs.svelte";

  const TABS = ["overview", "activity", "playback", "libraries", "indexing", "transcoding", "resources", "plugins", "users", "logs", "server"];
  let tab = $derived(TABS.includes(page.url.searchParams.get("tab") ?? "") ? page.url.searchParams.get("tab") : "overview");

  let { data: route } = $props();
  let admin = $derived(route.admin);
  let data = $derived($admin.data);
  let creating = $state({ library: false, user: false });

  function setTab(next) {
    goto(next === "overview" ? "?" : `?tab=${next}`, { replaceState: true, noScroll: true, keepFocus: true });
  }
</script>

<svelte:head><title>Administration · NoFlex</title></svelte:head>

<div class="page">
  <header class="page-head">
    <div>
      <p class="eyebrow">Server</p>
      <h1>Administration</h1>
      {#if app.server}<p>{app.server.name} · v{app.server.version} · <span class="mono">{app.server.backend}</span> storage</p>{/if}
    </div>
    {#if app.viewer.isAdmin}
      <div class="actions">
        {#if tab === "libraries"}<button class="btn primary" onclick={() => (creating.library = true)}><Plus size={16} />New library</button>{/if}
        {#if tab === "users"}<button class="btn primary" onclick={() => (creating.user = true)}><Plus size={16} />New user</button>{/if}
      </div>
    {/if}
  </header>

  {#if !app.viewer.isAdmin}
    <State icon={ShieldCheck} title="Administrators only" message="This area manages the whole server. Ask an administrator if you need something changed."><a class="btn sm" href="/">Go home</a></State>
  {:else}
    <Tabs value={tab} onchange={setTab} label="Administration sections" items={[
      { value: "overview", label: "Overview" },
      { value: "activity", label: "Activity" },
      { value: "playback", label: "Playback" },
      { value: "libraries", label: "Libraries", count: data?.libraries.length ?? null },
      { value: "indexing", label: "Indexing" },
      { value: "transcoding", label: "Transcoding" },
      { value: "resources", label: "Resources" },
      { value: "plugins", label: "Plugins", count: data?.plugins.length ?? null },
      { value: "users", label: "Users", count: data?.users.length ?? null },
      { value: "logs", label: "Logs" },
      { value: "server", label: "Server" },
    ]}>
      {#if failure($admin) && !data}
        <div class="panel-body"><State kind="error" title="Couldn’t read the server’s state" message={failure($admin)} action={() => refetch(admin)} /></div>
      {:else}
        <BitsTabs.Content value="overview" class="panel-body"><OverviewTab {admin} navigate={setTab} /></BitsTabs.Content>
        <BitsTabs.Content value="activity" class="panel-body"><ActivityTab {admin} /></BitsTabs.Content>
        <BitsTabs.Content value="playback" class="panel-body"><PlaybackTab /></BitsTabs.Content>
        <BitsTabs.Content value="indexing" class="panel-body"><IndexingTab /></BitsTabs.Content>
        <BitsTabs.Content value="transcoding" class="panel-body"><TranscodingTab /></BitsTabs.Content>
        <BitsTabs.Content value="resources" class="panel-body"><ResourcesTab /></BitsTabs.Content>
        <BitsTabs.Content value="logs" class="panel-body"><LogsTab /></BitsTabs.Content>
        <BitsTabs.Content value="server" class="panel-body"><ServerTab /></BitsTabs.Content>
        <BitsTabs.Content value="libraries" class="panel-body"><LibrariesTab {admin} bind:creating={creating.library} /></BitsTabs.Content>
        <BitsTabs.Content value="users" class="panel-body"><UsersTab {admin} bind:creating={creating.user} /></BitsTabs.Content>
        <BitsTabs.Content value="plugins" class="panel-body"><PluginsTab {admin} /></BitsTabs.Content>
      {/if}
    </Tabs>
  {/if}
</div>

<style>
  .page :global(.panel-body) { margin-top: 28px; animation: fade-up 360ms var(--ease-out); outline: none; }
</style>
