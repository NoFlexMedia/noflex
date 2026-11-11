<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowRight from "phosphor-svelte/lib/ArrowRight";
  import Key from "phosphor-svelte/lib/Key";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { CompleteSetupStore, ExternalProvidersStore, LoginStore, StartExternalLoginStore } from "$houdini";
  import { deviceName, newExternalNonce } from "$lib/api.js";
  import { request } from "$lib/graphql.js";
  import { app, signIn, start } from "$lib/session.svelte.js";
  import { ApiError } from "$lib/tokens.js";
  import BrandMark from "$components/BrandMark.svelte";

  let setup = $derived(app.phase === "setup");
  let username = $state("");
  let password = $state("");
  let confirmPassword = $state("");
  let error = $state(app.authError);
  let busy = $state(false);
  let shake = $state(0);
  let providers = $state([]);

  // Eight drifting columns of tiles; hues are spread so the wall reads as
  // a shelf of artwork without needing (unauthenticated) image access.
  const columns = Array.from({ length: 8 }, (_, column) =>
    Array.from({ length: 7 }, (_, row) => ({ hue: (column * 47 + row * 83) % 360, tall: (column + row) % 3 === 0 })),
  );

  $effect(() => {
    if (setup) return;
    request(new ExternalProvidersStore(), {}, { metadata: { noRefresh: true } })
      .then((data) => { providers = data.externalAuthProviders; })
      .catch(() => { providers = []; });
  });

  function fail(message) { error = message; shake++; }

  async function submit(event) {
    event.preventDefault();
    if (setup && password !== confirmPassword) return fail("Those passwords don’t match.");
    busy = true; error = "";
    try {
      const options = { metadata: { noRefresh: true } };
      const data = setup
        ? await request(new CompleteSetupStore(), { username, password }, options)
        : await request(new LoginStore(), { username, password, device: deviceName() }, options);
      await signIn(setup ? data.completeSetupWizard : data.login);
    } catch (cause) {
      if (cause instanceof ApiError && cause.codes.includes("SERVER_RESETTING")) return void start();
      fail(cause.message);
    } finally {
      busy = false;
    }
  }

  async function external(provider) {
    error = "";
    try {
      const data = await request(new StartExternalLoginStore(), { provider, nonce: newExternalNonce() }, { metadata: { noRefresh: true } });
      location.assign(data.startExternalLogin.authorizationUrl);
    } catch (cause) { fail(cause.message); }
  }
</script>

<svelte:head><title>{setup ? "Set up NoFlex" : "Sign in · NoFlex"}</title></svelte:head>

