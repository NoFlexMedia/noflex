<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import { untrack } from "svelte";
  import Check from "phosphor-svelte/lib/Check";
  import Sparkle from "phosphor-svelte/lib/Sparkle";
  import CheckCircle from "phosphor-svelte/lib/CheckCircle";
  import Heart from "phosphor-svelte/lib/Heart";
  import Books from "phosphor-svelte/lib/Books";
  import Broadcast from "phosphor-svelte/lib/Broadcast";
  import CardsThree from "phosphor-svelte/lib/CardsThree";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import ClosedCaptioning from "phosphor-svelte/lib/ClosedCaptioning";
  import ClockCounterClockwise from "phosphor-svelte/lib/ClockCounterClockwise";
  import DotsThree from "phosphor-svelte/lib/DotsThree";
  import DownloadSimple from "phosphor-svelte/lib/DownloadSimple";
  import Info from "phosphor-svelte/lib/Info";
  import Link from "phosphor-svelte/lib/Link";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import PencilSimple from "phosphor-svelte/lib/PencilSimple";
  import Play from "phosphor-svelte/lib/Play";
  import Plus from "phosphor-svelte/lib/Plus";
  import SpeakerHigh from "phosphor-svelte/lib/SpeakerHigh";
  import Star from "phosphor-svelte/lib/Star";
  import { goto } from "$app/navigation";
  import {
    cache,
    CollectionMenuStore,
    CreateCollectionStore,
    CreateExportJobStore,
    CreatePlaylistStore,
    MarkForSmartTranscodingStore,
    SmartMarksForWorkStore,
    UnmarkForSmartTranscodingStore,
    CreateWatchRoomStore,
    MarkPlayedStore,
    MarkUnplayedStore,
    MediaFilesStore,
    PlaylistMenuStore,
    ServerInfoStore,
    SetFavoriteStore,
    WorkCrumbStore,
    AddToPlaylistStore,
  } from "$houdini";
  import { addToCollection } from "$lib/collections.js";
  import { tilt } from "$lib/actions/index.js";
  import { bytes, count, runtime, techBadges, timecode, titleCase, versionLabel } from "$lib/format.js";
  import { failure, fetchQuery, refetch, writeUserData } from "$lib/graphql.js";
  import { ambientColor, pickImage } from "$lib/images.js";
  import { fetchChildren, player, playWork, subtitleFor } from "$lib/playback.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { deviceId } from "$lib/device.js";
  import { servers } from "$lib/servers.svelte.js";
  import { audioChoices, preferredAudio, preferredSubtitle, subtitleChoices } from "$lib/tracks.js";
  import { ambient, attempt, toast } from "$lib/ui.svelte.js";
  import Artwork from "$components/Artwork.svelte";
  import State from "$components/State.svelte";
  import AdminSheet from "$components/detail/AdminSheet.svelte";
  import Attributions from "$components/detail/Attributions.svelte";
  import Children from "$components/detail/Children.svelte";
  import Credits from "$components/detail/Credits.svelte";
  import TechSpecs from "$components/detail/TechSpecs.svelte";
  import Dialog from "$components/ui/Dialog.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Select from "$components/ui/Select.svelte";
  import Tip from "$components/ui/Tip.svelte";
  import { DropdownMenu } from "bits-ui";

  let { data: route } = $props();
  // Admin tools act on the server this page came from, where this user isn't known to be one.
  let manageable = $derived(app.viewer.isAdmin && !servers.active);

  let detail = $derived(route.detail);
  let work = $derived($detail.data?.work);
  // A match brings cast and tags as well as the card's fields.
  let wasIndexing = false;
  $effect(() => {
    const indexing = !!work?.indexing;
    if (wasIndexing && !indexing) untrack(() => refetch(detail));
    wasIndexing = indexing;
  });
  let editions = $derived($detail.data?.editions ?? []);
  let userData = $derived(work?.userData);

  let editionId = $state("");
  $effect(() => { if (!editions.some((edition) => edition.id === editionId)) editionId = editions[0]?.id ?? ""; });
  let edition = $derived(editions.find((item) => item.id === editionId));
  const media = new MediaFilesStore();
  $effect(() => { if (editionId) fetchQuery(media, { edition: editionId }); });
  let mediaFiles = $derived(editionId && $media.variables?.edition === editionId ? $media.data?.mediaFiles ?? [] : []);
  let mediaId = $state("");
  // Files sharing a part index are alternate versions (1080p vs 4K);
  // distinct indexes are parts of one long cut.
  let multiPart = $derived(new Set(mediaFiles.map((file) => file.partIndex)).size > 1);
  const fileLabel = (file) => [multiPart ? `Part ${file.partIndex + 1}` : "", versionLabel(file.probe) ?? (multiPart ? "" : file.path.split("/").pop())].filter(Boolean).join(" · ");
  $effect(() => { if (!mediaFiles.some((file) => file.id === mediaId)) mediaId = mediaFiles[0]?.id ?? ""; });
  let mediaFile = $derived(mediaFiles.find((file) => file.id === mediaId));

  // What to start with, seeded from the viewer's preferences for each file.
  let audioStreams = $derived(mediaFile?.probe?.audio ?? []);
  let subtitleTracks = $derived(mediaFile?.subtitleTracks ?? []);
  let audioChoice = $state("");
  let subtitleChoice = $state("off");
  $effect(() => {
    if (!mediaId) return;
    const audio = preferredAudio(audioStreams, app.playbackPrefs);
    const subtitle = preferredSubtitle(subtitleTracks, app.playbackPrefs);
    audioChoice = audio == null ? "" : String(audio);
    subtitleChoice = subtitle == null ? "off" : String(subtitle);
  });
  let chosenAudio = $derived(audioChoice === "" ? null : audioStreams[Number(audioChoice)]);

  const parent = new WorkCrumbStore();
  const grandparent = new WorkCrumbStore();
  let parentId = $derived(work?.parentId);
  $effect(() => { if (parentId) fetchQuery(parent, { id: parentId }); });
  let parentWork = $derived(parentId && $parent.data?.work?.id === parentId ? $parent.data.work : null);
  let grandparentId = $derived(parentWork?.parentId);
  $effect(() => { if (grandparentId) fetchQuery(grandparent, { id: grandparentId }); });
  let grandparentWork = $derived(grandparentId && $grandparent.data?.work?.id === grandparentId ? $grandparent.data.work : null);

  // ---- smart transcoding marks --------------------------------------------
  // Marks live on the home server, like the devices they name.
  const smartMarks = new SmartMarksForWorkStore();
  const markForSmart = new MarkForSmartTranscodingStore();
  const unmarkForSmart = new UnmarkForSmartTranscodingStore();
  const thisDevice = deviceId();
  let canMark = $derived(!servers.active && ["MOVIE", "SERIES", "SEASON", "EPISODE"].includes(work?.kind));
  let workMarks = $derived($smartMarks.data?.mySmartTranscodeMarks ?? []);
  let markedForDevices = $derived(workMarks.some((mark) => !mark.device));
  let markedForHere = $derived(workMarks.some((mark) => mark.device?.id === thisDevice));
  const MARK_STATE = { WAITING: "In the queue", READY: "Ready", NOT_NEEDED: "Plays as it is", TOO_LARGE: "Too large", NO_DEVICES: "No devices", NO_SPACE: "Waiting for space", FAILED: "Failed", DISABLED: "Off on this server" };
  const markState = (mark) => (!mark ? "" : mark.state === "PREPARING" ? `Preparing ${Math.round(mark.progress * 100)}%` : MARK_STATE[mark.state] ?? "");
  let stateForDevices = $derived(markState(workMarks.find((mark) => !mark.device)));
  let stateForHere = $derived(markState(workMarks.find((mark) => mark.device?.id === thisDevice)));
  $effect(() => {
    if (!workMarks.some((mark) => mark.state === "WAITING" || mark.state === "PREPARING")) return;
    const store = smartMarks;
    return poll(() => refetch(store), 5000);
  });
  $effect(() => { if (canMark && work?.id) fetchQuery(smartMarks, { work: work.id }); });

  async function toggleMark(device) {
    const marked = device ? markedForHere : markedForDevices;
    const store = marked ? unmarkForSmart : markForSmart;
    if ((await attempt(() => store.mutate({ work: work.id, device }))) === null) return;
    refetch(smartMarks);
    if (marked) { toast("Unmarked", "success"); return; }
    const view = { label: "View", run: () => goto("/account?tab=smart") };
    if (device) toast("It’ll be ready for this device next", "success", view);
    else if ($smartMarks.data?.myDevices.some((known) => known.smartTranscode)) toast("Marked. It’ll be ready next on your devices with smart transcoding on", "success", view);
    else toast("Marked, but none of your devices has smart transcoding on yet", "info", { label: "Choose devices", run: () => goto("/account?tab=devices") });
  }

  // ---- new playlist or collection, with this title in it ------------------
  const createPlaylist = new CreatePlaylistStore();
  const createCollection = new CreateCollectionStore();
  let newList = $state({ open: false, kind: "playlist", title: "", saving: false });

  async function saveNewList(event) {
    event.preventDefault();
    newList.saving = true;
    const title = newList.title.trim();
    const library = libraryId;
    const isCollection = newList.kind === "collection";
    const done = await attempt(async () => {
      if (isCollection) {
        const { data } = await createCollection.mutate({ library, title, overview: null });
        // Collection lists are per library; only that library's take the new one.
        const record = cache.get("Collection", { id: data.createCollection.id });
        for (const name of ["Collections", "CollectionMenu"]) cache.list(name).when({ must: { libraryId: library } }).append(record);
        await addToCollection(data.createCollection.id, work.id);
      } else {
        const { data } = await createPlaylist.mutate({ title, overview: null });
        await addPlaylistItem.mutate({ id: data.createPlaylist.id, items: [{ kind: "WORK", id: work.id }] });
      }
    }, `Added to “${title}”`);
    newList.saving = false;
    if (done !== null) { newList.open = false; refetch(playlists); }
  }

  const playlists = new PlaylistMenuStore();
  $effect(() => { fetchQuery(playlists); });
  const collections = new CollectionMenuStore();
  let libraryId = $derived(work?.libraryId);
  $effect(() => { if (manageable && libraryId) fetchQuery(collections, { library: libraryId }); });

  const setFavorite = new SetFavoriteStore();
  const markPlayed = new MarkPlayedStore();
  const markUnplayed = new MarkUnplayedStore();
  const addPlaylistItem = new AddToPlaylistStore();
  const createExport = new CreateExportJobStore();
  // Whichever server this title is on decides, so this asks the active one rather than reading the session.
  const info = new ServerInfoStore();
  $effect(() => { fetchQuery(info); });
  let downloadsAllowed = $derived($info.data?.serverInfo.downloadsAllowed ?? true);
  const createWatchRoom = new CreateWatchRoomStore();

  let adminOpen = $state(false);
  let expanded = $state(false);
  let heartBurst = $state(0);
  let isContainer = $derived(["SERIES", "SEASON", "ARTIST", "ALBUM"].includes(work?.kind));
  let resumeMs = $derived(userData && !userData.played ? userData.positionMs ?? 0 : 0);
  let duration = $derived(edition?.runtimeMs ?? work?.runtimeMs ?? 0);
  let progress = $derived(duration && resumeMs ? Math.min(1, resumeMs / duration) : 0);
  let crumbs = $derived([grandparentWork, parentWork].filter(Boolean));
  let backdropWork = $derived(pickImage(work?.images, "backdrop") ? work : crumbs.findLast((item) => pickImage(item.images, "backdrop")) ?? work);

  // For a show or season, the episode to start from: in progress first,
  // then the first unwatched, else the very first (a rewatch).
  let upNext = $state(null);
  // Per season id: episode count and how many are watched, for the season tiles.
  let seasonStats = $state({});
  let workId = $derived(work?.id);
  let workKind = $derived(work?.kind);
  $effect(() => {
    upNext = null;
    seasonStats = {};
    if (!workId || !["SERIES", "SEASON"].includes(workKind)) return;
    let stale = false;
    (async () => {
      const top = await fetchChildren(libraryId, workId);
      const groups = top.some((item) => item.kind === "SEASON")
        ? await Promise.all(top.filter((item) => item.kind === "SEASON").map((season) => fetchChildren(libraryId, season.id)))
        : [top];
      const seasonIds = top.filter((item) => item.kind === "SEASON").map((item) => item.id);
      const stats = Object.fromEntries(seasonIds.map((id, index) => [id, { total: groups[index].length, watched: groups[index].filter((item) => item.userData?.played).length }]));
      const episodes = groups.flat();
      const started = episodes.find((item) => !item.userData?.played && item.userData?.positionMs > 0);
      const next = started ?? episodes.find((item) => !item.userData?.played);
      if (!stale) seasonStats = stats;
      if (!stale) upNext = next ? { work: next, resume: !!started } : episodes[0] ? { work: episodes[0], rewatch: true } : null;
    })().catch(() => {});
    return () => { stale = true; };
  });

  $effect(() => {
    const url = pickImage(backdropWork?.images, "backdrop") ?? pickImage(backdropWork?.images, "poster");
    if (!url) { ambient.color = null; return; }
    let stale = false;
    ambientColor(url).then((color) => { if (!stale) ambient.color = color; });
    return () => { stale = true; ambient.color = null; };
  });

  let credits = $derived($detail.data?.work?.credits ?? []);
  let tags = $derived($detail.data?.work?.tags ?? []);
  let parental = $derived(tags.find((tag) => tag.namespace === "parental")?.display);
  let genres = $derived(tags.filter((tag) => tag.namespace === "genre").map((tag) => tag.display));

  function play(fromStart = false, positionMs = undefined) {
    return playWork(work, {
      editionId,
      mediaFileId: mediaId || undefined,
      fromStart,
      positionMs,
      audioStream: audioChoice === "" ? undefined : Number(audioChoice),
      subtitleIndex: subtitleChoice === "off" ? null : Number(subtitleChoice),
    });
  }

  async function toggleFavorite() {
    const before = userData;
    const favorite = !before.favorite;
    if (favorite) heartBurst++;
    writeUserData(work.id, { ...before, favorite });
    const result = await attempt(() => setFavorite.mutate({ id: work.id, favorite }));
    writeUserData(work.id, result ? result.data.setFavorite : before);
  }

  let playedPop = $state(0);
  async function togglePlayed() {
    // Optimistic like favorites: flip now, settle on the server's answer.
    const before = userData;
    const played = !before.played;
    if (played) playedPop++;
    writeUserData(work.id, { ...before, played });
    const result = await attempt(() => (played ? markPlayed : markUnplayed).mutate({ id: work.id }), played ? "Marked as watched" : "Marked as unwatched");
    writeUserData(work.id, result ? (played ? result.data.markPlayed : result.data.markUnplayed) : before);
  }

  async function addToPlaylist(playlist) {
    await attempt(() => addPlaylistItem.mutate({ id: playlist.id, items: [{ kind: "WORK", id: work.id }] }), `Added to “${playlist.title}”`);
  }

  async function addToLibraryCollection(collection) {
    await attempt(() => addToCollection(collection.id, work.id), `Added to “${collection.title}”`);
  }

  async function exportEdition(format) {
    const result = await attempt(() => createExport.mutate({ edition: editionId, format }));
    if (result) toast("Making an offline copy — follow it under Account → Offline copies.", "success", { label: "View", run: () => goto("/account?tab=downloads") });
  }

  async function createRoom() {
    const result = await attempt(() => createWatchRoom.mutate({ media: mediaId }));
    if (result) goto(`/watch-rooms?room=${result.data.createWatchRoom.id}`);
  }

  function copyLink() {
    navigator.clipboard?.writeText(location.href).then(() => toast("Link copied", "success"), () => toast("Couldn’t access the clipboard", "error"));
  }
