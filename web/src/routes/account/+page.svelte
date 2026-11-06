<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import SignOut from "phosphor-svelte/lib/SignOut";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Tabs as BitsTabs } from "bits-ui";
  import { date, titleCase } from "$lib/format.js";
  import { failure, refetch } from "$lib/graphql.js";
  import { app, logout } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";
  import DevicesTab from "$components/account/DevicesTab.svelte";
  import DownloadsTab from "$components/account/DownloadsTab.svelte";
  import HistoryTab from "$components/account/HistoryTab.svelte";
  import IntegrationsTab from "$components/account/IntegrationsTab.svelte";
  import ProfileTab from "$components/account/ProfileTab.svelte";
  import ServersTab from "$components/account/ServersTab.svelte";
  import SmartTranscodingTab from "$components/account/SmartTranscodingTab.svelte";
  import Tabs from "$components/ui/Tabs.svelte";

  const TABS = ["profile", "devices", "smart", "servers", "integrations", "downloads", "history"];
  let tab = $derived(TABS.includes(page.url.searchParams.get("tab") ?? "") ? page.url.searchParams.get("tab") : "profile");

  let { data: route } = $props();
  let account = $derived(route.account);
  let data = $derived($account.data);
  let activeJobs = $derived((data?.exportJobs.edges ?? []).filter(({ node: job }) => ["QUEUED", "RUNNING", "CANCEL_REQUESTED"].includes(job.status)).length);
  let items = $derived([
    { value: "profile", label: "Profile" },
    { value: "devices", label: "Devices & keys", count: data ? data.mySessions.length : null },
    { value: "smart", label: "Smart transcoding" },
    { value: "servers", label: "Servers" },
    { value: "integrations", label: "Integrations" },
    { value: "downloads", label: "Offline copies", count: activeJobs || null },
    { value: "history", label: "History" },
  ]);

  function setTab(next) {
    const params = new URLSearchParams(page.url.searchParams);
    if (next === "profile") params.delete("tab"); else params.set("tab", next);
    goto(`?${params}`, { replaceState: true, noScroll: true, keepFocus: true });
  }
</script>

<svelte:head><title>Account · NoFlex</title></svelte:head>

<div class="page">
  <header class="page-head">
    <div class="who">
      <span class="avatar lg" aria-hidden="true">{app.viewer.name.slice(0, 2).toUpperCase()}</span>
      <div>
        <p class="eyebrow">Your account</p>
        <h1>{app.viewer.name}</h1>
        <p>{titleCase(app.viewer.role)} · Member since {date(app.viewer.createdAt, { month: "long", year: "numeric" })}</p>
      </div>
    </div>
    <div class="actions"><button class="btn ghost" onclick={logout}><SignOut size={16} />Sign out</button></div>
  </header>

  <Tabs value={tab} onchange={setTab} {items} label="Account sections">
    {#if failure($account) && !data && tab !== "history"}
      <div class="panel-body"><State kind="error" title="Your account didn’t load" message={failure($account)} action={() => refetch(account)} /></div>
    {:else}
      <BitsTabs.Content value="profile" class="panel-body"><ProfileTab {account} /></BitsTabs.Content>
      <BitsTabs.Content value="devices" class="panel-body"><DevicesTab {account} /></BitsTabs.Content>
      <BitsTabs.Content value="smart" class="panel-body"><SmartTranscodingTab /></BitsTabs.Content>
      <BitsTabs.Content value="servers" class="panel-body"><ServersTab /></BitsTabs.Content>
      <BitsTabs.Content value="integrations" class="panel-body"><IntegrationsTab {account} /></BitsTabs.Content>
      <BitsTabs.Content value="downloads" class="panel-body"><DownloadsTab {account} /></BitsTabs.Content>
    {/if}
    <BitsTabs.Content value="history" class="panel-body"><HistoryTab /></BitsTabs.Content>
  </Tabs>
</div>

<style>
  .who { display: flex; align-items: center; gap: 20px; }
  .who .avatar { box-shadow: 0 18px 40px -16px rgb(212 255 90 / 0.5); }
  .page :global(.panel-body) { margin-top: 28px; animation: fade-up 360ms var(--ease-out); outline: none; }
  @media (max-width: 520px) { .who { gap: 14px; } .who .avatar { width: 56px; height: 56px; font-size: 1.2rem; } }
</style>
