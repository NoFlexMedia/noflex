<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Avatar from "$components/Avatar.svelte";
  import ArrowLeft from "phosphor-svelte/lib/ArrowLeft";
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import Link from "phosphor-svelte/lib/Link";
  import Play from "phosphor-svelte/lib/Play";
  import SignOut from "phosphor-svelte/lib/SignOut";
  import UsersThree from "phosphor-svelte/lib/UsersThree";
  import X from "phosphor-svelte/lib/X";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { graphql, JoinWatchRoomStore, ControlWatchRoomStore } from "$houdini";
  import { date, timecode, titleCase } from "$lib/format.js";
  import { failure, refetch } from "$lib/graphql.js";
  import { player, playRoom } from "$lib/playback.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt, confirmAction, poll, toast } from "$lib/ui.svelte.js";
  import { ago } from "$components/account/relative.js";
  import Artwork from "$components/Artwork.svelte";
  import State from "$components/State.svelte";

  let { data: route } = $props();
  let rooms = $derived(route.rooms);

  const joinRoom = new JoinWatchRoomStore();
  const controlRoom = new ControlWatchRoomStore();
  const leaveRoom = graphql(`
    mutation LeaveWatchRoom($id: ID!) { leaveWatchRoom(roomId: $id) }
  `);

  let focusId = $derived(page.url.searchParams.get("room"));
  let list = $derived(($rooms.data?.watchRooms ?? []).filter((room) => room.state !== "ENDED"));
  // With one room open there's nothing to choose between, so the sidebar link lands in it.
  let focused = $derived(focusId ? list.find((room) => room.id === focusId) : list.length === 1 ? list[0] : undefined);
  // The room in the waiting area isn't also a card to open.
  let others = $derived(list.filter((room) => room.id !== focused?.id));
  let joinText = $state("");

  // There's no subscription for the room list or who's in a room. A waiting area
  // should notice arrivals, and the host pressing play, about when they happen.
  let polling = $derived(focused ? 2000 : 4000);
  $effect(() => { const store = rooms; return poll(() => refetch(store), polling); });

  // Rooms are listed to their members, so an invite link joins first.
  let invited = "";
  $effect(() => {
    if (!focusId || !$rooms.data || focused || invited === focusId) return;
    invited = focusId;
    attempt(() => joinRoom.mutate({ id: focusId })).then((joined) => { if (joined) refetch(rooms); });
  });

  const shareUrl = (room) => `${location.origin}/watch-rooms?room=${room.id}`;
  const isHost = (room) => room.hostUserId === app.viewer.id;
  const hostName = (room) => room.members?.find((member) => member.userId === room.hostUserId)?.name ?? "the host";
  const memberName = (member) => member.name ?? "Someone";
  const open = (room) => goto(`?room=${room.id}`);

  async function join(room) {
    if (!isHost(room)) {
      const joined = await attempt(() => joinRoom.mutate({ id: room.id }));
      if (!joined) return;
      room = joined.data.joinWatchRoom;
    }
    await playRoom(room, isHost(room));
  }

  // Everyone waiting goes in when the host starts. Only a room seen waiting: someone
  // opening a room that's already playing, or back after closing the player, chose
  // not to be pulled in.
  let waitedIn = $state(null);
  $effect(() => {
    if (!focused || isHost(focused)) return;
    if (focused.state === "WAITING") { waitedIn = focused.id; return; }
    if (waitedIn === focused.id && (focused.state === "PLAYING" || focused.state === "PAUSED") && !player.current && !player.preparing) {
      waitedIn = null;
      playRoom(focused, false);
    }
  });

  async function joinById(event) {
    event.preventDefault();
    const id = joinText.trim().match(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i)?.[0];
    if (!id) { toast("That doesn’t look like a room link or ID", "error"); return; }
    const result = await attempt(() => joinRoom.mutate({ id }));
    if (!result) return;
    joinText = "";
    refetch(rooms);
    goto(`?room=${id}`, { replaceState: true, noScroll: true, keepFocus: true });
  }

  async function end(room) {
    if (!(await confirmAction({ title: "End this watch room?", message: "Everyone in it will be disconnected.", confirmLabel: "End room", danger: true }))) return;
    if (await attempt(() => controlRoom.mutate({ id: room.id, action: "STOP", position: room.positionMs }), "Room ended")) { goto("?", { replaceState: true }); refetch(rooms); }
  }

  async function leave(room) {
    if (await attempt(() => leaveRoom.mutate({ id: room.id }), "You left the room")) { goto("?", { replaceState: true }); refetch(rooms); }
  }

  function copy(room) {
    navigator.clipboard?.writeText(shareUrl(room)).then(() => toast("Invite link copied", "success"), () => toast("Couldn’t access the clipboard", "error"));
  }

  function status(room) {
    if (room.state === "WAITING") return isHost(room) ? "Everyone goes in together when you start." : `Waiting for ${hostName(room)} to start.`;
    return `${titleCase(room.state)} at ${timecode(room.positionMs)}`;
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
    {#if focusId && list.length > 1}<a class="back" href="?"><ArrowLeft size={15} />All rooms</a>{/if}
    <section class="lobby fade-up" aria-label="Waiting area">
      <div class="stage">
        <div class="art">{#if focused.work}<Artwork work={focused.work} kind="backdrop" width={900} eager rounded={false} />{:else}<div class="blank"><Broadcast size={36} /></div>{/if}</div>
        <div class="stage-body">
          <p class="eyebrow"><span class="dot {focused.state === 'PLAYING' ? 'live' : 'on'}"></span>{isHost(focused) ? "You’re hosting" : `Hosted by ${hostName(focused)}`} · {titleCase(focused.state)}</p>
          <h2>{focused.work?.title ?? "Watch room"}</h2>
          <p class="muted" aria-live="polite">{status(focused)}</p>
          <div class="row">
            {#if isHost(focused) || focused.state !== "WAITING"}
              <button class="btn primary lg" onclick={() => join(focused)} disabled={player.preparing === focused.id}>{#if player.preparing === focused.id}<span class="spinner sm"></span>{:else}<Play size={18} weight="fill" />{/if}{isHost(focused) ? "Start watching" : "Join stream"}</button>
            {:else}
              <span class="waiting"><span class="spinner sm"></span>Waiting for {hostName(focused)}</span>
            {/if}
            {#if isHost(focused)}<button class="btn ghost lg" onclick={() => end(focused)}><X size={17} />End room</button>
            {:else}<button class="btn ghost lg" onclick={() => leave(focused)}><SignOut size={17} />Leave room</button>{/if}
          </div>
        </div>
      </div>

      <aside class="people">
        <header><UsersThree size={18} /><h3>In this room</h3><span class="faint tabular">{focused.members?.length ?? 0}</span></header>
        <ul>
          {#each focused.members ?? [] as member (member.userId)}
            <li>
              <Avatar user={{ name: memberName(member), avatarUrl: member.avatarUrl }} />
              <span class="who"><strong>{memberName(member)}</strong><small>{member.userId === focused.hostUserId ? `Opened ${ago(member.joinedAt)}` : `Joined ${ago(member.joinedAt)}`}</small></span>
              {#if member.userId === app.viewer.id}<span class="chip">You</span>{/if}
              {#if member.userId === focused.hostUserId}<span class="chip accent">Host</span>{/if}
            </li>
          {/each}
        </ul>
        {#if (focused.members?.length ?? 0) < 2}<p class="alone">{isHost(focused) ? "Nobody else yet. Send them the link below." : "Just you so far."}</p>{/if}
        <button class="btn glass invite" onclick={() => copy(focused)}><Link size={16} />Copy invite link</button>
      </aside>
    </section>
  {/if}

  {#if !focused || others.length}
    <section class="section">
      <div class="section-head"><h2>{focused ? "Other rooms" : "Open rooms"}</h2>{#if others.length}<span class="faint">{others.length}</span>{/if}</div>
      {#if $rooms.fetching && !$rooms.data}
        <div class="grid">{#each Array(3) as _}<div class="skeleton" style:height="240px"></div>{/each}</div>
      {:else if failure($rooms) && !$rooms.data}
        <State kind="error" title="Rooms are unavailable" message={failure($rooms)} action={() => refetch(rooms)} />
      {:else if !others.length}
        <State icon={Broadcast} title="No rooms open" message="Open a title, choose ••• → Start a watch room, and share the invite link." />
      {:else}
        <div class="grid stagger">
          {#each others as room, index (room.id)}
            <article class="room" style:--i={index}>
              <button class="thumb" onclick={() => open(room)} aria-label="Open room for {room.work?.title ?? 'this title'}">
                {#if room.work}<Artwork work={room.work} kind="thumb" width={360} />{:else}<div class="blank"><Broadcast size={30} /></div>{/if}
                <span class="state"><span class="dot {room.state === 'PLAYING' ? 'live' : 'on'}"></span>{titleCase(room.state)}</span>
                <span class="go">Open room</span>
              </button>
              <div class="meta">
                <div><strong>{room.work?.title ?? "Untitled room"}</strong><small>{isHost(room) ? "Hosted by you" : `Hosted by ${hostName(room)}`} · {room.members?.length ?? 0} in the room · started {date(room.createdAt, { timeStyle: "short" })}</small></div>
                <div class="row">
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
  {/if}
</div>

<style>
  .join { display: flex; gap: 8px; width: min(420px, 100%); }
  .join .input { border-radius: 999px; padding-left: 16px; }
  .back { display: inline-flex; align-items: center; gap: 6px; margin-bottom: 14px; color: var(--text-2); font-size: 0.88rem; font-weight: 550; }
  .back:hover { color: var(--text); }
  .lobby { display: grid; grid-template-columns: minmax(0, 1fr) minmax(280px, 340px); gap: 16px; margin-bottom: 32px; }
  .stage { display: grid; border-radius: var(--r-xl); overflow: hidden; background: var(--surface); border: 1px solid var(--line-strong); }
  .stage .art { aspect-ratio: 21 / 9; overflow: hidden; }
  .stage .art :global(.art) { height: 100%; aspect-ratio: auto; }
  .stage-body { display: grid; align-content: start; gap: 12px; padding: clamp(20px, 3vw, 32px); }
  .stage-body .eyebrow { display: flex; align-items: center; gap: 8px; }
  .stage-body h2 { font-size: clamp(1.5rem, 3vw, 2.2rem); }
  .stage-body .row { flex-wrap: wrap; }
  .waiting { display: inline-flex; align-items: center; gap: 10px; height: 48px; padding: 0 18px; border-radius: 99px; background: var(--surface-2); color: var(--text-2); font-weight: 600; }
  .people { display: flex; flex-direction: column; gap: 14px; padding: 18px; border-radius: var(--r-xl); background: var(--surface); border: 1px solid var(--line); }
  .people header { display: flex; align-items: center; gap: 10px; color: var(--text-2); }
  .people h3 { flex: 1; color: var(--text); font-size: 1rem; font-weight: 650; }
  .people ul { display: grid; gap: 10px; flex: 1; align-content: start; }
  .people li { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .who { display: grid; flex: 1; min-width: 0; }
  .who strong { font-weight: 600; font-size: 0.92rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .who small { color: var(--text-3); font-size: 0.78rem; }
  .people .chip { height: 22px; font-size: 0.72rem; flex: none; }
  .alone { color: var(--text-3); font-size: 0.86rem; }
  .invite { width: 100%; justify-content: center; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr)); gap: 20px; }
  .room { border-radius: var(--r-l); background: var(--surface); border: 1px solid var(--line); overflow: hidden; transition: border-color var(--t), transform 400ms var(--ease-out); }
  .room:hover { border-color: var(--line-strong); transform: translateY(-2px); }
  .thumb { position: relative; display: block; width: 100%; }
  .thumb :global(.art) { border-radius: 0; }
  .blank { display: grid; place-items: center; aspect-ratio: 16 / 9; height: 100%; background: var(--surface-2); color: var(--text-3); }
  .state { position: absolute; left: 12px; top: 12px; display: inline-flex; align-items: center; gap: 8px; height: 26px; padding: 0 10px; border-radius: 99px; background: rgb(0 0 0 / 0.6); backdrop-filter: blur(10px); font-size: 0.76rem; font-weight: 650; }
  .go { position: absolute; inset: 0; display: grid; place-items: center; background: rgb(0 0 0 / 0.35); font-weight: 650; opacity: 0; transition: opacity var(--t); }
  .thumb:hover .go, .thumb:focus-visible .go { opacity: 1; }
  .meta { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 14px 16px; }
  .meta > div:first-child { display: grid; min-width: 0; }
  .meta strong { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .meta small { color: var(--text-3); font-size: 0.8rem; }
  @media (max-width: 860px) { .lobby { grid-template-columns: 1fr; } .stage .art { aspect-ratio: 16 / 9; } }
</style>
