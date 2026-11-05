<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Tabs as BitsTabs } from "bits-ui";
  import { LiveTvGuideStore, ScheduleRecordingStore, StartLiveChannelStore } from "$houdini";
  import { failure, fetchQuery, nextPage, refetch } from "$lib/graphql.js";
  import { player } from "$lib/playback.svelte.js";
  import { app } from "$lib/session.svelte.js";
  import { attempt, poll } from "$lib/ui.svelte.js";
  import State from "$components/State.svelte";
  import Channels from "$components/livetv/Channels.svelte";
  import Dvr from "$components/livetv/Dvr.svelte";
  import Guide from "$components/livetv/Guide.svelte";
  import Sources from "$components/livetv/Sources.svelte";
  import Select from "$components/ui/Select.svelte";
  import Tabs from "$components/ui/Tabs.svelte";

  const HALF_HOUR = 30 * 60_000;
  const SPAN = 12 * 60 * 60_000;
  const GUIDE_REFRESH = 5 * 60_000;

  let tabs = $derived(["guide", "channels", "dvr", ...(app.viewer.isAdmin ? ["sources"] : [])]);
  let tab = $derived(tabs.includes(page.url.searchParams.get("tab") ?? "") ? page.url.searchParams.get("tab") : "guide");
  function setTab(next) {
    goto(next === "guide" ? "?" : `?tab=${next}`, { replaceState: true, noScroll: true, keepFocus: true });
  }

  let { data: route } = $props();
  let live = $derived(route.live);
  const byNumber = (a, b) => (parseFloat(a.number) || Infinity) - (parseFloat(b.number) || Infinity) || a.name.localeCompare(b.name);
  let allChannels = $derived([...($live.data?.liveTvChannels.nodes ?? [])].filter((channel) => channel.enabled).sort(byNumber));
  let groups = $derived([...new Set(allChannels.map((channel) => channel.group).filter(Boolean))].sort());
  let group = $state("");
  let channels = $derived(group ? allChannels.filter((channel) => channel.group === group) : allChannels);
  let channelIds = $derived(allChannels.slice(0, 200).map((channel) => channel.id));

  // The guide window is anchored, not sliding, so its cache key stays put between refreshes.
  const anchorFor = (time) => Math.floor(time / HALF_HOUR) * HALF_HOUR - HALF_HOUR;
  let now = $state(Date.now());
  let anchor = $state(anchorFor(Date.now()));
  let windowEnd = $derived(anchor + SPAN);
  const guide = new LiveTvGuideStore();
  let channelKey = $derived(channelIds.join());
  $effect(() => {
    if (!channelKey) return;
    fetchQuery(guide, { channelIds: channelKey.split(","), start: new Date(anchor).toISOString(), end: new Date(windowEnd).toISOString() });
  });
  // Every page of the window, so no channel's row comes up short.
  let requested = "";
  $effect(() => {
    const { hasNextPage, endCursor } = $guide.pageInfo;
    if (!hasNextPage || !endCursor || endCursor === requested) return;
    requested = endCursor;
    nextPage(guide).catch(() => { requested = ""; });
  });
  let programs = $derived(($guide.data?.liveTvGuide.edges ?? []).map((edge) => edge.node));
  let guideError = $derived(failure($guide));
  let guideAt = Date.now();

  $effect(() => {
    const store = live;
    return poll(() => {
      now = Date.now();
      if (now > anchor + SPAN - 3 * 60 * 60_000) anchor = anchorFor(now);
      refetch(store);
      if (Date.now() - guideAt > GUIDE_REFRESH) { guideAt = Date.now(); refetch(guide); }
    }, 30_000);
  });

  let clock = $state(new Date());
  $effect(() => {
    let timer = setTimeout(function tick() { clock = new Date(); timer = setTimeout(tick, 1000 - (Date.now() % 1000)); }, 1000 - (Date.now() % 1000));
    return () => clearTimeout(timer);
  });
  const clockFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });
  // Parts, not a string: the locale decides where seconds fall relative to AM/PM.
  let clockParts = $derived(clockFormat.formatToParts(clock));
  let secondsAt = $derived(clockParts.findIndex((part) => part.type === "second"));

  let starting = $state("");
  const startChannel = new StartLiveChannelStore();
  const scheduleRecording = new ScheduleRecordingStore();
  let onNow = $derived.by(() => {
    const map = new Map();
    for (const program of programs) {
      if (Date.parse(program.startAt) <= now && Date.parse(program.endAt) > now) map.set(program.channelId, program);
    }
    return map;
  });

  async function play(channel) {
    if (starting) return;
    starting = channel.id;
    const result = await attempt(() => startChannel.mutate({ id: channel.id }));
    starting = "";
    if (result) player.current = { ...result.data.startLiveTvChannel, title: channel.name, subtitle: onNow.get(channel.id)?.title, workId: null, live: true };
  }

  async function record(program) {
    const result = await attempt(() => scheduleRecording.mutate({ channel: program.channelId, program: program.id, start: program.startAt, end: program.endAt, before: 30, after: 60 }), `“${program.title}” will be recorded`);
    if (result) refetch(live);
    return Boolean(result);
  }

  let schedules = $derived($live.data?.recordingSchedules.nodes ?? []);
  let scheduled = $derived(new Set(schedules.filter((item) => item.status === "SCHEDULED" && item.programId).map((item) => item.programId)));
  let recordingCount = $derived(($live.data?.recordings.nodes ?? []).filter((item) => item.status === "RUNNING").length);
