<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import Link from "phosphor-svelte/lib/Link";
  import Play from "phosphor-svelte/lib/Play";
  import SignOut from "phosphor-svelte/lib/SignOut";
  import X from "phosphor-svelte/lib/X";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { graphql, JoinWatchRoomStore, ControlWatchRoomStore } from "$houdini";
  import { date, timecode, titleCase } from "$lib/format.js";
  import { failure, refetch } from "$lib/graphql.js";
  import { player, playRoom } from "$lib/playback.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt, confirmAction, poll, toast } from "$lib/ui.svelte.js";
  import Artwork from "$components/Artwork.svelte";
  import State from "$components/State.svelte";

  let { data: route } = $props();
  let rooms = $derived(route.rooms);
  // There's no subscription for the room list itself.
  $effect(() => { const store = rooms; return poll(() => refetch(store), 4000); });

  const joinRoom = new JoinWatchRoomStore();
  const controlRoom = new ControlWatchRoomStore();
  const leaveRoom = graphql(`
    mutation LeaveWatchRoom($id: ID!) { leaveWatchRoom(roomId: $id) }
  `);

  let focusId = $derived(page.url.searchParams.get("room"));
  let list = $derived($rooms.data?.watchRooms ?? []);
  // With one room open there's nothing to choose between, so the sidebar link lands in it.
  let focused = $derived(focusId ? list.find((room) => room.id === focusId) : list.length === 1 ? list[0] : undefined);
  let joinText = $state("");

  // Rooms are listed to their members, so an invite link joins first.
  let invited = "";
  $effect(() => {
    if (!focusId || !$rooms.data || focused || invited === focusId) return;
    invited = focusId;
    attempt(() => joinRoom.mutate({ id: focusId })).then((joined) => { if (joined) refetch(rooms); });
  });

  const shareUrl = (room) => `${location.origin}/watch-rooms?room=${room.id}`;
  const isHost = (room) => room.hostUserId === app.viewer.id;
  const open = (room) => goto(`?room=${room.id}`);

  async function join(room) {
    if (!isHost(room)) {
      const joined = await attempt(() => joinRoom.mutate({ id: room.id }));
      if (!joined) return;
      room = joined.data.joinWatchRoom;
    }
    await playRoom(room, isHost(room));
  }

  async function joinById(event) {
    event.preventDefault();
    const id = joinText.trim().match(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i)?.[0];
    if (!id) { toast("That doesn’t look like a room link or ID", "error"); return; }
    const result = await attempt(() => joinRoom.mutate({ id }));
    if (!result) return;
    joinText = "";
    goto(`?room=${id}`, { replaceState: true, noScroll: true, keepFocus: true });
  }

  async function end(room) {
    if (!(await confirmAction({ title: "End this watch room?", message: "Everyone in it will be disconnected.", confirmLabel: "End room", danger: true }))) return;
    if (await attempt(() => controlRoom.mutate({ id: room.id, action: "STOP", position: room.positionMs }), "Room ended")) refetch(rooms);
  }

  async function leave(room) {
    if (await attempt(() => leaveRoom.mutate({ id: room.id }), "You left the room")) { goto("?", { replaceState: true }); refetch(rooms); }
  }

  function copy(room) {
    navigator.clipboard?.writeText(shareUrl(room)).then(() => toast("Invite link copied", "success"), () => toast("Couldn’t access the clipboard", "error"));
  }
</script>

<svelte:head><title>Watch together · NoFlex</title></svelte:head>

