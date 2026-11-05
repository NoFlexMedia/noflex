<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import Plus from "phosphor-svelte/lib/Plus";
  import { titleCase } from "$lib/format.js";
  import { connectedServers, servers } from "$lib/servers.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import State from "$components/State.svelte";
  import LibraryTile from "$components/LibraryTile.svelte";

  let linked = $derived(connectedServers().map((server) => ({ server, answer: servers.libraries[server.id] })).filter(({ answer }) => answer));
</script>

<svelte:head><title>Libraries · NoFlex</title></svelte:head>

<div class="page">
  <header class="page-head">
    <div><p class="eyebrow">Your media</p><h1>Libraries</h1></div>
    {#if app.viewer.isAdmin}<div class="actions"><a class="btn outline" href="/admin"><Plus size={16} />Add library</a></div>{/if}
  </header>
  {#if linked.length && app.libraries.length}<h2 class="server">{app.server?.name ?? "This server"}</h2>{/if}
  {#if app.libraries.length}
    <div class="tiles stagger">
      {#each app.libraries as library, index (library.id)}<div style:--i={index}><LibraryTile {library} /></div>{/each}
    </div>
  {:else if !linked.length}
    <State icon={Books} title="No libraries yet" message={app.viewer.isAdmin ? "Create a library and point it at a folder of media." : "An administrator hasn’t shared a library with you yet."}>
      {#if app.viewer.isAdmin}<a class="btn primary sm" href="/admin">Open administration</a>{/if}
    </State>
  {/if}
  {#each linked as { server, answer } (server.id)}
    <h2 class="server">{server.name}</h2>
    {#if answer.items.length}
      <div class="tiles stagger">
        {#each answer.items as library, index (library.id)}<div style:--i={index}><LibraryTile {library} {server} /></div>{/each}
      </div>
    {:else}
      <p class="empty-note">{answer.failed ? "Can’t reach this server right now." : "Nothing is shared with you here yet."}</p>
    {/if}
  {/each}
</div>

<style>
  .server { margin: 28px 0 14px; font-size: 1.04rem; }
  .server:first-of-type { margin-top: 0; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 340px), 1fr)); gap: 20px; }
</style>
