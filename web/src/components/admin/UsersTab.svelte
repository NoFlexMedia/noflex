<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Books from "phosphor-svelte/lib/Books";
  import DotsThree from "phosphor-svelte/lib/DotsThree";
  import Plus from "phosphor-svelte/lib/Plus";
  import ShieldCheck from "phosphor-svelte/lib/ShieldCheck";
  import SignOut from "phosphor-svelte/lib/SignOut";
  import Trash from "phosphor-svelte/lib/Trash";
  import Users from "phosphor-svelte/lib/Users";
  import { untrack } from "svelte";
  import { graphql } from "$houdini";
  import { date, titleCase } from "$lib/format.js";
  import { refetch } from "$lib/graphql.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt, confirmAction } from "$lib/ui.svelte.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Select from "$components/ui/Select.svelte";
  import RestrictionsSheet from "./RestrictionsSheet.svelte";
  import SessionsPanel from "./SessionsPanel.svelte";

  /** `admin` is the page's Admin store. */
  let { admin, creating = $bindable(false) } = $props();

  const createInvite = graphql(`
    mutation CreateInvite($libraries: [ID!]!, $days: Int!, $uses: Int!) {
      createInvite(libraryIds: $libraries, expiresInDays: $days, maxUses: $uses) { token }
    }
  `);
  let inviting = $state(false);
  let invite = $state({ libraries: [], days: 7, uses: 1, link: "" });
  function openInvite() {
    invite = { libraries: [], days: 7, uses: 1, link: "" };
    inviting = true;
  }
  async function makeInvite(event) {
    event.preventDefault();
    const result = await attempt(() => createInvite.mutate({ libraries: invite.libraries, days: Number(invite.days), uses: Number(invite.uses) }));
    if (result) invite.link = `${location.origin}/invite#${result.data.createInvite.token}`;
  }
  async function copyInvite() {
    await attempt(() => navigator.clipboard.writeText(invite.link), "Invite link copied");
  }

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
    mutation RevokeUserSessions($user: ID!) { revokeUserSessions(userId: $user) { id @Session_delete } }
  `);
  const deleteUser = graphql(`
    mutation DeleteUser($id: ID!) { deleteUser(id: $id) { user { id @User_delete } sessions { id @Session_delete } } }
  `);
  const revokeSessionMutation = graphql(`
    mutation RevokeUserSession($id: ID!) { revokeSession(id: $id) { id @Session_delete } }
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
    await attempt(() => revokeUserSessions.mutate({ user: user.id }), `${user.name} signed out everywhere`);
  }

  async function remove(user) {
    if (!(await confirmAction({ title: `Delete ${user.name}?`, message: "Their account, sessions, playlists and watch history are removed. This can’t be undone.", confirmLabel: "Delete user", danger: true }))) return;
    await attempt(() => deleteUser.mutate({ id: user.id }), `${user.name} deleted`);
  }

  async function revokeSession(session) {
    const who = names.get(session.userId) ?? "this user";
    if (!(await confirmAction({ title: `Sign out ${session.device || "this device"}?`, message: `${who} will need to sign in again on it.`, confirmLabel: "Sign out device", danger: true }))) return;
    await attempt(() => revokeSessionMutation.mutate({ id: session.id }), "Session revoked");
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
        <button class="btn sm" onclick={openInvite}><Users size={15} />Invite</button>
        <button class="btn sm" onclick={() => (creating = true)}><Plus size={15} />Add</button>
      </header>
      <ul class="list">
        {#each users as user (user.id)}
          <li>
            <span class="avatar" aria-hidden="true">{user.name.slice(0, 2).toUpperCase()}</span>
            <span class="what">
              <strong>{user.name}{#if user.id === app.viewer.id}<span class="chip outline">You</span>{/if}</strong>
              <small>{grants(user)} · joined {date(user.createdAt)}{#if sessionCount.get(user.id)}{" · "}{sessionCount.get(user.id)} signed in{/if}</small>
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

    <div style:--i={1}><SessionsPanel {sessions} {names} onrefresh={() => refetch(admin)} onrevoke={revokeSession} onrevokeall={revokeAll} /></div>
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

<Dialog bind:open={inviting} title="Invite someone" description="They choose their own username and password, and the account is never an administrator.">
  {#if invite.link}
    <div class="stack">
      <label class="field">Invite link<input readonly value={invite.link} onfocus={(event) => event.currentTarget.select()} /><small>Shown once. Anyone with this link can make an account until it expires or is used up.</small></label>
    </div>
  {:else}
    <form id="invite-form" class="stack" onsubmit={makeInvite}>
      <fieldset class="libs">
        <legend>Libraries they can see</legend>
        {#each libraries as library (library.id)}
          <label class="check"><input type="checkbox" bind:group={invite.libraries} value={library.id} />{library.name}</label>
        {:else}
          <p class="hint">No libraries exist yet.</p>
        {/each}
      </fieldset>
      <label class="field">Expires after (days)<input type="number" min="1" max="365" bind:value={invite.days} required /></label>
      <label class="field">Number of people<input type="number" min="1" max="100" bind:value={invite.uses} required /></label>
    </form>
  {/if}
  {#snippet footer()}
    {#if invite.link}
      <button class="btn ghost" onclick={() => (inviting = false)}>Done</button>
      <button class="btn primary" onclick={copyInvite}>Copy link</button>
    {:else}
      <button class="btn ghost" onclick={() => (inviting = false)}>Cancel</button>
      <button class="btn primary" form="invite-form">Create link</button>
    {/if}
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
  .libs { display: grid; gap: 12px; margin: 6px 0 0; padding: 0; border: 0; }
  .libs legend { padding: 0; margin-bottom: 10px; font-size: 0.72rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-3); }
  .libs .check { display: flex; }
  .libs .check small { margin-left: auto; }
  .hint { color: var(--text-3); font-size: 0.85rem; }
  @media (max-width: 980px) { .layout { grid-template-columns: 1fr; } }
</style>
