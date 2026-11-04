<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import HardDrives from "phosphor-svelte/lib/HardDrives";
  import Trash from "phosphor-svelte/lib/Trash";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { AddLinkedServerStore, LinkedServersStore, LogoutStore, RemoveLinkedServerStore, UpdateLinkedServerStore } from "$houdini";
  import { deviceName } from "$lib/api.js";
  import { request } from "$lib/graphql.js";
  import { serverRequest, servers, setLinkedServers } from "$lib/servers.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt, confirmAction, toast } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";

  const LOGIN = `mutation($username: String!, $password: String!, $device: String) {
    login(username: $username, password: $password, client: "NoFlex link", device: $device) { token user { isAdmin } }
  }`;
  const CREATE_KEY = `mutation($name: String!) { createApiKey(name: $name) { key } }`;
  const INFO = `{ serverInfo { name } }`;
  const DEFAULT_SERVER_NAME = "noflex";

  let address = $state("");
  let adding = $state(false);
  let signingIn = $state(null);
  let credentials = $state({ username: "", password: "" });
  let busy = $state(false);

  const reload = async () => setLinkedServers((await request(new LinkedServersStore())).linkedServers);

  async function add(event) {
    event.preventDefault();
    let url;
    try { url = new URL(address.trim()); } catch { toast("Enter a server address like https://media.example.com", "error"); return; }
    // The browser would refuse every request with no useful error.
    if (location.protocol === "https:" && url.protocol === "http:") {
      toast("This page is on HTTPS, so browsers won’t let it reach a plain HTTP server. Use an HTTPS address.", "error");
      return;
    }
    adding = true;
    const result = await attempt(() => new AddLinkedServerStore().mutate({ origin: url.origin, name: url.hostname, key: "" }));
    adding = false;
    // This page's security policy only allows servers known when it loaded.
    if (result) location.reload();
  }

  async function connect(event, server) {
    event.preventDefault();
    busy = true;
    try {
      const { login } = await serverRequest(server, LOGIN, { ...credentials, device: deviceName() });
      if (login.user.isAdmin && !(await confirmAction({
        title: "Link an administrator account?",
        message: `A key for this account will be stored on ${app.server?.name ?? "this server"}, so anyone who gets into this server could administer ${server.name}. An ordinary account is safer.`,
        confirmLabel: "Link anyway",
        danger: true,
      }))) return;
      const session = { origin: server.origin, apiKey: login.token };
      const [{ createApiKey }, info] = await Promise.all([
        serverRequest(session, CREATE_KEY, { name: `NoFlex link from ${app.server?.name ?? "another server"}` }),
        serverRequest(session, INFO),
      ]);
      // Servers nobody has named all share the default, which would tell them apart from nothing.
      const name = info.serverInfo.name && info.serverInfo.name !== DEFAULT_SERVER_NAME ? info.serverInfo.name : server.name;
      await new AddLinkedServerStore().mutate({ origin: server.origin, name, key: createApiKey.key });
      // The key is what stays; this sign-in has done its job.
      serverRequest(session, new LogoutStore().artifact.raw).catch(() => {});
      await reload();
      signingIn = null;
      credentials = { username: "", password: "" };
      toast(`Connected to ${name}`, "success");
    } catch (cause) {
      const refused = cause instanceof Error && !(cause instanceof TypeError) && cause.name !== "TimeoutError";
      toast(refused ? cause.message : `Couldn’t reach ${server.origin}. Check that it’s running and allows cross-origin requests.`, "error");
    } finally { busy = false; }
  }

  async function setSync(server, enabled) {
    if (await attempt(() => new UpdateLinkedServerStore().mutate({ id: server.id, sync: enabled }))) await reload();
  }

  async function remove(server) {
    if (!(await confirmAction({ title: `Remove ${server.name}?`, message: "Its titles leave your home screen and search. To fully cut access, also revoke the key from your account on that server.", confirmLabel: "Remove server", danger: true }))) return;
    if (await attempt(() => new RemoveLinkedServerStore().mutate({ id: server.id }), "Server removed")) await reload();
  }
</script>

<div class="grid stagger">
  <section class="panel wide" style:--i={0}>
    <header class="panel-head">
      <span class="panel-icon"><HardDrives size={19} /></span>
      <div><h2>Other servers</h2><p>Browse friends’ NoFlex servers alongside this one. Your device talks to them directly</p></div>
    </header>
    {#if servers.linked.length}
      <ul class="servers">
        {#each servers.linked as server (server.id)}
          <li>
            <div class="row">
              <span class="who"><strong>{server.name}</strong><small class="mono">{server.origin}</small></span>
              <div class="actions">
                {#if !server.apiKey}<button class="btn sm" onclick={() => (signingIn = signingIn === server.id ? null : server.id)}>Sign in</button>{/if}
                <button class="icon-btn sm" aria-label="Remove {server.name}" onclick={() => remove(server)}><Trash size={15} /></button>
              </div>
            </div>
            {#if server.apiKey}
              <label class="toggle"><input type="checkbox" class="switch" checked={server.syncEnabled} onchange={(event) => setSync(server, event.currentTarget.checked)} /><span><strong>Sync what I watch</strong><small>Keeps your place on both servers for titles they share. {server.name}’s administrators can see what you’ve watched there, including what syncs from here.</small></span></label>
            {/if}
            {#if signingIn === server.id}
              <form class="stack" onsubmit={(event) => connect(event, server)}>
                <label class="field">Username on {server.name}<input bind:value={credentials.username} autocomplete="off" required /></label>
                <label class="field">Password<input type="password" bind:value={credentials.password} autocomplete="off" required /><small>Used once to create a key, and never stored. You can revoke the key from your account there.</small></label>
                <div class="form-actions"><button class="btn primary sm" disabled={busy}>{#if busy}<span class="spinner sm"></span>{/if}Connect</button></div>
              </form>
            {/if}
          </li>
        {/each}
      </ul>
    {:else}
      <State icon={HardDrives} title="Only this server so far" message="Add another server you have an account on, or ask a friend for an invite to theirs." />
    {/if}
    <form class="add" onsubmit={add}>
      <label class="field">Server address<input bind:value={address} placeholder="https://media.example.com" inputmode="url" autocomplete="off" spellcheck="false" required /></label>
      <button class="btn sm" disabled={adding}>{#if adding}<span class="spinner sm"></span>{/if}Add server</button>
    </form>
    {#if location.protocol === "https:"}<p class="note"><WarningCircle size={15} />Servers must use HTTPS to work from this page.</p>{/if}
  </section>
</div>

<style>
  .servers { display: grid; gap: 18px; list-style: none; margin: 0 0 20px; padding: 0; }
  .servers li { display: grid; gap: 12px; }
  .row { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px 16px; }
  .who { display: grid; gap: 2px; min-width: 0; }
  .who small { color: var(--text-3); overflow-wrap: anywhere; }
  .actions { display: flex; align-items: center; gap: 8px; }
  .add { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 10px; }
  .add .field { flex: 1 1 260px; }
  .note { display: flex; align-items: center; gap: 6px; margin: 12px 0 0; color: var(--text-3); font-size: 0.84rem; }
</style>