<div class="page">
  <header class="page-head">
    <div><p class="eyebrow">Synchronised playback</p><h1>Watch together</h1><p>Same frame, different couches. Start a room from any title’s ••• menu.</p></div>
    <form class="join" onsubmit={joinById}>
      <label class="sr-only" for="join-room">Room link or ID</label>
      <input id="join-room" class="input" bind:value={joinText} placeholder="Paste an invite link or room ID" />
      <button class="btn" disabled={!joinText.trim()}>Join</button>
    </form>
  </header>

  {#if focused}
    <section class="focus fade-up">
      <div class="art">{#if focused.work}<Artwork work={focused.work} kind="backdrop" width={900} eager rounded={false} />{/if}</div>
      <div class="focus-body">
        <p class="eyebrow"><span class="dot {focused.state === 'PLAYING' ? 'live' : 'on'}"></span>{isHost(focused) ? "You’re hosting" : "You’re invited"} · {titleCase(focused.state)}</p>
        <h2>{focused.work?.title ?? "Watch room"}</h2>
        <p class="muted">{focused.state === "WAITING" ? "Waiting for the host to press play." : `At ${timecode(focused.positionMs)}`}</p>
        <div class="row">
          <button class="btn primary lg" onclick={() => join(focused)} disabled={player.preparing === focused.id}>{#if player.preparing === focused.id}<span class="spinner sm"></span>{:else}<Play size={18} weight="fill" />{/if}{isHost(focused) ? "Start watching" : "Join stream"}</button>
          <button class="btn glass lg" onclick={() => copy(focused)}><Link size={17} />Copy invite link</button>
          {#if isHost(focused)}<button class="btn ghost lg" onclick={() => end(focused)}><X size={17} />End room</button>
          {:else}<button class="btn ghost lg" onclick={() => leave(focused)}><SignOut size={17} />Leave room</button>{/if}
        </div>
      </div>
    </section>
  {/if}

  <section class="section">
    <div class="section-head"><h2>Open rooms</h2>{#if list.length}<span class="faint">{list.length}</span>{/if}</div>
    {#if $rooms.fetching && !$rooms.data}
      <div class="grid">{#each Array(3) as _}<div class="skeleton" style:height="240px"></div>{/each}</div>
    {:else if failure($rooms) && !$rooms.data}
      <State kind="error" title="Rooms are unavailable" message={failure($rooms)} action={() => refetch(rooms)} />
    {:else if !list.length}
      <State icon={Broadcast} title="No rooms open" message="Open a title, choose ••• → Start a watch room, and share the invite link." />
    {:else}
      <div class="grid stagger">
        {#each list as room, index (room.id)}
          <article class="room" class:current={room.id === focused?.id} style:--i={index}>
            <button class="thumb" onclick={() => open(room)} aria-label="Open room for {room.work?.title ?? 'this title'}" aria-current={room.id === focused?.id ? "true" : undefined}>
              {#if room.work}<Artwork work={room.work} kind="thumb" width={360} />{:else}<div class="blank"><Broadcast size={30} /></div>{/if}
              <span class="state"><span class="dot {room.state === 'PLAYING' ? 'live' : room.state === 'ENDED' ? '' : 'on'}"></span>{titleCase(room.state)}</span>
              <span class="go">Open room</span>
            </button>
            <div class="meta">
              <div><strong>{room.work?.title ?? "Untitled room"}</strong><small>{isHost(room) ? "Hosted by you" : "Hosted by a friend"} · started {date(room.createdAt, { timeStyle: "short" })}</small></div>
              <div class="row">
                <button class="icon-btn sm" aria-label={isHost(room) ? "Start watching" : "Join stream"} onclick={() => join(room)} disabled={player.preparing === room.id}>{#if player.preparing === room.id}<span class="spinner sm"></span>{:else}<Play size={16} weight="fill" />{/if}</button>
                <button class="icon-btn sm" aria-label="Copy invite link" onclick={() => copy(room)}><Link size={16} /></button>
                {#if isHost(room)}<button class="icon-btn sm" aria-label="End room" onclick={() => end(room)}><X size={16} /></button>
                {:else}<button class="icon-btn sm" aria-label="Leave room" onclick={() => leave(room)}><SignOut size={16} /></button>{/if}
              </div>
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .join { display: flex; gap: 8px; width: min(420px, 100%); }
  .join .input { border-radius: 999px; padding-left: 16px; }
  .focus { position: relative; display: grid; grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr); border-radius: var(--r-xl); overflow: hidden; background: var(--surface); border: 1px solid var(--line-strong); box-shadow: var(--shadow-2); }
  .focus .art :global(.art) { height: 100%; }
  .focus-body { display: grid; align-content: center; gap: 12px; padding: clamp(22px, 4vw, 44px); }
  .focus-body .eyebrow { display: flex; align-items: center; gap: 8px; }
  .focus-body h2 { font-size: clamp(1.5rem, 3vw, 2.2rem); }
  .focus-body .row { flex-wrap: wrap; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr)); gap: 20px; }
  .room { border-radius: var(--r-l); background: var(--surface); border: 1px solid var(--line); overflow: hidden; transition: border-color var(--t), transform 400ms var(--ease-out); }
  .room:hover { border-color: var(--line-strong); transform: translateY(-2px); }
  .room.current { border-color: rgb(212 255 90 / 0.4); }
  .thumb { position: relative; display: block; width: 100%; }
  .thumb :global(.art) { border-radius: 0; }
  .blank { display: grid; place-items: center; aspect-ratio: 16 / 9; background: var(--surface-2); color: var(--text-3); }
  .state { position: absolute; left: 12px; top: 12px; display: inline-flex; align-items: center; gap: 8px; height: 26px; padding: 0 10px; border-radius: 99px; background: rgb(0 0 0 / 0.6); backdrop-filter: blur(10px); font-size: 0.76rem; font-weight: 650; }
  .go { position: absolute; inset: 0; display: grid; place-items: center; background: rgb(0 0 0 / 0.35); font-weight: 650; opacity: 0; transition: opacity var(--t); }
  .thumb:hover .go, .thumb:focus-visible .go { opacity: 1; }
  .meta { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 14px 16px; }
  .meta > div:first-child { display: grid; min-width: 0; }
  .meta strong { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .meta small { color: var(--text-3); font-size: 0.8rem; }
  @media (max-width: 760px) { .focus { grid-template-columns: 1fr; } }
</style>
