<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import ArrowsClockwise from "phosphor-svelte/lib/ArrowsClockwise";
  import CaretLeft from "phosphor-svelte/lib/CaretLeft";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import Devices from "phosphor-svelte/lib/Devices";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import SignOut from "phosphor-svelte/lib/SignOut";
  import X from "phosphor-svelte/lib/X";
  import { date } from "$lib/format.js";
  import { deviceIcon } from "$components/account/deviceIcon.js";
  import { ago } from "$components/account/relative.js";
  import Dialog from "$components/ui/Dialog.svelte";
  import Segmented from "$components/ui/Segmented.svelte";

  /**
   * `sessions` newest first; `names` maps user ids to names. `onrevoke` takes
   * a session; `onrevokeall` a `{ id, name }` user.
   */
  let { sessions, names, onrefresh, onrevoke, onrevokeall } = $props();

  const PREVIEW = 5;
  const PAGE_SIZE = 20;
  const ACTIVE_MS = 600_000;
  const SHOWS = [{ value: "all", label: "All" }, { value: "active", label: "Active now" }, { value: "idle", label: "Idle" }];

  const isActive = (session) => Date.now() - Date.parse(session.lastSeenAt) < ACTIVE_MS;
  const owner = (session) => names.get(session.userId) ?? "Deleted user";
  const device = (session) => session.device || "Unknown device";

  let activeCount = $derived(sessions.filter(isActive).length);
  let peopleCount = $derived(new Set(sessions.map((session) => session.userId)).size);

  let open = $state(false);
  let search = $state("");
  let show = $state("all");
  let pageIndex = $state(0);

  let matching = $derived.by(() => {
    const words = search.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return sessions.filter((session) => {
      if (show !== "all" && isActive(session) !== (show === "active")) return false;
      const text = `${owner(session)} ${device(session)} ${session.client ?? ""} ${session.remoteAddr ?? ""}`.toLowerCase();
      return words.every((word) => text.includes(word));
    });
  });
  let pageCount = $derived(Math.max(1, Math.ceil(matching.length / PAGE_SIZE)));
  // Signing devices out can shrink the list from under the last page.
  let shownPage = $derived(Math.min(pageIndex, pageCount - 1));
  // By person, most recently active first. Sorting is stable, so each person's stay newest first.
  let grouped = $derived.by(() => {
    const latest = new Map();
    const totals = new Map();
    matching.forEach((session, index) => {
      if (!latest.has(session.userId)) latest.set(session.userId, index);
      totals.set(session.userId, (totals.get(session.userId) ?? 0) + 1);
    });
    const sorted = [...matching].sort((a, b) => latest.get(a.userId) - latest.get(b.userId));
    const groups = [];
    for (const session of sorted.slice(shownPage * PAGE_SIZE, (shownPage + 1) * PAGE_SIZE)) {
      const last = groups.at(-1);
      if (last?.userId === session.userId) last.sessions.push(session);
      else groups.push({ userId: session.userId, name: owner(session), total: totals.get(session.userId), sessions: [session] });
    }
    return groups;
  });
</script>