</script>

<svelte:head><title>Live TV · NoFlex</title></svelte:head>

<div class="page">
  <header class="page-head">
    <div>
      <p class="eyebrow">Live & recorded</p>
      <h1>Live TV</h1>
      <p>{allChannels.length ? `${allChannels.length} channels${recordingCount ? ` · ${recordingCount} recording now` : ""}` : "Channels, guide and DVR in one place."}</p>
    </div>
    <div class="actions">
      {#if groups.length > 1 && (tab === "guide" || tab === "channels")}
        <Select pill bind:value={group} items={[{ value: "", label: "All channels" }, ...groups.map((name) => ({ value: name, label: name }))]} label="Channel group" />
      {/if}
      <div class="clock" role="timer" aria-label="Current time">
        <span class="dot live"></span><span class="label">Live</span>
        <time class="tabular">{#each clockParts as part, index}{#if index === secondsAt || index === secondsAt - 1}<span class="sec">{part.value}</span>{:else}{part.value}{/if}{/each}</time>
      </div>
    </div>
  </header>

  <Tabs value={tab} onchange={setTab} label="Live TV sections" items={[
    { value: "guide", label: "Guide" },
    { value: "channels", label: "Channels", count: allChannels.length || null },
    { value: "dvr", label: "DVR", count: schedules.filter((item) => item.status === "SCHEDULED").length || null },
    ...(app.viewer.isAdmin ? [{ value: "sources", label: "Sources" }] : []),
  ]}>
    {#if failure($live) && !$live.data && tab !== "sources"}
      <div class="panel-body"><State kind="error" title="Live TV is off the air" message={failure($live)} action={() => refetch(live)} /></div>
    {:else}
      <BitsTabs.Content value="guide" class="panel-body">
        {#if guideError && !programs.length && allChannels.length}
          <p class="guide-error" role="alert"><WarningCircle size={16} />The guide didn’t load: {guideError}<button class="btn ghost sm" onclick={() => refetch(guide)}>Retry</button></p>
        {/if}
        <Guide {channels} {programs} {now} start={anchor} end={windowEnd} {scheduled} loading={($live.fetching && !$live.data) || ($guide.fetching && !programs.length)} {starting} onplay={play} onrecord={record} onsetup={() => setTab("sources")} />
      </BitsTabs.Content>
      <BitsTabs.Content value="channels" class="panel-body">
        <Channels {channels} {onNow} {now} loading={$live.fetching && !$live.data} {starting} onplay={play} onsetup={() => setTab("sources")} />
      </BitsTabs.Content>
      <BitsTabs.Content value="dvr" class="panel-body">
        <Dvr {live} channels={allChannels} {programs} />
      </BitsTabs.Content>
    {/if}
    {#if app.viewer.isAdmin}
      <BitsTabs.Content value="sources" class="panel-body"><Sources onchanged={() => { refetch(live); refetch(guide); }} /></BitsTabs.Content>
    {/if}
  </Tabs>
</div>

<style>
  .page :global(.panel-body) { margin-top: 24px; animation: fade-up 360ms var(--ease-out); outline: none; }
  .guide-error { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 14px; color: var(--warning); font-size: 0.88rem; }
  .clock { display: inline-flex; align-items: center; gap: 10px; height: 40px; padding: 0 16px 0 14px; border-radius: 999px; background: var(--surface); border: 1px solid var(--line); }
  .clock .label { font-size: 0.72rem; font-weight: 700; letter-spacing: 0.14em; text-transform: uppercase; color: #ff6b78; }
  .clock time { font-weight: 650; font-size: 0.98rem; letter-spacing: -0.01em; }
  .sec { color: var(--text-3); font-weight: 550; }
</style>
