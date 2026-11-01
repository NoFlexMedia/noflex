<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import HardDrives from "phosphor-svelte/lib/HardDrives";
  import Check from "phosphor-svelte/lib/Check";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import Copy from "phosphor-svelte/lib/Copy";
  import Devices from "phosphor-svelte/lib/Devices";
  import Key from "phosphor-svelte/lib/Key";
  import Plus from "phosphor-svelte/lib/Plus";
  import Television from "phosphor-svelte/lib/Television";
  import { cache, graphql } from "$houdini";
  import { accessClaims, deviceName } from "$lib/api.js";
  import { date } from "$lib/format.js";
  import { refetch, request } from "$lib/graphql.js";
  import { logout } from "$lib/session.svelte.js";
  import { attempt, confirmAction, poll, toast } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import { ago } from "./relative.js";

  let { account } = $props();

  const revokeSession = graphql(`
    mutation RevokeSession($id: ID!) { revokeSession(id: $id) }
  `);
  const createApiKey = graphql(`
    mutation CreateApiKey($name: String!) {
      createApiKey(name: $name) { key apiKey { id userId name createdAt revokedAt ...ApiKeys_insert } }
    }
  `);
  const revokeApiKey = graphql(`
    mutation RevokeApiKey($id: ID!) { revokeApiKey(id: $id) }
  `);
  const authorizeDevicePin = graphql(`
    mutation AuthorizeDevicePin($pin: String!) { authorizeDevicePin(pin: $pin) }
  `);
  const createDevicePin = graphql(`
    mutation CreateDevicePin($device: String!) { createDevicePin(client: "NoFlex Web", device: $device) { pin expiresAt } }
  `);
  const pollDevicePin = graphql(`
    mutation PollDevicePin($pin: String!) { pollDevicePin(pin: $pin) { token } }
  `);
  const dropSession = (id) => cache.get("Session", { id }).delete();

  let data = $derived($account.data);
  let currentId = accessClaims().sid ?? null;
  let sessions = $derived([...(data?.mySessions ?? [])].sort((a, b) => (b.id === currentId) - (a.id === currentId) || Date.parse(b.lastSeenAt) - Date.parse(a.lastSeenAt)));
  let others = $derived(sessions.filter((session) => session.id !== currentId));
  let keys = $derived((data?.apiKeys ?? []).filter((key) => !key.revokedAt));
  let revokedCount = $derived((data?.apiKeys ?? []).length - keys.length);

  const deviceIcon = (session) => (/tv|roku|fire|android tv|apple tv/i.test(`${session.device} ${session.client}`) ? Television : /iphone|ipad|android|ios/i.test(session.device ?? "") ? Devices : HardDrives);

  async function revoke(session) {
    if (!(await confirmAction({ title: `Sign out “${session.device || "this device"}”?`, message: "It will need to sign in again to keep using NoFlex.", confirmLabel: "Sign out device", danger: true }))) return;
    if (await attempt(() => revokeSession.mutate({ id: session.id }), "Device signed out")) dropSession(session.id);
  }

  async function revokeOthers() {
    if (!(await confirmAction({ title: `Sign out ${others.length} other ${others.length === 1 ? "device" : "devices"}?`, message: "Only this browser stays signed in.", confirmLabel: "Sign out others", danger: true }))) return;
    const results = await Promise.allSettled(others.map((session) => revokeSession.mutate({ id: session.id }).then(() => dropSession(session.id))));
    const failed = results.filter((result) => result.status === "rejected").length;
    toast(failed ? `${failed} ${failed === 1 ? "device" : "devices"} couldn’t be signed out` : "Other devices signed out", failed ? "error" : "success");
  }

  // ---- API keys ----
  let keyName = $state("");
  let creating = $state(false);
  let secret = $state({ open: false, key: "", name: "", copied: false });

  async function createKey(event) {
    event.preventDefault();
    creating = true;
    const result = (await attempt(() => createApiKey.mutate({ name: keyName.trim() })))?.data;
    creating = false;
    if (!result) return;
    secret = { open: true, key: result.createApiKey.key, name: result.createApiKey.apiKey.name, copied: false };
    keyName = "";
  }

  async function copySecret() {
    try {
      await navigator.clipboard.writeText(secret.key);
      secret.copied = true;
      setTimeout(() => (secret.copied = false), 2000);
    } catch { toast("Couldn’t reach the clipboard — select the key and copy it manually", "error"); }
  }

  async function revokeKey(key) {
    if (!(await confirmAction({ title: `Revoke “${key.name}”?`, message: "Anything using this key stops working immediately.", confirmLabel: "Revoke key", danger: true }))) return;
    if (await attempt(() => revokeApiKey.mutate({ id: key.id }), "API key revoked")) refetch(account);
  }

  // ---- Device PINs ----
  let pin = $state("");
  let authorizing = $state(false);

  async function authorize(event) {
    event.preventDefault();
    authorizing = true;
    const ok = await attempt(() => authorizeDevicePin.mutate({ pin: pin.trim() }), "Device approved — it will finish signing in on its own");
    authorizing = false;
    if (ok) { pin = ""; refetch(account); }
  }

  let generated = $state(null);
  let generating = $state(false);
  let approved = $state(false);
  let clock = $state(Date.now());
  let remaining = $derived(generated ? Math.max(0, Date.parse(generated.expiresAt) - clock) : 0);

  async function generate() {
    generating = true;
    const result = await attempt(() => createDevicePin.mutate({ device: deviceName() }));
    generating = false;
    if (result) { generated = result.data.createDevicePin; approved = false; clock = Date.now(); }
  }

  async function check() {
    if (!generated || approved) return;
    try {
      const result = await request(pollDevicePin, { pin: generated.pin }, { metadata: { noRefresh: true } });
      if (result.pollDevicePin) { approved = true; toast("PIN approved and used", "success"); refetch(account); }
    } catch { /* pending PINs can error until approved; keep waiting */ }
  }

  $effect(() => {
    if (!generated || approved) return;
    const tick = setInterval(() => (clock = Date.now()), 1000);
    const stop = poll(check, 4000);
    return () => { clearInterval(tick); stop(); };
  });
  $effect(() => { if (generated && !approved && remaining === 0) { generated = null; toast("That PIN expired", "info"); } });

  const countdown = (ms) => `${Math.floor(ms / 60_000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
</script>

{#if ($account.fetching && !$account.data)}
  <div class="grid" aria-busy="true">
    <div class="panel wide">{#each Array(3) as _}<div class="skeleton row-skel"></div>{/each}</div>
    <div class="panel"><div class="skeleton row-skel"></div><div class="skeleton row-skel short"></div></div>
    <div class="panel"><div class="skeleton row-skel"></div><div class="skeleton row-skel short"></div></div>
  </div>
{:else if data}
  <div class="grid stagger">
    <section class="panel wide" style:--i={0}>
      <header class="panel-head">
        <span class="panel-icon"><Devices size={19} /></span>
        <div><h2>Signed-in devices</h2><p>{sessions.length} active {sessions.length === 1 ? "session" : "sessions"}</p></div>
        <span class="spacer"></span>
        {#if others.length}<button class="btn ghost sm" onclick={revokeOthers}>Sign out all others</button>{/if}
      </header>
      <ul class="list">
        {#each sessions as session (session.id)}
          {@const Glyph = deviceIcon(session)}
          <li class:current={session.id === currentId}>
            <span class="glyph"><Glyph size={18} /></span>
            <span class="what">
              <strong>{session.device || "Unknown device"}{#if session.id === currentId}<span class="chip accent">This browser</span>{/if}</strong>
              <small>{[session.client, session.appVersion && `v${session.appVersion}`].filter(Boolean).join(" ") || "NoFlex client"} · signed in {date(session.createdAt)}</small>
            </span>
            <span class="seen">
              <time datetime={session.lastSeenAt} title={date(session.lastSeenAt, { dateStyle: "medium", timeStyle: "short" })}>{session.id === currentId ? "Active now" : `Seen ${ago(session.lastSeenAt)}`}</time>
              <code>{session.remoteAddr || "local"}</code>
            </span>
            {#if session.id === currentId}
              <button class="btn ghost sm" onclick={logout}>Sign out</button>
            {:else}
              <button class="btn ghost sm" onclick={() => revoke(session)} aria-label="Sign out {session.device || 'unknown device'}">Revoke</button>
            {/if}
          </li>
        {:else}
          <li><p class="empty-note">No active sessions.</p></li>
        {/each}
      </ul>
    </section>

    <section class="panel" style:--i={1}>
      <header class="panel-head">
        <span class="panel-icon"><Key size={19} /></span>
        <div><h2>API keys</h2><p>For scripts, automations and trusted apps</p></div>
      </header>
      {#if keys.length}
        <ul class="list keys">
          {#each keys as key (key.id)}
            <li>
              <span class="what"><strong>{key.name}</strong><small>Created {date(key.createdAt)}</small></span>
              <button class="btn ghost sm" onclick={() => revokeKey(key)} aria-label="Revoke {key.name}">Revoke</button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty-note">No active keys{revokedCount ? ` · ${revokedCount} revoked` : ""}.</p>
      {/if}
      <form class="inline-form" onsubmit={createKey}>
        <label class="field grow">New key name<input bind:value={keyName} required maxlength="80" placeholder="Home Assistant" autocomplete="off" /></label>
        <button class="btn sm" disabled={creating || !keyName.trim()}>{#if creating}<span class="spinner sm"></span>{:else}<Plus size={15} />{/if}Create</button>
      </form>
    </section>

    <section class="panel" style:--i={2}>
      <header class="panel-head">
        <span class="panel-icon"><Television size={19} /></span>
        <div><h2>Link a device</h2><p>Approve the code shown on a TV or another screen</p></div>
      </header>
      <form class="inline-form" onsubmit={authorize}>
        <label class="field grow">Device PIN<input class="pin-input" bind:value={pin} required inputmode="numeric" autocomplete="one-time-code" placeholder="123456" spellcheck="false" /></label>
        <button class="btn primary sm" disabled={authorizing || !pin.trim()}>{#if authorizing}<span class="spinner sm"></span>{/if}Approve</button>
      </form>
      <div class="divider"></div>
      {#if generated}
        <div class="pin-card fade-up" aria-live="polite">
          <span class="eyebrow">{approved ? "Approved" : "Enter on a signed-in device"}</span>
          <strong class="pin mono" class:done={approved}>{generated.pin}</strong>
          {#if approved}
            <small><CheckCircle size={14} />This PIN has been used.</small>
          {:else}
            <small class="tabular"><span class="spinner sm"></span>Waiting for approval · expires in {countdown(remaining)}</small>
          {/if}
          <div class="row">
            {#if !approved}<button class="btn ghost sm" onclick={check}>Check now</button>{/if}
            <button class="btn ghost sm" onclick={() => (generated = null)}>Dismiss</button>
          </div>
        </div>
      {:else}
        <div class="pin-offer">
          <p>Need a code for this browser instead?</p>
          <button class="btn outline sm" onclick={generate} disabled={generating}>{#if generating}<span class="spinner sm"></span>{/if}Generate a PIN</button>
        </div>
      {/if}
    </section>
  </div>
{/if}

<Dialog bind:open={secret.open} title="Your new API key" description="Copy it now — NoFlex stores only a hash, so this is the only time it’s shown." width={500} onclose={() => (secret = { open: false, key: "", name: "", copied: false })}>
  <div class="secret">
    <span class="eyebrow">{secret.name}</span>
    <code class="mono" aria-label="API key">{secret.key}</code>
  </div>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (secret.open = false)}>Done</button>
    <button class="btn primary" onclick={copySecret}>{#if secret.copied}<Check size={16} />{:else}<Copy size={16} />{/if}{secret.copied ? "Copied" : "Copy key"}</button>
  {/snippet}
</Dialog>

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 360px), 1fr)); gap: 18px; align-items: start; }
  .wide { grid-column: 1 / -1; }
  .row-skel { height: 44px; margin-top: 12px; border-radius: 10px; }
  .row-skel.short { width: 60%; }
  .glyph { display: grid; place-items: center; width: 38px; height: 38px; border-radius: 11px; background: var(--surface-2); color: var(--text-2); flex: none; }
  .current .glyph { background: var(--accent-soft); color: var(--accent); }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what strong { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .what .chip { height: 21px; font-size: 0.7rem; }
  .what small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .seen { display: grid; justify-items: end; gap: 2px; text-align: right; }
  .seen code { color: var(--text-3); font-size: 0.78rem; }
  .seen time { color: var(--text-3); font-size: 0.83rem; }
  .keys li { padding-block: 10px; }
  .inline-form { display: flex; gap: 10px; align-items: flex-end; margin-top: 14px; }
  .inline-form .grow { flex: 1; }
  .inline-form .btn { height: 42px; }
  .pin-input { font-family: ui-monospace, "SF Mono", Menlo, monospace; letter-spacing: 0.3em; font-size: 1.05rem !important; }
  .pin-offer { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .pin-offer p { color: var(--text-3); font-size: 0.88rem; }
  .pin-card { display: grid; justify-items: center; gap: 8px; padding: 18px; border-radius: var(--r); background: var(--surface); border: 1px dashed var(--line-strong); text-align: center; }
  .pin { font-size: 2.1rem; font-weight: 700; letter-spacing: 0.28em; padding-left: 0.28em; color: var(--text); }
  .pin.done { color: var(--success); }
  .pin-card small { display: inline-flex; align-items: center; gap: 8px; color: var(--text-3); font-size: 0.84rem; }
  .secret { display: grid; gap: 10px; }
  .secret code { display: block; padding: 16px; border-radius: var(--r); background: rgb(0 0 0 / 0.35); border: 1px solid var(--line); color: var(--accent); font-size: 0.92rem; word-break: break-all; user-select: all; }
  @media (max-width: 620px) {
    .list li { flex-wrap: wrap; }
    .seen { justify-items: start; text-align: left; order: 3; flex-basis: 100%; padding-left: 52px; }
  }
</style>