{#snippet row(session, withOwner)}
  {@const Glyph = deviceIcon(session)}
  <li>
    <span class="glyph" class:on={isActive(session)}><Glyph size={18} /></span>
    <span class="what">
      <strong>{withOwner ? owner(session) : device(session)}</strong>
      <small>{#if withOwner}{device(session)}{" · "}{:else if session.client}{session.client}{" · "}{/if}<time datetime={session.lastSeenAt} title={date(session.lastSeenAt, { dateStyle: "medium", timeStyle: "short" })}>{isActive(session) ? "active now" : ago(session.lastSeenAt)}</time></small>
    </span>
    <code class="ip">{session.remoteAddr || "local"}</code>
    <button class="icon-btn sm" aria-label="Sign out {owner(session)}’s session on {device(session)}" title="Sign out this device" onclick={() => onrevoke(session)}><X size={16} /></button>
  </li>
{/snippet}

<section class="panel">
  <header class="panel-head">
    <span class="panel-icon"><Devices size={19} /></span>
    <div>
      <h2>Active sessions</h2>
      <p>{#if sessions.length}{sessions.length} signed in · {activeCount} active now · {peopleCount} {peopleCount === 1 ? "person" : "people"}{:else}Every signed-in device on this server{/if}</p>
    </div>
    <span class="spacer"></span>
    <button class="icon-btn sm" aria-label="Refresh sessions" onclick={onrefresh}><ArrowsClockwise size={16} /></button>
  </header>
  {#if sessions.length}
    <ul class="list">
      {#each sessions.slice(0, PREVIEW) as session (session.id)}{@render row(session, true)}{/each}
    </ul>
    {#if sessions.length > PREVIEW}
      <button class="btn ghost sm more" onclick={() => (open = true)}>View all {sessions.length} sessions<CaretRight size={14} /></button>
    {/if}
  {:else}
    <p class="empty-note">Nobody is signed in right now.</p>
  {/if}
</section>

<Dialog bind:open variant="sheet" width={620} title="Active sessions" description="{sessions.length} signed in · {activeCount} active now">
  <div class="tools">
    <label class="search">
      <MagnifyingGlass size={16} />
      <input type="search" placeholder="Person, device or address" aria-label="Search sessions" value={search} oninput={(event) => { search = event.currentTarget.value; pageIndex = 0; }} />
    </label>
    <Segmented value={show} items={SHOWS} label="Which sessions" size="sm" onchange={(value) => { show = value; pageIndex = 0; }} />
  </div>

  {#each grouped as group (group.userId)}
    <section class="person">
      <header>
        <h3>{group.name}<small>{group.total} {group.total === 1 ? "session" : "sessions"}</small></h3>
        {#if names.has(group.userId)}<button class="btn ghost sm" onclick={() => onrevokeall({ id: group.userId, name: group.name })}><SignOut size={15} />Sign out everywhere</button>{/if}
      </header>
      <ul class="list">
        {#each group.sessions as session (session.id)}{@render row(session, false)}{/each}
      </ul>
    </section>
  {:else}
    <p class="empty-note">No sessions match.</p>
  {/each}

  {#snippet footer()}
    {#if pageCount > 1}
      <div class="pager">
        <button class="icon-btn sm" aria-label="Previous page" disabled={shownPage === 0} onclick={() => (pageIndex = shownPage - 1)}><CaretLeft size={16} /></button>
        <span class="tabular">{shownPage * PAGE_SIZE + 1}–{Math.min((shownPage + 1) * PAGE_SIZE, matching.length)} of {matching.length}</span>
        <button class="icon-btn sm" aria-label="Next page" disabled={shownPage >= pageCount - 1} onclick={() => (pageIndex = shownPage + 1)}><CaretRight size={16} /></button>
      </div>
    {/if}
    <button class="btn ghost" onclick={() => (open = false)}>Close</button>
  {/snippet}
</Dialog>

<style>
  .list > li { gap: 12px; padding-block: 10px; }
  .glyph { position: relative; display: grid; place-items: center; flex: none; width: 36px; height: 36px; border-radius: 10px; background: var(--surface-2); color: var(--text-2); }
  .glyph.on { color: var(--text); }
  .glyph.on::after { content: ""; position: absolute; right: -2px; bottom: -2px; width: 11px; height: 11px; border: 2px solid var(--surface); border-radius: 50%; background: var(--success); }
  .what { flex: 1; display: grid; gap: 1px; min-width: 0; }
  .what strong, .what small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ip { flex: none; color: var(--text-3); font-size: 0.78rem; }
  .more { margin-top: 10px; width: 100%; justify-content: center; }
  .tools { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-bottom: 16px; }
  .search { flex: 1 1 200px; display: flex; align-items: center; gap: 8px; min-width: 0; height: 36px; padding: 0 12px; border: 1px solid var(--line); border-radius: var(--r); color: var(--text-3); }
  .search:focus-within { border-color: var(--line-strong); }
  .search input { flex: 1; min-width: 0; border: 0; outline: 0; background: none; color: var(--text); font: inherit; font-size: 0.88rem; }
  .person + .person { margin-top: 18px; }
  .person header { display: flex; align-items: center; justify-content: space-between; gap: 10px; min-height: 32px; }
  .person h3 { display: flex; align-items: baseline; gap: 8px; min-width: 0; font-size: 0.94rem; font-weight: 650; }
  .person h3 small { flex: none; font-size: 0.78rem; font-weight: 450; color: var(--text-3); }
  .pager { display: flex; align-items: center; gap: 6px; margin-right: auto; font-size: 0.84rem; color: var(--text-2); }
  .pager button:disabled { opacity: 0.35; pointer-events: none; }
  @media (max-width: 480px) { .ip { display: none; } .person header .btn { padding-inline: 8px; } }
</style>
