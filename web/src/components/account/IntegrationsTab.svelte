<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowSquareOut from "phosphor-svelte/lib/ArrowSquareOut";
  import Check from "phosphor-svelte/lib/Check";
  import Link from "phosphor-svelte/lib/Link";
  import Plug from "phosphor-svelte/lib/Plug";
  import User from "phosphor-svelte/lib/User";
  import { graphql } from "$houdini";
  import { newExternalNonce, webUrl } from "$lib/api.js";
  import { date } from "$lib/format.js";
  import { fetchQuery, refetch } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import { ApiError } from "$lib/tokens.js";
  import { attempt, confirmAction, toast } from "$lib/ui.svelte.js";

  let { account } = $props();

  // `plugins` is admin-only on most servers; without it we can still list
  // existing links, just not offer new ones.
  const plugins = graphql(`
    query AccountPlugins @cache(policy: CacheOrNetwork) {
      plugins { id displayName hooks tasks { name } }
    }
  `);
  $effect(() => { if (app.viewer.isAdmin) fetchQuery(plugins); });
  const startIdentityLink = graphql(`
    mutation StartIdentityLink($provider: String!, $nonce: String!) {
      startExternalIdentityLink(provider: $provider, clientNonce: $nonce) { authorizationUrl }
    }
  `);
  const revokeIdentityMutation = graphql(`
    mutation RevokeExternalIdentity($provider: String!) { revokeExternalIdentity(provider: $provider) { ...MyExternalIdentities_remove } }
  `);
  const startPluginLink = graphql(`
    mutation StartPluginLink($plugin: String!) { startPluginLink(pluginId: $plugin) { authUrl state } }
  `);
  const unlinkPlugin = graphql(`
    mutation UnlinkPluginAccount($plugin: String!) { unlinkPluginAccount(pluginId: $plugin) { ...MyPluginLinks_remove } }
  `);

  let data = $derived($account.data);
  let identities = $derived(data?.myExternalIdentities ?? []);
  let providers = $derived(data?.externalAuthProviders ?? []);
  let unlinked = $derived(providers.filter((provider) => !identities.some((identity) => identity.pluginId === provider.id)));
  // An account made by signing in through a provider has no password.
  let hasPassword = $derived(data?.accountDeletion.passwordRequired ?? true);
  let links = $derived(data?.myPluginLinks ?? []);
  let linkable = $derived.by(() => {
    if (!$plugins.data) return null;
    const list = $plugins.data.plugins.filter((plugin) => plugin.hooks.includes("USER_LINK"));
    return list.map((plugin) => ({ id: plugin.id, name: plugin.displayName, link: links.find((link) => link.pluginId === plugin.id) }));
  });
  let pluginRows = $derived(linkable ?? links.map((link) => ({ id: link.pluginId, name: link.pluginId, link })));
  let busy = $state("");

  const providerName = (id) => providers.find((provider) => provider.id === id)?.displayName ?? id;

  async function linkIdentity(provider) {
    busy = provider.id;
    const result = (await attempt(() => startIdentityLink.mutate({ provider: provider.id, nonce: newExternalNonce() })))?.data;
    const url = webUrl(result?.startExternalIdentityLink.authorizationUrl);
    if (url) { location.assign(url); return; }
    if (result) toast("The provider returned an unusable sign-in link", "error");
    busy = "";
  }

  const onlySignIn = (name) => unlinked.length
    ? `${name} is the only way you can sign in. Link another provider before disconnecting it.`
    : `${name} is the only way you can sign in, so it can’t be disconnected.`;

  async function revokeIdentity(identity) {
    const name = providerName(identity.pluginId);
    const others = identities.filter((other) => other.pluginId !== identity.pluginId).map((other) => providerName(other.pluginId));
    if (!hasPassword && !others.length) { toast(onlySignIn(name), "error"); return; }
    const still = hasPassword ? "Your password still works." : `You can still sign in with ${others.join(" or ")}.`;
    if (!(await confirmAction({ title: `Disconnect ${name}?`, message: `You won’t be able to sign in with it until you link it again. ${still}`, confirmLabel: "Disconnect", danger: true }))) return;
    try {
      await revokeIdentityMutation.mutate({ provider: identity.pluginId });
      toast(`${name} disconnected`, "success");
    } catch (cause) {
      // Another identity went meanwhile, from another tab or device.
      if (cause instanceof ApiError && cause.codes.includes("LAST_SIGN_IN_METHOD")) {
        toast(onlySignIn(name), "error");
        refetch(account);
      } else {
        toast(cause instanceof Error ? cause.message : String(cause), "error");
      }
    }
  }

  async function connect(row) {
    busy = row.id;
    const result = await attempt(() => startPluginLink.mutate({ plugin: row.id }));
    busy = "";
    if (!result) return;
    const url = webUrl(result.data.startPluginLink.authUrl);
    if (!url) { toast(`${row.name} returned an unusable link`, "error"); return; }
    window.open(url, "_blank", "noopener,noreferrer");
    toast(`Finish connecting ${row.name} in the new tab`, "info", { label: "I’m done", run: () => refetch(account) });
  }

  async function unlink(row) {
    if (!(await confirmAction({ title: `Unlink ${row.name}?`, message: "NoFlex forgets the credentials it stored for this service.", confirmLabel: "Unlink", danger: true }))) return;
    await attempt(() => unlinkPlugin.mutate({ plugin: row.id }), `${row.name} unlinked`);
  }
