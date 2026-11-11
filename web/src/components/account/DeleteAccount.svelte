<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Trash from "phosphor-svelte/lib/Trash";
  import { graphql } from "$houdini";
  import { ApiError } from "$lib/tokens.js";
  import { request } from "$lib/graphql.js";
  import { accountDeleted, app } from "$lib/session.svelte.js";
  import { attempt, confirmAction, toast } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";

  // What the last administrator types to erase the server: words, not their
  // name, so it can't be filled in without reading what it does.
  const PHRASE = "delete everything";

  const accountDeletion = graphql(`
    query AccountDeletion @cache(policy: NetworkOnly) { accountDeletion { passwordRequired erasesServer } }
  `);
  const deleteAccount = graphql(`
    mutation DeleteAccount($password: String, $username: String, $eraseServer: Boolean!) {
      deleteAccount(currentPassword: $password, username: $username, eraseServer: $eraseServer) { user { id } serverErased }
    }
  `);

  let open = $state(false);
  let terms = $state({ passwordRequired: true, erasesServer: false });
  let form = $state({ password: "", username: "", phrase: "" });
  let checking = $state(false);
  let deleting = $state(false);
  let confirmed = $derived(
    (terms.passwordRequired ? form.password.length > 0 : form.username.trim() === app.viewer?.username)
      && (!terms.erasesServer || form.phrase.trim().toLowerCase() === PHRASE),
  );

  async function begin() {
    checking = true;
    const next = await attempt(async () => (await request(accountDeletion)).accountDeletion);
    checking = false;
    if (next) await ask(next);
  }

  /** The last administrator is warned before they're asked for anything. */
  async function ask(next) {
    if (next.erasesServer && !(await confirmAction({
      title: "Deleting your account erases this server",
      message: "You’re its last administrator, so your account can only go with the whole server, which starts again from first-run setup. Every account on it is deleted and everyone is signed out, and every library, watch history, setting, linked server and plugin link goes, with everything the server made: transcodes, optimized versions, artwork, thumbnails and the search index. Media files on disk and the server’s configuration file are kept.",
      confirmLabel: "Continue",
      danger: true,
    }))) return;
    terms = next;
    form = { password: "", username: "", phrase: "" };
    open = true;
  }

  async function submit(event) {
    event.preventDefault();
    if (!confirmed) return;
    deleting = true;
    try {
      const result = await deleteAccount.mutate({
        password: terms.passwordRequired ? form.password : null,
        username: terms.passwordRequired ? null : form.username.trim(),
        eraseServer: terms.erasesServer,
      });
      open = false;
      accountDeleted(result.data.deleteAccount.serverErased);
    } catch (cause) {
      // Someone else stopped being an administrator meanwhile: this now erases
      // the server, which they're told before anything happens.
      if (cause instanceof ApiError && cause.codes.includes("LAST_ADMINISTRATOR")) {
        open = false;
        await ask({ ...terms, erasesServer: true });
      } else {
        toast(cause instanceof Error ? cause.message : String(cause), "error");
      }
    } finally {
      deleting = false;
    }
  }
</script>

<section class="panel danger-zone" style:--i={3}>
  <header class="panel-head">
    <span class="panel-icon"><Trash size={19} /></span>
    <div><h2>Delete account</h2><p>For good, from every device</p></div>
  </header>
  <div class="row">
    <p>Your watch history, favorites, playlists, devices and keys, invites, linked services and servers, picture and offline copies are deleted, and you’re signed out everywhere. It can’t be undone. {#if app.viewer?.isAdmin}If you’re the server’s last administrator, the whole server goes with it.{/if}</p>
    <button type="button" class="btn danger sm" disabled={checking} onclick={begin}>{#if checking}<span class="spinner sm"></span>{/if}Delete account…</button>
  </div>
</section>

<Dialog bind:open title={terms.erasesServer ? "Erase this server" : "Delete your account"} description={terms.erasesServer ? "Last chance. Every account and everything the server holds is erased, and it can’t be undone. Media files stay." : "Everything that’s yours on this server is deleted, and it can’t be undone."} width={460}>
  <form id="delete-account-form" class="stack" onsubmit={submit}>
    {#if terms.erasesServer}
      <label class="field"><span>Type <strong class="phrase">{PHRASE}</strong> to confirm</span><input bind:value={form.phrase} autocomplete="off" autocapitalize="none" spellcheck="false" required /></label>
    {/if}
    {#if terms.passwordRequired}
      <label class="field">Your password<input type="password" bind:value={form.password} autocomplete="current-password" required /></label>
    {:else}
      <label class="field"><span>Type your username, <strong class="phrase">{app.viewer?.username}</strong>, to confirm</span><input bind:value={form.username} autocomplete="off" autocapitalize="none" spellcheck="false" required /><small>You sign in through another service, so there’s no password to ask for.</small></label>
    {/if}
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (open = false)}>Cancel</button>
    <button class="btn danger" form="delete-account-form" disabled={deleting || !confirmed}>{#if deleting}<span class="spinner sm"></span>{/if}{terms.erasesServer ? "Erase the server" : "Delete my account"}</button>
  {/snippet}
</Dialog>

<style>
  .danger-zone { grid-column: 1 / -1; border-color: rgb(255 107 107 / 0.28); }
  .danger-zone .panel-icon { background: var(--danger-soft); color: var(--danger); }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 18px; flex-wrap: wrap; }
  .row p { flex: 1 1 320px; color: var(--text-2); font-size: 0.92rem; max-width: 62ch; }
  .phrase { color: var(--text); font-weight: 650; }
</style>