</script>

<svelte:head><title>{work?.title ?? "Loading"} · NoFlex</title></svelte:head>

{#if $detail.fetching && !$detail.data}
  <div class="page detail">
    <div class="layout" aria-hidden="true">
      <div class="poster skeleton"></div>
      <div class="stack"><div class="skeleton" style:height="18px" style:width="120px"></div><div class="skeleton" style:height="52px" style:width="min(520px, 80%)"></div><div class="skeleton" style:height="88px" style:width="min(640px, 90%)"></div></div>
    </div>
  </div>
{:else if failure($detail) && !work}
  <div class="page"><State kind="error" title="Couldn’t open this title" message={failure($detail)} action={() => refetch(detail)} /></div>
{:else if !work}
  <div class="page"><State icon={MagnifyingGlass} title="This title isn’t here" message="It may have been removed, or you don’t have access to its library."><a class="btn sm" href="/">Go home</a></State></div>
{:else}
  <div class="backdrop" aria-hidden="true">
    {#key backdropWork?.id}<Artwork work={backdropWork} kind="backdrop" width={1600} eager rounded={false} />{/key}
  </div>

  <div class="page detail">
    {#if crumbs.length}
      <nav class="crumbs" aria-label="Breadcrumb">
        {#each crumbs as crumb}<a href="/works/{crumb.id}">{crumb.title}</a><CaretRight size={14} />{/each}
        <span aria-current="page">{subtitleFor(work) || work.title}</span>
      </nav>
    {/if}

    <div class="layout">
      <div class="poster-col">
        <div class="poster" use:tilt={5}><Artwork {work} kind={work.kind === "EPISODE" ? "thumb" : "poster"} width={300} eager vt="poster" /></div>
      </div>

      <section class="info">
        <p class="eyebrow">{titleCase(work.kind)}{#if work.metadataStatus === "UNCONFIRMED" && !work.indexing}{" · "}<span class="unconfirmed">Unconfirmed match</span>{/if}</p>
        {#if pickImage(work.images, "logo")}
          <h1 class="logo"><span class="sr-only">{work.title}</span><Artwork {work} kind="logo" width={460} rounded={false} /></h1>
        {:else}
          <h1>{work.title}</h1>
        {/if}
        {#if work.originalTitle && work.originalTitle !== work.title}<p class="original">{work.originalTitle}</p>{/if}

        <div class="meta">
          {#if work.communityRating}<span class="rating"><Star size={14} weight="fill" />{work.communityRating.toFixed(1)}</span>{/if}
          {#each [subtitleFor(work) !== String(work.year) ? subtitleFor(work) : "", work.year, runtime(duration)].filter(Boolean) as part}<span class="part">{part}</span>{/each}
          {#if parental}<span class="badge">{parental}</span>{/if}
        </div>
        {#if genres.length}<p class="genres">{genres.join(" · ")}</p>{/if}
        {#if !isContainer && edition}
          <div class="tech" aria-label="Video and audio format">{#each techBadges(mediaFile?.probe, chosenAudio) as badge}<span class="badge">{badge}</span>{/each}</div>
        {/if}

        {#if work.tagline}<p class="tagline">{work.tagline}</p>{/if}
        {#if work.overview}
          <button class="overview" class:expanded onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
            <span class:clamp-3={!expanded}>{work.overview}</span>
          </button>
        {/if}

        <div class="actions">
          {#if upNext}
            <button class="btn light lg play" onclick={() => playWork(upNext.work)} disabled={player.preparing === upNext.work.id}>
              {#if player.preparing === upNext.work.id}<span class="spinner sm"></span>{:else}<Play size={19} weight="fill" />{/if}
              {upNext.rewatch ? "Watch again" : upNext.resume ? "Resume" : "Play"}{#if subtitleFor(upNext.work)} <span class="next-ep">{subtitleFor(upNext.work)}</span>{/if}
            </button>
          {/if}
          {#if !isContainer}
            <div class="play-wrap">
              <button class="btn light lg play" onclick={() => play(false)} disabled={!mediaId || player.preparing === work.id}>
                {#if player.preparing === work.id}<span class="spinner sm"></span>{:else}<Play size={19} weight="fill" />{/if}
                {resumeMs > 30_000 ? `Resume from ${timecode(resumeMs)}` : "Play"}
              </button>
              {#if progress > 0}<span class="play-progress"><span style:width="{progress * 100}%"></span></span>{/if}
            </div>
            {#if resumeMs > 30_000}
              <Tip text="Play from the beginning">{#snippet children(props)}<button {...props} class="icon-btn glass lg-icon" aria-label="Play from the beginning" onclick={() => play(true)}><ClockCounterClockwise size={20} /></button>{/snippet}</Tip>
            {/if}
          {/if}
          <Tip text={userData?.favorite ? "Remove from favorites" : "Add to favorites"}>
            {#snippet children(props)}
              <button {...props} class="icon-btn glass lg-icon heart" class:on={userData?.favorite} aria-pressed={userData?.favorite} aria-label="Favorite" onclick={toggleFavorite}>
                {#key heartBurst}<span class="burst" class:go={heartBurst > 0}></span>{/key}
                <Heart size={20} weight={userData?.favorite ? "fill" : "regular"} />
              </button>
            {/snippet}
          </Tip>
          <Tip text={userData?.played ? "Mark unwatched" : "Mark watched"}>
            {#snippet children(props)}<button {...props} class="icon-btn glass lg-icon watched-btn" class:on={userData?.played} aria-pressed={userData?.played} aria-label="Watched" onclick={togglePlayed}>{#key playedPop}<span class="burst accent" class:go={playedPop > 0}></span><span class="pop" class:go={playedPop > 0}>{#if userData?.played}<CheckCircle size={20} />{:else}<Check size={20} />{/if}</span>{/key}</button>{/snippet}
          </Tip>
          <Menu width={290} align="start">
            {#snippet trigger(props)}<button {...props} class="icon-btn glass lg-icon" aria-label="More actions"><DotsThree size={20} weight="bold" /></button>{/snippet}
            <DropdownMenu.Sub>
              <DropdownMenu.SubTrigger class="nf-menu-item"><CardsThree size={17} /><span>Add to playlist</span><CaretRight size={15} /></DropdownMenu.SubTrigger>
              <DropdownMenu.Portal>
              <DropdownMenu.SubContent class="nf-menu" sideOffset={6} collisionPadding={12} style="--menu-w: 240px">
                {#each $playlists.data?.playlists.edges.map((edge) => edge.node) ?? [] as playlist (playlist.id)}<MenuItem onSelect={() => addToPlaylist(playlist)}>{playlist.title}</MenuItem>{:else}<div class="nf-menu-label">No playlists yet</div>{/each}
                <div class="nf-menu-sep"></div>
                <MenuItem icon={Plus} onSelect={() => (newList = { open: true, kind: "playlist", title: "", saving: false })}>New playlist…</MenuItem>
              </DropdownMenu.SubContent>
              </DropdownMenu.Portal>
            </DropdownMenu.Sub>
            {#if manageable}
              <DropdownMenu.Sub>
                <DropdownMenu.SubTrigger class="nf-menu-item"><Books size={17} /><span>Add to collection</span><CaretRight size={15} /></DropdownMenu.SubTrigger>
                <DropdownMenu.Portal>
                <DropdownMenu.SubContent class="nf-menu" sideOffset={6} collisionPadding={12} style="--menu-w: 240px">
                  {#each $collections.data?.collections.edges.map((edge) => edge.node) ?? [] as collection (collection.id)}<MenuItem onSelect={() => addToLibraryCollection(collection)}>{collection.title}</MenuItem>{:else}<div class="nf-menu-label">No collections in this library</div>{/each}
                  <div class="nf-menu-sep"></div>
                  <MenuItem icon={Plus} onSelect={() => (newList = { open: true, kind: "collection", title: "", saving: false })}>New collection…</MenuItem>
                </DropdownMenu.SubContent>
                </DropdownMenu.Portal>
              </DropdownMenu.Sub>
            {/if}
            {#if canMark}
              <div class="nf-menu-sep"></div>
              <MenuItem icon={markedForDevices ? Check : Sparkle} onSelect={() => toggleMark(null)}>Mark for smart transcoding{#if stateForDevices} · {stateForDevices}{/if}</MenuItem>
              {#if thisDevice}<MenuItem icon={markedForHere ? Check : Sparkle} onSelect={() => toggleMark(thisDevice)}>Smart transcode for this device{#if stateForHere} · {stateForHere}{/if}</MenuItem>{/if}
              <div class="nf-menu-sep"></div>
            {/if}
            {#if !isContainer}
              <MenuItem icon={Broadcast} disabled={!mediaId} onSelect={createRoom}>Start a watch room</MenuItem>
              {#if downloadsAllowed}
                <MenuItem icon={DownloadSimple} disabled={!editionId} onSelect={() => exportEdition("MP4")}>Make an offline copy (MP4)</MenuItem>
                <MenuItem icon={DownloadSimple} disabled={!editionId} onSelect={() => exportEdition("MKV")}>Make an offline copy (MKV)</MenuItem>
              {/if}
            {/if}
            <MenuItem icon={Link} onSelect={copyLink}>Copy link</MenuItem>
            {#if manageable}
              <div class="nf-menu-sep"></div>
              <MenuItem icon={PencilSimple} onSelect={() => (adminOpen = true)}>Edit, identify & manage…</MenuItem>
            {/if}
          </Menu>
        </div>

        {#if !isContainer && (audioStreams.length > 1 || subtitleTracks.length)}
          <div class="tracks">
            {#if audioStreams.length > 1}
              <div class="pick"><SpeakerHigh size={18} /><Select bind:value={audioChoice} items={audioChoices(audioStreams)} label="Audio track" /></div>
            {/if}
            {#if subtitleTracks.length}
              <div class="pick"><ClosedCaptioning size={18} /><Select bind:value={subtitleChoice} items={subtitleChoices(subtitleTracks)} label="Subtitles" /></div>
            {/if}
          </div>
        {/if}

        {#if !isContainer && (editions.length > 1 || mediaFiles.length > 1)}
          <div class="versions">
            {#if editions.length > 1}
              <div class="choice" role="radiogroup" aria-label="Edition">
                <span class="eyebrow">Edition</span>
                {#each editions as item (item.id)}
                  <button role="radio" aria-checked={item.id === editionId} class:on={item.id === editionId} onclick={() => (editionId = item.id)}>{item.name}{#if item.runtimeMs}<small>{runtime(item.runtimeMs)}</small>{/if}</button>
                {/each}
              </div>
            {/if}
            {#if mediaFiles.length > 1}
              <div class="choice" role="radiogroup" aria-label={multiPart ? "Part" : "Version"}>
                <span class="eyebrow">{multiPart && mediaFiles.length === new Set(mediaFiles.map((file) => file.partIndex)).size ? "Part" : "Version"}</span>
                {#each mediaFiles as file (file.id)}
                  <button role="radio" aria-checked={file.id === mediaId} class:on={file.id === mediaId} onclick={() => (mediaId = file.id)}>{fileLabel(file)}<small>{file.container?.toUpperCase() ?? ""} · {bytes(file.sizeBytes)}</small></button>
                {/each}
              </div>
            {/if}
          </div>
        {:else if !isContainer && !editions.length}
          <p class="note"><Info size={16} />No playable files are attached to this title yet.</p>
        {/if}
      </section>
    </div>

    {#if isContainer}
      <Children {work} art={backdropWork} {seasonStats} upNextId={upNext && !upNext.rewatch ? upNext.work.id : null} />
    {/if}

    <Credits {credits} />

    {#if work.ratings.length || work.communityRating || parental}
      <section class="section ratings">
        <div class="section-head"><h2>Ratings</h2></div>
        <div class="scores">
          {#each work.ratings as rating (rating.source)}
            <div class="score">
              <strong class="tabular">{rating.display}{#if rating.outOf}<small>{rating.outOf}</small>{/if}</strong>
              <span class="stars" role="img" aria-label="{rating.display}{rating.outOf ?? ''} on {rating.name}"><span class="meter" style:width="{rating.normalized * 10}%"></span></span>
              <span class="faint">{rating.name}{#if rating.votes} · {count(rating.votes)} votes{/if}</span>
            </div>
          {:else}
            {#if work.communityRating}
              <div class="score">
                <strong class="tabular">{work.communityRating.toFixed(1)}<small>/10</small></strong>
                <span class="stars" role="img" aria-label="{work.communityRating.toFixed(1)} out of 10"><span class="meter" style:width="{work.communityRating * 10}%"></span></span>
                <span class="faint">Community score</span>
              </div>
            {/if}
          {/each}
          {#if parental}
            <div class="score"><strong>{parental}</strong><span class="faint">Parental rating</span></div>
          {/if}
        </div>
      </section>
    {/if}

    {#if edition && !isContainer}
      <TechSpecs {edition} file={mediaFile} audio={chosenAudio} onchapter={(ms) => play(false, ms)} />
    {/if}

    <Attributions attributions={work.attributions} />
  </div>

  <Dialog bind:open={newList.open} title={newList.kind === "collection" ? "New collection" : "New playlist"} description="“{work.title}” goes in it." width={460}>
    <form id="new-list" class="stack" onsubmit={saveNewList}>
      <label class="field">Title<input bind:value={newList.title} required placeholder={newList.kind === "collection" ? "The Criterion shelf" : "Sunday night"} /></label>
    </form>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (newList.open = false)}>Cancel</button>
      <button class="btn primary" form="new-list" disabled={newList.saving || !newList.title.trim()}>{#if newList.saving}<span class="spinner sm"></span>{/if}Create</button>
    {/snippet}
  </Dialog>

  {#if manageable}
    <AdminSheet bind:open={adminOpen} {work} {edition} media={mediaFile} onremoved={() => { refetch(detail); refetch(media); }} />
  {/if}
{/if}

<style>
  .backdrop { position: absolute; inset: 0 0 auto 0; height: min(78vh, 820px); z-index: -1; overflow: hidden; mask-image: linear-gradient(to bottom, #000 30%, transparent 100%); }
  .backdrop :global(.art) { position: absolute; inset: 0; aspect-ratio: auto; height: 100%; }
  .backdrop :global(img) { opacity: 0.55 !important; }
  .backdrop::after { content: ""; position: absolute; inset: 0; background: linear-gradient(90deg, rgb(7 8 11 / 0.92), rgb(7 8 11 / 0.5) 45%, rgb(7 8 11 / 0.2)), radial-gradient(80% 60% at 70% 20%, transparent, rgb(7 8 11 / 0.4)); }
  .detail { padding-top: calc(var(--topbar-h) + clamp(28px, 10vh, 120px)); }
  .crumbs { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-bottom: 18px; color: var(--text-3); font-size: 0.88rem; font-weight: 550; }
  .crumbs a { color: var(--text-2); transition: color var(--t); }
  .crumbs a:hover { color: var(--text); }
  .layout { display: grid; grid-template-columns: clamp(180px, 20vw, 290px) minmax(0, 1fr); gap: clamp(24px, 4vw, 56px); align-items: start; }
  .poster { border-radius: var(--r-l); box-shadow: 0 40px 80px -30px rgb(0 0 0 / 0.95), 0 0 0 1px rgb(255 255 255 / 0.06); transform: perspective(900px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg)); transition: transform 600ms var(--ease-out); }
  .poster:global([data-tilting]) { transition: transform 120ms linear; }
  .poster.skeleton { aspect-ratio: 2 / 3; }
  .poster :global(.art) { border-radius: var(--r-l); }
  .info { display: grid; gap: 14px; max-width: 860px; }
  .unconfirmed { color: var(--warning); }
  h1 { font-size: clamp(2.2rem, 4.8vw, 4rem); font-weight: 750; letter-spacing: -0.045em; line-height: 1; animation: fade-up 600ms var(--ease-out) both; }
  h1.logo { width: min(460px, 90%); }
  h1.logo :global(.art) { aspect-ratio: 3 / 1; }
  .original { color: var(--text-3); font-size: 0.96rem; margin-top: -6px; }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 14px; color: var(--text-2); font-weight: 550; font-size: 0.94rem; }
  .meta .part + .part::before { content: "·"; margin-right: 14px; color: var(--text-3); }
  .rating { display: inline-flex; align-items: center; gap: 5px; color: var(--warning); }
  .meta .badge { margin-left: 2px; }
  .genres { color: var(--text-3); font-size: 0.92rem; margin-top: -6px; }
  .tech { display: flex; flex-wrap: wrap; gap: 6px; }
  .tracks { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 6px; }
  .pick { display: flex; align-items: center; gap: 8px; width: min(100%, 230px); color: var(--text-3); }
  .pick :global(.nf-select) { min-height: 38px; font-size: 0.88rem; }
  .ratings { margin-top: 56px; }
  .scores { display: flex; flex-wrap: wrap; gap: 14px; }
  .score { display: grid; gap: 4px; min-width: 170px; padding: 16px 20px; border-radius: var(--r-l); background: var(--surface); border: 1px solid var(--line); font-size: 0.84rem; }
  .score strong { font-size: 1.7rem; font-weight: 700; letter-spacing: -0.03em; line-height: 1.1; }
  .score strong small { font-size: 0.86rem; font-weight: 550; color: var(--text-3); letter-spacing: 0; margin-left: 2px; }
  .stars { height: 4px; border-radius: 9px; background: var(--surface-3); overflow: hidden; }
  .meter { display: block; height: 100%; background: var(--warning); }
  .tagline { font-size: 1.08rem; font-style: italic; color: var(--text-2); }
  .overview { text-align: left; color: var(--text-2); font-size: 1.02rem; line-height: 1.6; max-width: 720px; cursor: pointer; border-radius: 8px; }
  .overview:hover { color: var(--text); }
  .actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-top: 8px; }
  .play-wrap { display: grid; gap: 7px; }
  .play { min-width: 168px; }
  .next-ep { font-weight: 500; opacity: 0.6; }
  .play-progress { height: 3px; border-radius: 9px; background: rgb(255 255 255 / 0.16); overflow: hidden; margin: 0 18px; }
  .play-progress span { display: block; height: 100%; background: var(--accent); }
  .lg-icon { width: 50px; height: 50px; }
  .lg-icon.on { color: var(--accent); }
  .heart { position: relative; }
  .heart.on { color: #ff6b8b; }
  .heart :global(svg) { transition: transform 400ms var(--ease-spring); }
  .heart:active :global(svg) { transform: scale(0.8); }
  .burst { position: absolute; inset: 0; border-radius: 50%; pointer-events: none; }
  .burst.go { animation: burst 600ms var(--ease-out); }
  @keyframes burst { 0% { box-shadow: 0 0 0 0 rgb(255 107 139 / 0.7); } 100% { box-shadow: 0 0 0 18px rgb(255 107 139 / 0); } }
  .burst.accent.go { animation-name: burst-accent; }
  @keyframes burst-accent { 0% { box-shadow: 0 0 0 0 rgb(212 255 90 / 0.6); } 100% { box-shadow: 0 0 0 18px rgb(212 255 90 / 0); } }
  .watched-btn { position: relative; }
  .watched-btn.on { color: var(--accent); }
  .pop { display: grid; place-items: center; }
  .pop.go { animation: pop 520ms var(--ease-spring); }
  @keyframes pop { 0% { transform: scale(0.6); } 100% { transform: scale(1); } }
  .versions { display: grid; gap: 12px; margin-top: 10px; }
  .choice { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .choice .eyebrow { width: 72px; }
  .choice button { display: inline-flex; align-items: baseline; gap: 8px; height: 36px; padding: 0 14px; border-radius: 999px; border: 1px solid var(--line-strong); color: var(--text-2); font-weight: 600; font-size: 0.86rem; transition: all var(--t) var(--ease-out); align-items: center; }
  .choice button small { color: var(--text-3); font-weight: 500; }
  .choice button:hover { border-color: rgb(255 255 255 / 0.3); color: var(--text); }
  .choice button.on { background: var(--text); color: #0b0c0f; border-color: var(--text); }
  .choice button.on small { color: rgb(0 0 0 / 0.55); }
  .note { display: flex; gap: 8px; align-items: center; color: var(--text-3); font-size: 0.9rem; }
  @media (max-width: 759px) {
    .layout { grid-template-columns: 1fr; }
    .poster-col { width: 44%; max-width: 200px; }
    .detail { padding-top: calc(var(--topbar-h) + 16px); }
    .play { flex: 1; }
    .play-wrap { flex: 1 1 100%; }
  }
</style>