</script>

{#if ($account.fetching && !$account.data)}
  <div class="grid" aria-busy="true">
    {#each [0, 1] as _}<div class="panel"><div class="skeleton head"></div><div class="skeleton line"></div><div class="skeleton line short"></div></div>{/each}
  </div>
{:else if data}
  <div class="grid stagger">
    <section class="panel" style:--i={0}>
      <header class="panel-head">
        <span class="panel-icon"><User size={19} /></span>
        <div><h2>Sign-in identities</h2><p>External accounts you can sign in with</p></div>
      </header>
      {#if identities.length}
        <ul class="list">
          {#each identities as identity (identity.pluginId + identity.subject)}
            <li>
              <span class="mark on"><Check size={16} weight="bold" /></span>
              <span class="what"><strong>{identity.displayName || identity.username || providerName(identity.pluginId)}</strong><small>{providerName(identity.pluginId)} · {identity.email || identity.issuer} · linked {date(identity.linkedAt)}</small></span>
              <button class="btn ghost sm" onclick={() => revokeIdentity(identity)}>Disconnect</button>
            </li>
          {/each}
        </ul>
      {/if}
      {#if unlinked.length}
        <div class="providers" class:spaced={identities.length}>
          {#each unlinked as provider (provider.id)}
            <button class="btn outline sm" onclick={() => linkIdentity(provider)} disabled={!!busy}>{#if busy === provider.id}<span class="spinner sm"></span>{:else}<Link size={15} />{/if}Link {provider.displayName}</button>
          {/each}
        </div>
      {:else if !identities.length}
        <p class="empty-note">This server has no external sign-in providers. An administrator can add one with an auth plugin.</p>
      {/if}
    </section>

    <section class="panel" style:--i={1}>
      <header class="panel-head">
        <span class="panel-icon"><Plug size={19} /></span>
        <div><h2>Connected services</h2><p>Scrobblers, watch-history sync and other plugin accounts</p></div>
      </header>
      {#if plugins.loading && app.viewer.isAdmin}
        <div class="skeleton line"></div><div class="skeleton line short"></div>
      {:else if pluginRows.length}
        <ul class="list">
          {#each pluginRows as row (row.id)}
            <li>
              <span class="mark" class:on={row.link}>{#if row.link}<Check size={16} />{:else}<Plug size={16} />{/if}</span>
              <span class="what"><strong>{row.name}</strong><small>{row.link ? `Connected${row.link.keys.length ? ` · ${row.link.keys.join(", ")}` : ""} · ${date(row.link.updatedAt)}` : "Not connected"}</small></span>
              {#if row.link}
                <button class="btn ghost sm" onclick={() => unlink(row)}>Unlink</button>
              {:else}
                <button class="btn sm" onclick={() => connect(row)} disabled={busy === row.id}>{#if busy === row.id}<span class="spinner sm"></span>{:else}<ArrowSquareOut size={14} />{/if}Connect</button>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty-note">{linkable ? "No account-linking plugins are installed on this server." : "Nothing connected yet. Ask your administrator which services this server can link to."}</p>
      {/if}
    </section>
  </div>
{/if}

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 380px), 1fr)); gap: 18px; align-items: start; }
  .skeleton.head { height: 38px; width: 60%; margin-bottom: 22px; border-radius: 11px; }
  .skeleton.line { height: 44px; margin-top: 12px; border-radius: 10px; }
  .skeleton.short { width: 65%; }
  .mark { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 10px; background: var(--surface-2); color: var(--text-3); flex: none; }
  .mark.on { background: rgb(110 231 168 / 0.12); color: var(--success); }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .providers { display: flex; flex-wrap: wrap; gap: 8px; }
  .providers.spaced { margin-top: 14px; padding-top: 14px; border-top: 1px solid var(--line); }
</style>
