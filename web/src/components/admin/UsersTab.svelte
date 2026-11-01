<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import Books from "phosphor-svelte/lib/Books";
  import Devices from "phosphor-svelte/lib/Devices";
  import DotsThree from "phosphor-svelte/lib/DotsThree";
  import Plus from "phosphor-svelte/lib/Plus";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import SignOut from "phosphor-svelte/lib/SignOut";
  import Trash from "phosphor-svelte/lib/Trash";
  import Users from "phosphor-svelte/lib/Users";
  import X from "phosphor-svelte/lib/X";
  import { untrack } from "svelte";
  import { cache, graphql } from "$houdini";
  import { date, titleCase } from "$lib/format.js";
  import { refetch } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt, confirmAction } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Select from "$components/ui/Select.svelte";
  import { ago } from "$components/account/relative.js";
  import RestrictionsSheet from "./RestrictionsSheet.svelte";

  /** `admin` is the page's Admin store. */
  let { admin, creating = $bindable(false) } = $props();

  const createUser = graphql(`
    mutation CreateUser($username: String!, $password: String!, $role: UserRole!) {
      createUser(username: $username, password: $password, role: $role) { ...UserFields ...Users_insert }
    }
  `);
  const updateUser = graphql(`
    mutation UpdateUser($id: ID!, $role: UserRole, $libraries: [ID!]) {
      updateUser(id: $id, role: $role, libraryIds: $libraries) { ...UserFields }
    }
  `);
  const revokeUserSessions = graphql(`
    mutation RevokeUserSessions($user: ID!) { revokeUserSessions(userId: $user) }
  `);
  const deleteUser = graphql(`
    mutation DeleteUser($id: ID!) { deleteUser(id: $id) }
  `);
  const revokeSessionMutation = graphql(`
    mutation RevokeUserSession($id: ID!) { revokeSession(id: $id) }
  `);

  const ROLES = [
    { value: "USER", label: "User", hint: "Watches granted libraries" },
    { value: "GUEST", label: "Guest", hint: "Limited, read-only" },
    { value: "ADMIN", label: "Administrator", hint: "Manages the server" },
  ];

  let data = $derived($admin.data);
  let users = $derived([...(data?.users ?? [])].sort((a, b) => (b.id === app.viewer.id) - (a.id === app.viewer.id) || a.name.localeCompare(b.name)));
  let libraries = $derived(data?.libraries ?? []);
  let sessions = $derived([...(data?.sessions ?? [])].sort((a, b) => Date.parse(b.lastSeenAt) - Date.parse(a.lastSeenAt)));
  let names = $derived(new Map((data?.users ?? []).map((user) => [user.id, user.name])));
  let sessionCount = $derived(sessions.reduce((counts, session) => counts.set(session.userId, (counts.get(session.userId) ?? 0) + 1), new Map()));

  // ---- Create ----
  let draft = $state({ username: "", password: "", role: "USER" });
  let saving = $state(false);
  $effect(() => { if (creating) untrack(() => (draft = { username: "", password: "", role: "USER" })); });

  async function create(event) {
    event.preventDefault();
    saving = true;
    const result = await attempt(() => createUser.mutate({ username: draft.username.trim(), password: draft.password, role: draft.role }), `${draft.username.trim()} can now sign in`);
    saving = false;
    if (!result) return;
    creating = false;
    const user = result.data.createUser;
    if (user.role !== "ADMIN" && libraries.length) openAccess(user);
  }

  // ---- Access ----
  let access = $state({ open: false, user: null, role: "USER", libraries: [] });
  function openAccess(user) { access = { open: true, user, role: user.role, libraries: [...user.libraryAccess] }; }
  function toggleLibrary(id) { access.libraries = access.libraries.includes(id) ? access.libraries.filter((item) => item !== id) : [...access.libraries, id]; }

  async function saveAccess(event) {
    event.preventDefault();
    saving = true;
    const result = await attempt(() => updateUser.mutate({ id: access.user.id, role: access.role, libraries: access.libraries }), `Access updated for ${access.user.name}`);
    saving = false;
    if (result) access.open = false;
  }

  // ---- Restrictions ----
  let restricting = $state({ open: false, user: null });

  // ---- Destructive ----
  async function revokeAll(user) {
    if (!(await confirmAction({ title: `Sign ${user.name} out everywhere?`, message: "Every device they’re using will need to sign in again.", confirmLabel: "Sign out everywhere", danger: true }))) return;
    if (await attempt(() => revokeUserSessions.mutate({ user: user.id }), `${user.name} signed out everywhere`)) refetch(admin);
  }

  async function remove(user) {
    if (!(await confirmAction({ title: `Delete ${user.name}?`, message: "Their account, sessions, playlists and watch history are removed. This can’t be undone.", confirmLabel: "Delete user", danger: true }))) return;
    if (!(await attempt(() => deleteUser.mutate({ id: user.id }), `${user.name} deleted`))) return;
    for (const session of sessions) if (session.userId === user.id) cache.get("Session", { id: session.id }).delete();
    cache.get("User", { id: user.id }).delete();
  }

  async function revokeSession(session) {
    const who = names.get(session.userId) ?? "this user";
    if (!(await confirmAction({ title: `Sign out ${session.device || "this device"}?`, message: `${who} will need to sign in again on it.`, confirmLabel: "Sign out device", danger: true }))) return;
    if (await attempt(() => revokeSessionMutation.mutate({ id: session.id }), "Session revoked")) cache.get("Session", { id: session.id }).delete();
  }

  const grants = (user) => (user.role === "ADMIN" ? "All libraries" : `${user.libraryAccess.length} of ${libraries.length} ${libraries.length === 1 ? "library" : "libraries"}`);
