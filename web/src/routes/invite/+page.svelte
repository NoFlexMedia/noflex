<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { goto } from "$app/navigation";
  import { RedeemInviteStore } from "$houdini";
  import { deviceName } from "$lib/api.js";
  import { request } from "$lib/graphql.js";
  import { app, signIn } from "$lib/session.svelte.js";
  import BrandMark from "$components/BrandMark.svelte";

  // In the fragment, so the token never reaches server logs or a Referer header.
  const token = location.hash.slice(1);
  let form = $state({ username: "", password: "" });
  let busy = $state(false);
  let error = $state("");

  async function submit(event) {
    event.preventDefault();
    busy = true;
    error = "";
    try {
      const data = await request(new RedeemInviteStore(), { token, username: form.username.trim(), password: form.password, device: deviceName() }, { metadata: { noRefresh: true } });
      history.replaceState(null, "", "/invite");
      await signIn(data.redeemInvite);
      await goto("/");
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally { busy = false; }
  }
</script>

<svelte:head><title>Join · NoFlex</title></svelte:head>

<main class="invite" id="main">
  <BrandMark size={44} />
  {#if !token}
    <h1>This link is incomplete</h1>
    <p>Ask whoever sent it for a new invite.</p>
  {:else if app.viewer}
    <h1>You’re already signed in</h1>
    <p>Sign out first to make a new account with this invite.</p>
    <a class="btn" href="/">Back to NoFlex</a>
  {:else}
    <h1>You’re invited to {app.server?.name ?? "this server"}</h1>
    <p>Choose how you’ll sign in. Your account is only on this server.</p>
    <form class="stack" onsubmit={submit}>
      <label class="field">Username<input bind:value={form.username} autocomplete="username" required /></label>
      <label class="field">Password<input type="password" bind:value={form.password} autocomplete="new-password" minlength="8" required /><small>At least 8 characters.</small></label>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <button class="btn primary" disabled={busy || !form.username.trim() || form.password.length < 8}>{#if busy}<span class="spinner sm"></span>{/if}Create account</button>
    </form>
    <p class="hint">Already use NoFlex somewhere else? Afterwards, add this server under Account → Servers there.</p>
  {/if}
</main>

<style>
  .invite { display: grid; gap: 14px; justify-items: start; max-width: 420px; margin: 0 auto; padding: 72px 16px; }
  .invite h1 { margin: 8px 0 0; }
  .invite p { margin: 0; color: var(--text-2); }
  .invite form { width: 100%; }
  .error { color: var(--danger); }
  .hint { font-size: 0.86rem; color: var(--text-3); }
</style>