<main class="auth" id="main">
  <div class="wall" aria-hidden="true">
    <div class="plane">
      {#each columns as column, index}
        <div class="column" style:--speed="{48 + (index % 3) * 14}s" style:--dir={index % 2 ? "reverse" : "normal"}>
          {#each [...column, ...column] as tile}
            <span class="tile" class:tall={tile.tall} style:--h={tile.hue}></span>
          {/each}
        </div>
      {/each}
    </div>
  </div>
  <div class="vignette" aria-hidden="true"></div>

  <section class="intro">
    <span class="brand"><BrandMark size={36} />NoFlex</span>
    <h1>{setup ? "Make this server yours." : "Everything you keep,"}<br /><em>{setup ? "Start in a minute." : "beautifully in reach."}</em></h1>
    <p>{setup ? "Create the first administrator, then point NoFlex at your media. Metadata, artwork and search take it from there." : "Films, series, music and photos from your own server — no ads, no feeds, no one else’s algorithm."}</p>
  </section>

  <form class="panel-glass" onsubmit={submit}>
    {#key shake}<div class="shake-host" class:shake={shake > 0}>
      <p class="eyebrow">{setup ? "First run" : "Welcome back"}</p>
      <h2>{setup ? "Create the administrator" : "Sign in"}</h2>
      {#if app.server}<p class="server"><span class="dot on"></span>{app.server.name} · v{app.server.version}</p>{/if}

      <label class="field">Username<input bind:value={username} autocomplete="username" required autocapitalize="off" spellcheck="false" /></label>
      <label class="field">Password<input bind:value={password} type="password" autocomplete={setup ? "new-password" : "current-password"} minlength={setup ? 8 : undefined} required />{#if setup}<small>At least 8 characters.</small>{/if}</label>
      {#if setup}<label class="field">Confirm password<input bind:value={confirmPassword} type="password" autocomplete="new-password" required /></label>{/if}

      {#if error}<p class="error" role="alert"><WarningCircle size={16} />{error}</p>{/if}

      <button class="btn primary lg submit" type="submit" disabled={busy}>
        {#if busy}<span class="spinner sm"></span>{setup ? "Creating…" : "Signing in…"}{:else}{setup ? "Create administrator" : "Sign in"}<ArrowRight size={18} />{/if}
      </button>

      {#if !setup && providers.length}
        <div class="or"><span>or</span></div>
        <div class="providers">
          {#each providers as provider}<button class="btn outline" type="button" onclick={() => external(provider.id)}><Key size={16} />Continue with {provider.displayName}</button>{/each}
        </div>
      {/if}
    </div>{/key}
  </form>
</main>

<style>
  .auth { position: relative; min-height: 100dvh; display: grid; grid-template-columns: minmax(0, 1.1fr) minmax(340px, 440px); align-items: center; gap: clamp(32px, 6vw, 96px); padding: 48px clamp(20px, 7vw, 120px); overflow: hidden; isolation: isolate; }
  .wall { position: absolute; inset: -10%; z-index: -2; perspective: 1400px; overflow: hidden; }
  .plane { position: absolute; inset: -20% -10%; display: flex; gap: 18px; justify-content: center; transform: rotateX(32deg) rotateZ(-14deg) translateZ(-120px); transform-style: preserve-3d; }
  .column { display: flex; flex-direction: column; gap: 18px; width: 170px; flex: none; animation: drift var(--speed) linear infinite; animation-direction: var(--dir); will-change: transform; }
  @keyframes drift { to { transform: translateY(-50%); } }
  .tile { flex: none; height: 240px; border-radius: 16px; background: radial-gradient(120% 70% at 20% 10%, hsl(var(--h) 55% 38%), transparent 65%), radial-gradient(90% 80% at 90% 100%, hsl(calc(var(--h) + 50) 45% 24%), transparent 70%), hsl(var(--h) 30% 12%); box-shadow: 0 30px 60px -30px rgb(0 0 0 / 0.9), inset 0 1px 0 rgb(255 255 255 / 0.08); }
  .tile.tall { height: 300px; }
  .vignette { position: absolute; inset: 0; z-index: -1; background: radial-gradient(80% 90% at 75% 50%, rgb(7 8 11 / 0.55), rgb(7 8 11 / 0.92) 70%), linear-gradient(90deg, rgb(7 8 11 / 0.92) 10%, rgb(7 8 11 / 0.4) 55%, rgb(7 8 11 / 0.75)); }
  .intro { max-width: 720px; }
  .brand { display: inline-flex; align-items: center; gap: 12px; font-size: 1.3rem; font-weight: 750; letter-spacing: -0.03em; margin-bottom: clamp(32px, 8vh, 72px); }
  h1 { font-size: clamp(2.2rem, 4.3vw, 3.9rem); font-weight: 700; letter-spacing: -0.045em; line-height: 1.04; text-wrap: nowrap; }
  h1 em { font-style: normal; color: transparent; background: linear-gradient(100deg, var(--accent), #8de2c5 60%, #9fb8ff); -webkit-background-clip: text; background-clip: text; }
  .intro > p { margin-top: 22px; font-size: 1.08rem; color: var(--text-2); max-width: 500px; }
  .panel-glass { padding: 34px; border-radius: 28px; background: rgb(16 18 23 / 0.72); backdrop-filter: blur(28px) saturate(1.5); border: 1px solid var(--line-strong); box-shadow: var(--shadow-3); animation: rise 700ms var(--ease-out) both 120ms; }
  @keyframes rise { from { opacity: 0; transform: translateY(18px) scale(0.98); } }
  .shake-host { display: grid; gap: 16px; }
  .shake { animation: shake 420ms var(--ease-out); }
  @keyframes shake { 20% { transform: translateX(-8px); } 40% { transform: translateX(7px); } 60% { transform: translateX(-4px); } 80% { transform: translateX(2px); } }
  h2 { font-size: 1.6rem; margin-top: -8px; }
  .server { display: flex; align-items: center; gap: 9px; font-size: 0.84rem; color: var(--text-3); margin: -6px 0 6px; }
  .error { display: flex; gap: 9px; align-items: flex-start; padding: 11px 13px; border-radius: 12px; background: var(--danger-soft); color: #ffb3b3; font-size: 0.88rem; }
  .error :global(svg) { flex: none; margin-top: 2px; }
  .submit { width: 100%; margin-top: 4px; }
  .or { display: flex; align-items: center; gap: 12px; color: var(--text-3); font-size: 0.8rem; }
  .or::before, .or::after { content: ""; flex: 1; height: 1px; background: var(--line); }
  .providers { display: grid; gap: 8px; }
  @media (max-width: 900px) {
    .auth { grid-template-columns: 1fr; align-content: center; padding: 32px 18px; gap: 28px; }
    .brand { margin-bottom: 20px; }
    .intro > p { display: none; }
    h1 { font-size: clamp(2rem, 9vw, 2.8rem); }
    .panel-glass { padding: 24px; }
  }
  @media (prefers-reduced-motion: reduce) { .column { animation: none; } }
</style>
