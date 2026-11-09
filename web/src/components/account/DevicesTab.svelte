<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import Copy from "phosphor-svelte/lib/Copy";
  import Devices from "phosphor-svelte/lib/Devices";
  import Key from "phosphor-svelte/lib/Key";
  import MonitorPlay from "phosphor-svelte/lib/MonitorPlay";
  import Plus from "phosphor-svelte/lib/Plus";
  import Television from "phosphor-svelte/lib/Television";
  import { page } from "$app/state";
  import { graphql } from "$houdini";
  import { accessClaims, deviceName } from "$lib/api.js";
  import { deviceId } from "$lib/device.js";
  import { receiving, setReceiving } from "$lib/receiver.svelte.js";
  import { date } from "$lib/format.js";
  import { fetchQuery, refetch, request } from "$lib/graphql.js";
  import { app, logout } from "$lib/session.svelte.js";
  import { attempt, confirmAction, poll, toast } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import Select from "$components/ui/Select.svelte";
  import DeviceIcon from "$components/DeviceIcon.svelte";
  import { ago } from "./relative.js";

  let { account } = $props();

  const updateDevice = graphql(`
    mutation UpdateDeviceSmart($id: ID!, $smart: Boolean, $limit: Int, $clear: Boolean) {
      updateDevice(id: $id, smartTranscode: $smart, maxBitrateBps: $limit, clearMaxBitrate: $clear) { id name smartTranscode maxBitrateBps lastSeenAt }
    }
  `);
  const removeDevice = graphql(`
    mutation RemoveDevice($id: ID!) { removeDevice(id: $id) { id @Device_delete } }
  `);
  const QUALITY = [
    { value: "", label: "Best it can play" },
    { value: "20000000", label: "Up to 20 Mbps" },
    { value: "8000000", label: "Up to 8 Mbps" },
    { value: "4000000", label: "Up to 4 Mbps" },
  ];
  const thisDevice = deviceId();
  let devices = $derived($account.data?.myDevices ?? []);
  const setSmart = (device, smart) => attempt(() => updateDevice.mutate({ id: device.id, smart }), smart ? `Preparing titles for ${device.name} while the server is idle` : "Stopped preparing titles for this device");
  const setLimit = (device, value) => attempt(() => updateDevice.mutate(value ? { id: device.id, limit: Number(value) } : { id: device.id, clear: true }), "Quality limit saved");
  async function forget(device) {
    if (!(await confirmAction({ title: `Forget “${device.name}”?`, message: "The server stops preparing titles for it. It registers again the next time it signs in.", confirmLabel: "Forget device", danger: true }))) return;
    await attempt(() => removeDevice.mutate({ id: device.id }), "Device forgotten");
  }

  const revokeSession = graphql(`
    mutation RevokeSession($id: ID!) { revokeSession(id: $id) { id @Session_delete } }
  `);
  const createApiKey = graphql(`
    mutation CreateApiKey($name: String!) {
      createApiKey(name: $name) { key apiKey { id userId name createdAt revokedAt ...ApiKeys_insert } }
    }
  `);
  const revokeApiKey = graphql(`
    mutation RevokeApiKey($id: ID!) { revokeApiKey(id: $id) { id revokedAt } }
  `);
  const authorizeDeviceCode = graphql(`
    mutation AuthorizeDeviceCode($code: String!, $user: ID) { authorizeDeviceCode(code: $code, userId: $user) }
  `);
  const createDeviceCode = graphql(`
    mutation CreateDeviceCode($device: String!) { createDeviceCode(client: "NoFlex Web", device: $device) { code expiresAt } }
  `);
  const pollDeviceCode = graphql(`
    mutation PollDeviceCode($code: String!) { pollDeviceCode(code: $code) { token } }
  `);

  let data = $derived($account.data);
  let currentId = accessClaims().sid ?? null;
  let sessions = $derived([...(data?.mySessions ?? [])].sort((a, b) => (b.id === currentId) - (a.id === currentId) || Date.parse(b.lastSeenAt) - Date.parse(a.lastSeenAt)));
  let others = $derived(sessions.filter((session) => session.id !== currentId));
  let keys = $derived((data?.apiKeys ?? []).filter((key) => !key.revokedAt));
  let revokedCount = $derived((data?.apiKeys ?? []).length - keys.length);

  async function revoke(session) {
    if (!(await confirmAction({ title: `Sign out “${session.device || "this device"}”?`, message: "It will need to sign in again to keep using NoFlex.", confirmLabel: "Sign out device", danger: true }))) return;
    await attempt(() => revokeSession.mutate({ id: session.id }), "Device signed out");
  }

  async function revokeOthers() {
    if (!(await confirmAction({ title: `Sign out ${others.length} other ${others.length === 1 ? "device" : "devices"}?`, message: "Only this browser stays signed in.", confirmLabel: "Sign out others", danger: true }))) return;
    const results = await Promise.allSettled(others.map((session) => revokeSession.mutate({ id: session.id })));
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
    await attempt(() => revokeApiKey.mutate({ id: key.id }), "API key revoked");
  }

  // ---- Device codes ----
  // A TV's QR code carries the code it's showing.
  let code = $state(page.url.searchParams.get("code")?.toUpperCase() ?? "");
  let authorizing = $state(false);
  // An administrator can approve for someone with no device of their own to
  // approve from, a child's profile on the family TV being the usual one.
  const people = graphql(`
    query LinkablePeople { users { id name username } }
  `);
  $effect(() => { if (app.viewer.isAdmin) fetchQuery(people); });
  let approveFor = $state(app.viewer.id);
  let everyoneElse = $derived(($people.data?.users ?? []).filter((user) => user.id !== app.viewer.id));
  let approveChoices = $derived([
    { value: app.viewer.id, label: `${app.viewer.name} (you)` },
    ...everyoneElse.map((user) => ({ value: user.id, label: user.name, hint: user.name === user.username ? undefined : user.username })),
  ]);

  async function authorize(event) {
    event.preventDefault();
    const other = everyoneElse.find((user) => user.id === approveFor);
    authorizing = true;
    const result = await attempt(() => authorizeDeviceCode.mutate({ code: code.trim(), user: other?.id ?? null }));
    authorizing = false;
    if (!result) return;
    if (result.data.authorizeDeviceCode) {
      toast(other ? `Device approved for ${other.name} — it will finish signing in on its own` : "Device approved — it will finish signing in on its own", "success");
      code = "";
      approveFor = app.viewer.id;
      refetch(account);
    } else {
      toast("That code isn’t right, or it has expired", "error");
    }
  }

  let generated = $state(null);
  let generating = $state(false);
  let approved = $state(false);
  let clock = $state(Date.now());
  let remaining = $derived(generated ? Math.max(0, Date.parse(generated.expiresAt) - clock) : 0);

  async function generate() {
    generating = true;
    const result = await attempt(() => createDeviceCode.mutate({ device: deviceName() }));
    generating = false;
    if (result) { generated = result.data.createDeviceCode; approved = false; clock = Date.now(); }
  }

  async function check() {
    if (!generated || approved) return;
    try {
      const result = await request(pollDeviceCode, { code: generated.code }, { metadata: { noRefresh: true } });
      if (result.pollDeviceCode) { approved = true; toast("Code approved and used", "success"); refetch(account); }
    } catch { /* pending codes can error until approved; keep waiting */ }
  }

  $effect(() => {
    if (!generated || approved) return;
    const tick = setInterval(() => (clock = Date.now()), 1000);
    const stop = poll(check, 4000);
    return () => { clearInterval(tick); stop(); };
  });
  $effect(() => { if (generated && !approved && remaining === 0) { generated = null; toast("That code expired", "info"); } });

  const countdown = (ms) => `${Math.floor(ms / 60_000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
</script>

{#if ($account.fetching && !$account.data)}
  <div class="grid" aria-busy="true">
    <div class="panel"><div class="skeleton row-skel"></div><div class="skeleton row-skel short"></div></div>
    <div class="panel"><div class="skeleton row-skel"></div><div class="skeleton row-skel short"></div></div>
    <div class="panel wide">{#each Array(3) as _}<div class="skeleton row-skel"></div>{/each}</div>
  </div>
{:else if data}
  <div class="grid stagger">
    <section class="panel" style:--i={0}>
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

    <section class="panel" style:--i={1}>
      <header class="panel-head">
        <span class="panel-icon"><Television size={19} /></span>
        <div><h2>Link a device</h2><p>Approve the code shown on a TV or another screen</p></div>
      </header>
      <form class="inline-form" onsubmit={authorize}>
        <label class="field grow">Device code<input class="code-input" bind:value={code} oninput={() => (code = code.toUpperCase())} required autocapitalize="characters" autocomplete="off" placeholder="K7M2QX" spellcheck="false" /></label>
        {#if everyoneElse.length}<label class="field" for="approve-for">Sign in as<Select id="approve-for" bind:value={approveFor} items={approveChoices} label="Sign in as" /></label>{/if}
        <button class="btn primary sm" disabled={authorizing || !code.trim()}>{#if authorizing}<span class="spinner sm"></span>{/if}Approve</button>
      </form>
      <div class="divider"></div>
      {#if generated}
        <div class="code-card fade-up" aria-live="polite">
          <span class="eyebrow">{approved ? "Approved" : "Enter on a signed-in device"}</span>
          <strong class="link-code mono" class:done={approved}>{generated.code}</strong>
          {#if approved}
            <small><CheckCircle size={14} />This code has been used.</small>
          {:else}
            <small class="tabular"><span class="spinner sm"></span>Waiting for approval · expires in {countdown(remaining)}</small>
          {/if}
          <div class="row">
            {#if !approved}<button class="btn ghost sm" onclick={check}>Check now</button>{/if}
            <button class="btn ghost sm" onclick={() => (generated = null)}>Dismiss</button>
          </div>
        </div>
      {:else}
        <div class="code-offer">
          <p>Need a code for this browser instead?</p>
          <button class="btn outline sm" onclick={generate} disabled={generating}>{#if generating}<span class="spinner sm"></span>{/if}Generate a code</button>
        </div>
      {/if}
    </section>

    <section class="panel wide" style:--i={2}>
      <header class="panel-head">
        <span class="panel-icon"><MonitorPlay size={19} /></span>
        <div><h2>Play on this browser</h2><p>Let your other devices start and steer a title here, from their “Play on” button</p></div>
      </header>
      <label class="toggle"><input type="checkbox" class="switch" checked={receiving.on} onchange={(event) => setReceiving(event.currentTarget.checked)} /><span class="what"><strong>Allow other devices to play on this browser</strong><small>Only your own devices. It works while a NoFlex tab is open, and browsers hold back sound until the page has been used.</small></span></label>
    </section>

    <section class="panel wide" style:--i={3}>
      <header class="panel-head">
        <span class="panel-icon"><Devices size={19} /></span>
        <div><h2>Smart transcoding</h2><p>For these devices, the server prepares what you’re likely to watch next while it’s idle, so playback starts without a live transcode, and <a class="inline" href="/account?tab=smart">what you’ve marked</a> before anything else</p></div>
      </header>
      {#if devices.length}
        <ul class="list smart-devices">
          {#each devices as device (device.id)}
            <li>
              <label class="toggle"><input type="checkbox" class="switch" checked={device.smartTranscode} onchange={(event) => setSmart(device, event.currentTarget.checked)} /><span class="what"><strong>{device.name}{#if device.id === thisDevice}<span class="chip accent">This browser</span>{/if}</strong><small>Last seen {ago(device.lastSeenAt)}</small></span></label>
              <Select value={device.maxBitrateBps ? String(device.maxBitrateBps) : ""} items={QUALITY} label="Quality limit for {device.name}" onchange={(value) => setLimit(device, value)} />
              {#if device.id !== thisDevice}<button class="btn ghost sm" onclick={() => forget(device)} aria-label="Forget {device.name}">Forget</button>{/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="faint">This device registers itself once it has measured what it can play.</p>
      {/if}
    </section>

    <section class="panel wide" style:--i={3}>
      <header class="panel-head">
        <span class="panel-icon"><Devices size={19} /></span>
        <div><h2>Signed-in devices</h2><p>{sessions.length} active {sessions.length === 1 ? "session" : "sessions"}</p></div>
        <span class="spacer"></span>
        {#if others.length}<button class="btn ghost sm" onclick={revokeOthers}>Sign out all others</button>{/if}
      </header>
      <ul class="list">
        {#each sessions as session (session.id)}
          <li class:current={session.id === currentId}>
            <span class="glyph"><DeviceIcon device={session.device} client={session.client} /></span>
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
  /* Fixed columns keep the selects lined up whether or not a row can be forgotten. */
  .smart-devices li { display: grid; grid-template-columns: minmax(0, 1fr) 190px 72px; }
  .grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 18px; align-items: start; }
  @media (max-width: 820px) { .grid { grid-template-columns: minmax(0, 1fr); } }
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
  a.inline { color: var(--text); text-decoration: underline; text-underline-offset: 3px; }
  .inline-form { display: flex; gap: 10px; align-items: flex-end; margin-top: 14px; }
  .inline-form .grow { flex: 1; }
  .inline-form .btn { height: 42px; }
  .code-input { font-family: ui-monospace, "SF Mono", Menlo, monospace; letter-spacing: 0.3em; font-size: 1.05rem !important; }
  .code-offer { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .code-offer p { color: var(--text-3); font-size: 0.88rem; }
  .code-card { display: grid; justify-items: center; gap: 8px; padding: 18px; border-radius: var(--r); background: var(--surface); border: 1px dashed var(--line-strong); text-align: center; }
  .link-code { font-size: 2.1rem; font-weight: 700; letter-spacing: 0.28em; padding-left: 0.28em; color: var(--text); }
  .link-code.done { color: var(--success); }
  .code-card small { display: inline-flex; align-items: center; gap: 8px; color: var(--text-3); font-size: 0.84rem; }
  .secret { display: grid; gap: 10px; }
  .secret code { display: block; padding: 16px; border-radius: var(--r); background: rgb(0 0 0 / 0.35); border: 1px solid var(--line); color: var(--accent); font-size: 0.92rem; word-break: break-all; user-select: all; }
  @media (max-width: 620px) {
    .list li { flex-wrap: wrap; }
    .smart-devices li { grid-template-columns: minmax(0, 1fr) auto; }
    .smart-devices .toggle { grid-column: 1 / -1; }
    .seen { justify-items: start; text-align: left; order: 3; flex-basis: 100%; padding-left: 52px; }
  }
</style>