</script>

{#if ($admin.fetching && !$admin.data)}
  <div class="layout" aria-busy="true">
    <div class="panel">{#each Array(4) as _}<div class="row-skel"><div class="skeleton avatar-skel"></div><div class="skeleton bar"></div></div>{/each}</div>
    <div class="panel">{#each Array(3) as _}<div class="row-skel"><div class="skeleton bar"></div></div>{/each}</div>
  </div>
{:else if data}
  <div class="layout stagger">
    <section class="panel" style:--i={0}>
      <header class="panel-head">
        <span class="panel-icon"><Users size={19} /></span>
        <div><h2>Accounts</h2><p>{users.length} {users.length === 1 ? "person" : "people"} can sign in</p></div>
        <span class="spacer"></span>
        <button class="btn sm" onclick={() => (creating = true)}><Plus size={15} />Add</button>
      </header>
      <ul class="list">
        {#each users as user (user.id)}
          <li>
            <span class="avatar" aria-hidden="true">{user.name.slice(0, 2).toUpperCase()}</span>
            <span class="what">
              <strong>{user.name}{#if user.id === app.viewer.id}<span class="chip outline">You</span>{/if}</strong>
              <small>{grants(user)} · joined {date(user.createdAt)}{#if sessionCount.get(user.id)} · {sessionCount.get(user.id)} active{/if}</small>
            </span>
            <span class="chip" class:accent={user.role === "ADMIN"}>{titleCase(user.role)}</span>
            <Menu width={240}>
              {#snippet trigger(props)}<button {...props} class="icon-btn sm" aria-label="Manage {user.name}"><DotsThree size={18} weight="bold" /></button>{/snippet}
              <MenuItem icon={Books} onSelect={() => openAccess(user)}>Role & library access…</MenuItem>
              <MenuItem icon={ShieldCheck} onSelect={() => (restricting = { open: true, user })}>Restrictions…</MenuItem>
              <MenuItem icon={SignOut} disabled={!sessionCount.get(user.id)} onSelect={() => revokeAll(user)}>Sign out everywhere</MenuItem>
              <div class="nf-menu-sep"></div>
              <MenuItem icon={Trash} danger disabled={user.id === app.viewer.id} onSelect={() => remove(user)}>Delete user</MenuItem>
            </Menu>
          </li>
        {/each}
      </ul>
    </section>

    <section class="panel" style:--i={1}>
      <header class="panel-head">
        <span class="panel-icon"><Devices size={19} /></span>
        <div><h2>Active sessions</h2><p>Every signed-in device on this server</p></div>
        <span class="spacer"></span>
        <button class="icon-btn sm" aria-label="Refresh sessions" onclick={() => refetch(admin)}><ArrowsClockwise size={16} /></button>
      </header>
      {#if sessions.length}
        <ul class="list">
          {#each sessions as session (session.id)}
            <li>
              <span class="dot" class:on={Date.now() - Date.parse(session.lastSeenAt) < 600_000}></span>
              <span class="what">
                <strong>{names.get(session.userId) ?? "Deleted user"}</strong>
                <small>{session.device || "Unknown device"}{session.client ? ` · ${session.client}` : ""} · {ago(session.lastSeenAt)}</small>
              </span>
              <code class="ip">{session.remoteAddr || "local"}</code>
              <button class="icon-btn sm" aria-label="Revoke {names.get(session.userId) ?? 'user'}’s session on {session.device || 'unknown device'}" onclick={() => revokeSession(session)}><X size={16} /></button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty-note">Nobody is signed in right now.</p>
      {/if}
    </section>
  </div>
{/if}

<Dialog bind:open={creating} title="New user" description="They can change the password after signing in." width={460}>
  <form id="user-form" class="stack" onsubmit={create}>
    <label class="field">Username<input bind:value={draft.username} required autocomplete="off" autocapitalize="off" spellcheck="false" /></label>
    <label class="field">Initial password<input type="password" bind:value={draft.password} required minlength="8" autocomplete="new-password" /><small>At least 8 characters.</small></label>
    <label class="field">Role<Select bind:value={draft.role} items={ROLES} label="Role" /></label>
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (creating = false)}>Cancel</button>
    <button class="btn primary" form="user-form" disabled={saving || !draft.username.trim() || draft.password.length < 8}>{#if saving}<span class="spinner sm"></span>{/if}Create user</button>
  {/snippet}
</Dialog>

<Dialog bind:open={access.open} title="Access for {access.user?.name ?? ''}" description="Choose a role and which libraries they can browse." variant="sheet" width={480}>
  <form id="access-form" class="stack" onsubmit={saveAccess}>
    <label class="field">Role<Select bind:value={access.role} items={ROLES} label="Role" /></label>
    <fieldset class="libs">
      <legend>Libraries</legend>
      {#if access.role === "ADMIN"}<p class="hint">Administrators can see every library; these grants apply if the role changes later.</p>{/if}
      {#each libraries as library (library.id)}
        <label class="check"><input type="checkbox" checked={access.libraries.includes(library.id)} onchange={() => toggleLibrary(library.id)} />{library.name}<small class="faint">{titleCase(library.kind)}</small></label>
      {:else}
        <p class="hint">No libraries exist yet.</p>
      {/each}
      {#if libraries.length > 3}
        <div class="row">
          <button type="button" class="btn ghost sm" onclick={() => (access.libraries = libraries.map((library) => library.id))}>Select all</button>
          <button type="button" class="btn ghost sm" onclick={() => (access.libraries = [])}>None</button>
        </div>
      {/if}
    </fieldset>
  </form>
  {#snippet footer()}
    <button class="btn ghost" onclick={() => (access.open = false)}>Cancel</button>
    <button class="btn primary" form="access-form" disabled={saving}>{#if saving}<span class="spinner sm"></span>{/if}Save access</button>
  {/snippet}
</Dialog>

<RestrictionsSheet bind:open={restricting.open} user={restricting.user} />

<style>
  .layout { display: grid; grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr); gap: 18px; align-items: start; }
  .row-skel { display: flex; align-items: center; gap: 14px; padding: 12px 0; }
  .avatar-skel { width: 34px; height: 34px; border-radius: 50%; flex: none; }
  .row-skel .bar { flex: 1; height: 18px; border-radius: 6px; }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what strong { display: flex; align-items: center; gap: 8px; }
  .what .chip { height: 20px; font-size: 0.68rem; }
  .what small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ip { color: var(--text-3); font-size: 0.78rem; }
  .libs { display: grid; gap: 12px; margin: 6px 0 0; padding: 0; border: 0; }
  .libs legend { padding: 0; margin-bottom: 10px; font-size: 0.72rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-3); }
  .libs .check { display: flex; }
  .libs .check small { margin-left: auto; }
  .hint { color: var(--text-3); font-size: 0.85rem; }
  @media (max-width: 980px) { .layout { grid-template-columns: 1fr; } }
  @media (max-width: 480px) { .ip { display: none; } }
</style>
