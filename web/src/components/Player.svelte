<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Check from "phosphor-svelte/lib/Check";
  import Info from "phosphor-svelte/lib/Info";
  import SpeakerHigh from "phosphor-svelte/lib/SpeakerHigh";
  import SpeakerLow from "phosphor-svelte/lib/SpeakerLow";
  import SpeakerSlash from "phosphor-svelte/lib/SpeakerSlash";
  import ArrowClockwise from "phosphor-svelte/lib/ArrowClockwise";
  import ArrowCounterClockwise from "phosphor-svelte/lib/ArrowCounterClockwise";
  import ArrowLeft from "phosphor-svelte/lib/ArrowLeft";
  import CornersIn from "phosphor-svelte/lib/CornersIn";
  import CornersOut from "phosphor-svelte/lib/CornersOut";
  import Gear from "phosphor-svelte/lib/Gear";
  import Pause from "phosphor-svelte/lib/Pause";
  import PictureInPicture from "phosphor-svelte/lib/PictureInPicture";
  import Play from "phosphor-svelte/lib/Play";
  import SkipForward from "phosphor-svelte/lib/SkipForward";
  import Stack from "phosphor-svelte/lib/Stack";
  import Subtitles from "phosphor-svelte/lib/Subtitles";
  import Translate from "phosphor-svelte/lib/Translate";
  import WarningCircle from "phosphor-svelte/lib/WarningCircle";
  import X from "phosphor-svelte/lib/X";
  import { Slider } from "bits-ui";
  import { CachePolicy, ControlWatchRoomStore, MarkPlayedStore, MediaFilesStore, ReportProgressStore, StopPlaybackStore, WatchRoomEventsStore, WatchRoomStore } from "$houdini";
  import { reportOnExit } from "$lib/api.js";
  import { languageName, profileName, timecode } from "$lib/format.js";
  import { mediaUrl } from "$lib/tokens.js";
  import { request, writeUserData } from "$lib/graphql.js";
  import { onReconnect } from "$lib/live.js";
  import { closePlayer, nextSibling, player, playWork, restartSession, stopSession } from "$lib/playback.svelte.js";
  import { poll, toast } from "$lib/ui.svelte.js";
  import Artwork from "$components/Artwork.svelte";
  import Menu from "$components/ui/Menu.svelte";
  import MenuItem from "$components/ui/MenuItem.svelte";
  import Tip from "$components/ui/Tip.svelte";

  let { playback } = $props();

  let root = $state();
  let video = $state();
  let hls = null;
  // Only prepared versions offer these: a live transcode carries one track.
  let audioTracks = $state([]);
  let audioTrack = $state(-1);
  let current = $state(0);
  let duration = $state(0);
  let buffered = $state(0);
  let paused = $state(true);
  let waiting = $state(true);
  let volume = $state(readVolume());
  let muted = $state(false);
  let fullscreen = $state(false);
  let chromeVisible = $state(true);
  let error = $state("");
  let flash = $state(null);
  let hover = $state(null);
  let scrubbing = $state(false);
  let next = $state(null);
  let countdown = $state(null);
  let subtitleIndex = $state(null);
  /** Index burned into the picture; text tracks otherwise render natively. */
  let burned = $state(null);
  let subtitleTracks = $state([]);
  let infoOpen = $state(false);
  let stats = $state(null);
  let idleTimer, heartbeat, countdownTimer, restartTimer;
  let finalized = false;
  let suppressRoomEcho = false;
  /** Source position (s) a restarted transcode is coming up at; null otherwise. */
  let restarting = $state(null);

  let edition = $derived(playback.edition);
  let trick = $derived(edition?.trickplay);
  let chapters = $derived(edition?.trickplay?.chapterCues?.length ? edition.trickplay.chapterCues : edition?.chapters ?? []);
  let live = $derived(playback.live || !playback.workId);
  let isGuest = $derived(playback.room && !playback.room.isHost);
  // A transcode's time zero is `startPositionMs` into the source; everything
  // the viewer sees (timeline, chapters, intro, reports, room sync) is in
  // source time, i.e. `offset + video.currentTime`.
  let transcoding = $derived(playback.route === "TRANSCODE");
  let offset = $derived(transcoding ? (playback.startPositionMs ?? 0) / 1000 : 0);
  let inIntro = $derived(edition?.introEndMs != null && current * 1000 >= (edition.introStartMs ?? 0) && current * 1000 < edition.introEndMs - 1500);
  let inCredits = $derived(edition?.creditsStartMs != null && duration > 0 && current * 1000 >= edition.creditsStartMs);
  // Live sessions started elsewhere may only carry the job id, which is
  // the session id on the transcode route.
  let sessionId = $derived(playback.sessionId ?? playback.jobId);
  let delivered = $derived(playback.subtitles ?? []);
  // Tracks without an out-of-band rendition can only be burned in; ASS
  // can be either, since WebVTT drops its styling.
  let burnable = $derived(subtitleTracks.filter((track) => !delivered.some((item) => item.index === track.index) || /^(ass|ssa)$/.test(track.codec)));

  function readVolume() {
    try { const value = Number(localStorage.getItem("noflex.volume")); return Number.isFinite(value) && value > 0 ? Math.min(1, value) : 1; } catch { return 1; }
  }
  $effect(() => { try { localStorage.setItem("noflex.volume", String(volume)); } catch { /* storage unavailable */ } });
  $effect(() => { if (video) { video.volume = volume; video.muted = muted; } });

  const mediaTime = () => restarting ?? offset + (Number.isFinite(video?.currentTime) ? video.currentTime : 0);
  const positionMs = () => Math.max(0, Math.round(mediaTime() * 1000));

  // The growing HLS duration would make the timeline creep; the runtime is the real length.
  function timeline() {
    const own = Number.isFinite(video?.duration) ? video.duration : 0;
    if (!transcoding) return own;
    const runtime = (edition?.runtimeMs ?? playback.work?.runtimeMs ?? 0) / 1000;
    return runtime || offset + own;
  }

  async function report(state) {
    if (!playback.workId || live || restarting != null) return;
    const workId = playback.workId;
    try {
      const data = await request(new ReportProgressStore(), { id: workId, position: positionMs(), state });
      writeUserData(workId, data.reportProgress);
    } catch { /* best effort */ }
  }

  // ---- stream lifecycle -------------------------------------------------
  $effect(() => {
    const url = playback.url;
    if (!video || !url) return;
    finalized = false;
    error = "";
    waiting = true;
    let disposed = false;
    // Prefer hls.js wherever MSE exists: it retries playlists the transcoder
    // hasn't finished writing, which native HLS (Safari, newer Chrome) won't.
    const native = video.canPlayType("application/vnd.apple.mpegurl");
    if (playback.route !== "DIRECT") {
      import("hls.js").then(({ default: Hls }) => {
        if (disposed) return;
        if (!Hls.isSupported()) {
          if (native) { video.src = url; video.play().catch(() => {}); }
          else error = "Adaptive streaming isn’t supported in this browser.";
          return;
        }
        const retry = { maxNumRetry: 8, retryDelayMs: 500, maxRetryDelayMs: 3000, shouldRetry: (config, count) => count < config.maxNumRetry };
        const policy = { default: { maxTimeToFirstByteMs: 15_000, maxLoadTimeMs: 30_000, timeoutRetry: retry, errorRetry: retry } };
        hls = new Hls({ enableWorker: false, backBufferLength: 60, maxBufferLength: 30, manifestLoadPolicy: policy, playlistLoadPolicy: policy });
        hls.loadSource(url);
        hls.attachMedia(video);
        hls.on(Hls.Events.MANIFEST_PARSED, () => video.play().catch(() => {}));
        hls.on(Hls.Events.AUDIO_TRACKS_UPDATED, (_event, data) => { audioTracks = data.audioTracks.map((track) => ({ id: track.id, label: languageName(track.lang) || track.name })); });
        hls.on(Hls.Events.AUDIO_TRACK_SWITCHED, (_event, data) => { audioTrack = data.id; });
        hls.on(Hls.Events.ERROR, (_event, data) => {
          if (!data.fatal) return;
          if (data.type === Hls.ErrorTypes.NETWORK_ERROR) hls.startLoad();
          else if (data.type === Hls.ErrorTypes.MEDIA_ERROR) hls.recoverMediaError();
          else { error = "This stream couldn’t be played."; hls.destroy(); hls = null; }
        });
      }).catch(() => { if (!disposed) error = "Adaptive streaming isn’t available."; });
    } else {
      video.src = url;
      video.play().catch(() => {});
    }
    return () => {
      disposed = true;
      hls?.destroy();
      hls = null;
      audioTracks = [];
      audioTrack = -1;
      video?.removeAttribute("src");
      video?.load();
    };
  });

  // Burn-in choices come from the media file: sidecars and muxed text tracks alike.
  $effect(() => {
    const id = playback.mediaFileId;
    if (!id || !edition) return;
    request(new MediaFilesStore(), { edition: edition.id }, { policy: CachePolicy.CacheOrNetwork }).then((data) => {
      subtitleTracks = (data.mediaFiles.find((file) => file.id === id)?.subtitleTracks ?? []).map((track, index) => ({ ...track, index }));
    }).catch(() => {});
  });

  $effect(() => {
    if (!playback.work || live) return;
    let stale = false;
    nextSibling(playback.work).then((item) => { if (!stale) next = item; });
    return () => { stale = true; };
  });

  async function finish() {
    if (finalized) return;
    finalized = true;
    clearInterval(heartbeat);
    await report("STOPPED");
    await stopSession(sessionId);
  }

  async function close() {
    if (document.fullscreenElement) await document.exitFullscreen().catch(() => {});
    await finish();
    closePlayer();
  }

  async function playNext() {
    const target = next;
    cancelCountdown();
    if (!target) return close();
    await finish();
    await playWork(target, { fromStart: true });
  }

  function pageExit(event) {
    if (event.persisted || finalized) return;
    if (playback.workId && !live) reportOnExit(new ReportProgressStore(), { id: playback.workId, position: positionMs(), state: "STOPPED" });
    if (sessionId) reportOnExit(new StopPlaybackStore(), { session: sessionId });
  }

  // ---- media events -------------------------------------------------------
  function onLoaded() {
    duration = timeline();
    // A transcode already starts at the resume point; direct play seeks to it.
    const resume = (playback.resumeMs ?? 0) / 1000 - offset;
    if (resume > 1 && video.currentTime < 1) video.currentTime = resume;
  }
  function onTime() {
    if (restarting != null) return;
    if (!scrubbing) current = mediaTime();
    if (video.buffered.length) buffered = offset + video.buffered.end(video.buffered.length - 1);
    if (inCredits && next && countdown == null && !countdownTimer && !video.paused) startCountdown();
  }
  function onPlaying() {
    restarting = null;
    waiting = false; paused = false;
    clearInterval(heartbeat);
    report("PLAYING");
    heartbeat = setInterval(() => report("PLAYING"), 15_000);
    roomSend("PLAY");
  }
  function onPause() {
    if (restarting != null) return;
    paused = true;
    clearInterval(heartbeat);
    if (!finalized && !video.ended) report("PAUSED");
    showChrome();
    if (!video.ended) roomSend("PAUSE");
  }
  function onEnded() {
    // The server infers completion from progress, but only with a known runtime.
    if (playback.workId && !live) {
      const workId = playback.workId;
      request(new MarkPlayedStore(), { id: workId }).then((data) => writeUserData(workId, data.markPlayed)).catch(() => {});
    }
    next ? playNext() : close();
  }

  // ---- controls -----------------------------------------------------------
  function toggle() {
    if (!video || isGuest) return;
    if (video.paused) video.play().catch(() => {}); else video.pause();
    pulse(video.paused ? Pause : Play);
  }
  function seekTo(seconds) {
    if (!video || isGuest || live) return;
    seekMedia(seconds);
  }
  // Within what the transcoder has produced (plus a little it will reach
  // soon) a seek is native; before the offset or far beyond, a new session
  // starting at the target beats waiting for ffmpeg to get there.
  const TRANSCODE_REACH = 30;
  function seekMedia(seconds) {
    const target = Math.max(0, Math.min(duration || seconds, seconds));
    current = target;
    const local = target - offset;
    const produced = video.seekable.length ? video.seekable.end(video.seekable.length - 1) : Number.isFinite(video.duration) ? video.duration : 0;
    if (!transcoding || (local >= 0 && local <= produced + TRANSCODE_REACH)) {
      clearTimeout(restartTimer);
      if (restarting == null) video.currentTime = local;
      else restartAt(target);
      return;
    }
    restartAt(target);
  }
  // Debounced so a run of skips or a drag opens one session, not several.
  function restartAt(target) {
    restarting = target;
    waiting = true;
    clearTimeout(restartTimer);
    restartTimer = setTimeout(async () => {
      clearInterval(heartbeat);
      if (!(await restartSession(burned, Math.round(target * 1000)))) { restarting = null; waiting = false; }
    }, 350);
  }
  function skip(seconds) { seekTo(mediaTime() + seconds); pulse(seconds > 0 ? ArrowClockwise : ArrowCounterClockwise); }
  function pulse(icon) { flash = { icon, key: Date.now() }; }

  async function toggleFullscreen() {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await root.requestFullscreen();
    } catch { toast("Fullscreen isn’t available here", "error"); }
  }
  async function togglePip() {
    try {
      if (document.pictureInPictureElement) await document.exitPictureInPicture();
      else await video.requestPictureInPicture();
    } catch { toast("Picture-in-picture isn’t available here", "error"); }
  }

  function showChrome() {
    chromeVisible = true;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => { if (!video?.paused && !scrubbing) chromeVisible = false; }, 2600);
  }

  function startCountdown() {
    countdown = 10;
    countdownTimer = setInterval(() => {
      countdown -= 1;
      if (countdown <= 0) playNext();
    }, 1000);
  }
  function cancelCountdown() { clearInterval(countdownTimer); countdownTimer = null; countdown = null; }

  async function chooseSubtitle(index, burn = false) {
    subtitleIndex = index;
    // Native tracks switch in place; entering or leaving burn-in needs a
    // new session.
    if (!burn && burned == null) return;
    burned = burn ? index : null;
    await finish();
    await restartSession(burned, positionMs());
  }

  $effect(() => {
    const showing = burned == null ? subtitleIndex : null;
    if (!video || !delivered) return;
    for (const element of video.querySelectorAll("track")) element.track.mode = Number(element.dataset.index) === showing ? "showing" : "disabled";
  });

  const trackLabel = (track) => track.title || track.language?.toUpperCase() || `Track ${track.index + 1}`;

  // ---- playback info ------------------------------------------------------
  const PLAN = { DIRECT_PLAY: "Direct play", REMUX: "Direct stream (remux)", TRANSCODE_AUDIO: "Transcoding audio", TRANSCODE_VIDEO: "Transcoding video", TRANSCODE: "Transcoding" };
  const CODEC = { H264: "H.264", H265: "HEVC", AV1: "AV1", VP9: "VP9", VP8: "VP8", MPEG2: "MPEG-2", AC3: "AC-3", EAC3: "E-AC-3", MP3: "MP3" };
  // Server levels are ffprobe's: H.264 ×10, HEVC ×30, AV1 seq_level_idx.
  const levelText = (codec, level) => level == null ? null : codec === "H264" ? `L${level / 10}` : codec === "H265" ? `L${Math.round(level / 3) / 10}` : `L${level}`;
  function videoText(s) {
    if (!s?.videoCodec) return "none";
    return [CODEC[s.videoCodec] ?? s.videoCodec, profileName(s.videoProfile), levelText(s.videoCodec, s.videoLevel), s.width && `${s.width}×${s.height}`, s.bitDepth && `${s.bitDepth}-bit`, s.videoRange && s.videoRange !== "SDR" && s.videoRange.replace(/_/g, " ")].filter(Boolean).join(" · ");
  }
  const audioText = (s) => s?.audioCodec ? [s.audioCodec.toUpperCase(), s.audioChannels && `${s.audioChannels} ch`].filter(Boolean).join(" · ") : "none";
  const rate = (bps) => bps ? `${(bps / 1e6).toFixed(1)} Mb/s` : null;
  const flow = (from, to) => from === to ? from : `${from} → ${to}`;

  $effect(() => {
    if (!infoOpen) return;
    const tick = () => {
      if (!video) return;
      const quality = video.getVideoPlaybackQuality?.();
      const ranges = video.buffered;
      let ahead = 0;
      for (let i = 0; i < ranges.length; i++) if (ranges.start(i) <= video.currentTime && video.currentTime <= ranges.end(i)) ahead = ranges.end(i) - video.currentTime;
      stats = {
        size: video.videoWidth ? `${video.videoWidth}×${video.videoHeight}` : "—",
        dropped: quality ? `${quality.droppedVideoFrames} / ${quality.totalVideoFrames}` : "—",
        buffer: `${ahead.toFixed(1)} s`,
        bandwidth: rate(hls?.bandwidthEstimate),
        level: rate(hls?.levels?.[hls.currentLevel]?.bitrate),
      };
    };
    tick();
    return poll(tick, 1000);
  });

  function key(event) {
    if (event.target instanceof HTMLInputElement) return;
    const handled = {
      " ": toggle, k: toggle,
      ArrowLeft: () => skip(-10), j: () => skip(-10),
      ArrowRight: () => skip(10), l: () => skip(10),
      ArrowUp: () => { volume = Math.min(1, volume + 0.05); muted = false; pulse(SpeakerHigh); },
      ArrowDown: () => { volume = Math.max(0, volume - 0.05); pulse(volume ? SpeakerLow : SpeakerSlash); },
      m: () => { muted = !muted; pulse(muted ? SpeakerSlash : SpeakerHigh); },
      f: toggleFullscreen,
      p: togglePip,
      n: () => next && playNext(),
      s: () => inIntro && seekTo(edition.introEndMs / 1000),
      i: () => (infoOpen = !infoOpen),
      Escape: () => { if (!document.fullscreenElement) close(); },
      Home: () => seekTo(0),
    }[event.key];
    if (!handled) return;
    event.preventDefault();
    event.stopPropagation();
    handled();
    showChrome();
  }

  // ---- scrubber + trickplay ---------------------------------------------
  let bar = $state();
  function timeAt(event) {
    const rect = bar.getBoundingClientRect();
    return Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)) * duration;
  }
  function scrubMove(event) {
    if (!duration) return;
    const time = timeAt(event);
    hover = { time, x: ((time / duration) * 100) };
    if (scrubbing) current = time;
  }
  function scrubDown(event) {
    if (!duration || isGuest) return;
    scrubbing = true;
    bar.setPointerCapture(event.pointerId);
    current = timeAt(event);
  }
  function scrubUp(event) {
    if (!scrubbing) return;
    scrubbing = false;
    seekTo(timeAt(event));
    showChrome();
  }
  function scrubKey(event) {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); event.stopPropagation(); skip(event.key === "ArrowLeft" ? -5 : 5); }
  }

  function sprite(time) {
    if (!trick?.sheets?.length) return null;
    const perSheet = trick.tileColumns * trick.tileRows;
    const index = Math.floor((time * 1000) / trick.intervalMs);
    const sheet = trick.sheets.find((item) => item.index === Math.floor(index / perSheet));
    if (!sheet) return null;
    const within = index % perSheet;
    return { url: mediaUrl(sheet.url), x: (within % trick.tileColumns) * trick.thumbWidth, y: Math.floor(within / trick.tileColumns) * trick.thumbHeight, w: trick.thumbWidth, h: trick.thumbHeight };
  }
  let preview = $derived(hover ? sprite(hover.time) : null);
  let hoverChapter = $derived(hover ? [...chapters].reverse().find((chapter) => chapter.positionMs <= hover.time * 1000) : null);

  // ---- watch-room sync ----------------------------------------------------
  // The host's player drives the room; guests follow its events.
  function roomSend(action) {
    if (!playback.room?.isHost || suppressRoomEcho) return;
    request(new ControlWatchRoomStore(), { id: playback.room.id, action, position: positionMs() }).catch(() => {});
  }

  // Server clock minus ours, from event timestamps, so drift doesn't hinge
  // on the two clocks agreeing.
  let clockSkew = 0;
  async function follow(room) {
    if (!room || !video) return;
    if (room.state === "ENDED" || room.state === "STOPPED") { toast("The host ended the watch room"); close(); return; }
    const elapsed = room.state === "PLAYING" ? (Date.now() + clockSkew - Date.parse(room.updatedAt)) / 1000 : 0;
    const target = room.positionMs / 1000 + Math.max(0, elapsed);
    suppressRoomEcho = true;
    if (Math.abs(mediaTime() - target) > 2) seekMedia(target);
    if (room.state === "PLAYING" && video.paused) await video.play().catch(() => {});
    if (room.state !== "PLAYING" && !video.paused) video.pause();
    suppressRoomEcho = false;
  }

  const roomEvents = new WatchRoomEventsStore();
  $effect(() => {
    if (!playback.room || playback.room.isHost) return;
    const id = playback.room.id;
    let sequence = -1;
    const apply = (room) => {
      if (!room || room.sequence < sequence) return;
      sequence = room.sequence;
      follow(room);
    };
    // Events missed while the socket was down are gone: resync on
    // reconnect, and now and then in case one slipped through.
    const resync = () => request(new WatchRoomStore(), { id }).then((data) => apply(data.watchRoom), () => {});
    const unsubscribe = roomEvents.subscribe((result) => {
      const event = result.data?.watchRoomEvents;
      if (!event) return;
      clockSkew = Date.parse(event.serverTimestamp) - Date.now();
      apply(event.room);
    });
    roomEvents.listen({ room: id });
    resync();
    const stopPolling = poll(resync, 15_000);
    const stopReconnect = onReconnect(resync);
    return () => {
      unsubscribe();
      roomEvents.unlisten();
      stopPolling();
      stopReconnect();
    };
  });

  $effect(() => {
    const onFullscreen = () => (fullscreen = !!document.fullscreenElement);
    document.addEventListener("fullscreenchange", onFullscreen);
    showChrome();
    root?.focus();
    return () => {
      document.removeEventListener("fullscreenchange", onFullscreen);
      clearTimeout(idleTimer); clearInterval(heartbeat); clearInterval(countdownTimer); clearTimeout(restartTimer);
      finish();
    };
  });
</script>

<svelte:window onpagehide={pageExit} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="player"
  class:idle={!chromeVisible}
  bind:this={root}
  role="dialog"
  aria-modal="true"
  aria-label="Playing {playback.title}"
  tabindex="0"
  onkeydown={key}
  onpointermove={showChrome}
>
  {#if playback.work}<div class="poster-bg" aria-hidden="true"><Artwork work={playback.work} kind="backdrop" width={1280} rounded={false} /></div>{/if}

  <!-- svelte-ignore a11y_media_has_caption -->
  <video
    bind:this={video}
    playsinline
    preload="auto"
    onclick={toggle}
    ondblclick={toggleFullscreen}
    onloadedmetadata={onLoaded}
    ondurationchange={() => (duration = timeline() || duration)}
    ontimeupdate={onTime}
    onplaying={onPlaying}
    onpause={onPause}
    onwaiting={() => (waiting = true)}
    oncanplay={() => (waiting = false)}
    onseeked={() => { if (restarting == null) roomSend(video.paused ? "PAUSE" : "SEEK"); }}
    onended={onEnded}
    onerror={() => { if (video?.error) error = "Your browser couldn’t decode this stream."; }}
  >{#each delivered as track (track.url)}<track kind="subtitles" src={track.url} srclang={track.language ?? ""} label={trackLabel(track)} data-index={track.index} />{/each}</video>

  {#if waiting && !error}<div class="center"><span class="loader"></span></div>{/if}
  {#if flash}{#key flash.key}<div class="center flash" aria-hidden="true"><span><flash.icon size={34} weight={flash.icon === Play || flash.icon === Pause ? "fill" : undefined} /></span></div>{/key}{/if}

  {#if error}
    <div class="center error" role="alert">
      <WarningCircle size={30} />
      <p>{error}</p>
      <div class="row"><button class="btn light" onclick={() => restartSession(burned, positionMs())}>Try again</button><button class="btn glass" onclick={close}>Close</button></div>
    </div>
  {/if}

  <header class="top chrome">
    <button class="icon-btn glass" aria-label="Close player" onclick={close}><ArrowLeft size={20} /></button>
    <div class="title">
      <strong>{playback.title}</strong>
      {#if playback.subtitle || playback.room}<small>{#if playback.room}<span class="dot live"></span>Watch room · {playback.room.isHost ? "you’re hosting" : "synced to host"}{#if playback.subtitle} · {/if}{/if}{playback.subtitle ?? ""}</small>{/if}
    </div>
    <button class="route" aria-pressed={infoOpen} aria-label="Playback info" onclick={() => (infoOpen = !infoOpen)}>{playback.route === "TRANSCODE" ? "Adaptive" : playback.route === "OPTIMIZED" ? "Optimized" : "Direct"}</button>
  </header>

  {#if infoOpen}
    {@const decision = playback.decision}
    <aside class="info" aria-label="Playback info">
      <div class="info-head"><strong>Playback info</strong><button class="ctl small" aria-label="Close playback info" onclick={() => (infoOpen = false)}><X size={16} /></button></div>
      <dl>
        <dt>Mode</dt><dd>{decision ? PLAN[decision.plan] ?? decision.plan : live ? "Live transcode" : "—"}</dd>
        {#if decision?.reasons?.length}<dt>Why</dt><dd><ul>{#each decision.reasons as reason (reason.code)}<li>{reason.description}</li>{/each}</ul></dd>{/if}
        {#if decision}
          <dt>Container</dt><dd>{flow(decision.source.container ?? "?", decision.target.container ?? "?")}</dd>
          <dt>Video</dt><dd>{flow(videoText(decision.source), videoText(decision.target))}</dd>
          <dt>Audio</dt><dd>{flow(audioText(decision.source), audioText(decision.target))}</dd>
          {#if decision.encoder}<dt>Encoder</dt><dd>{decision.encoder}{decision.hardware ? " (hardware)" : ""}{decision.audioEncoder ? ` · audio ${decision.audioEncoder}` : ""}{decision.tonemap ? " · HDR → SDR tonemap" : ""}</dd>{/if}
          {#if rate(decision.target.bitrateBps)}<dt>Bitrate</dt><dd>{rate(decision.target.bitrateBps)}</dd>{/if}
        {/if}
        {#if stats}
          <dt>Decoded</dt><dd class="tabular">{stats.size}</dd>
          <dt>Dropped</dt><dd class="tabular">{stats.dropped}</dd>
          <dt>Buffer</dt><dd class="tabular">{stats.buffer}</dd>
          {#if stats.bandwidth}<dt>Bandwidth</dt><dd class="tabular">{stats.bandwidth}{stats.level ? ` (stream ${stats.level})` : ""}</dd>{/if}
        {/if}
      </dl>
    </aside>
  {/if}

  {#if inIntro && !isGuest}
    <button class="pill skip-intro" onclick={() => seekTo(edition.introEndMs / 1000)}><SkipForward size={16} />Skip intro</button>
  {/if}

  {#if countdown != null && next}
    <div class="up-next" role="status">
      <div class="thumb"><Artwork work={next} kind="thumb" width={220} /><svg viewBox="0 0 36 36" aria-hidden="true"><circle cx="18" cy="18" r="16" style:stroke-dashoffset={100 - countdown * 10} /></svg></div>
      <div class="text"><small>Up next in {countdown}s</small><strong>{next.title}</strong></div>
      <div class="row"><button class="btn light sm" onclick={playNext}><Play size={14} weight="fill" />Play now</button><button class="btn glass sm" onclick={cancelCountdown}>Cancel</button></div>
    </div>
  {/if}

  <footer class="bottom chrome">
    {#if !live}
      <div class="scrub" bind:this={bar} role="slider" tabindex="0" aria-label="Seek" aria-valuemin="0" aria-valuemax={Math.round(duration)} aria-valuenow={Math.round(current)} aria-valuetext={timecode(current * 1000)}
        onpointermove={scrubMove} onpointerleave={() => { if (!scrubbing) hover = null; }} onpointerdown={scrubDown} onpointerup={scrubUp} onkeydown={scrubKey} class:active={scrubbing || hover}>
        <div class="rail">
          <span class="buffer" style:width="{duration ? (buffered / duration) * 100 : 0}%"></span>
          <span class="played" style:width="{duration ? (current / duration) * 100 : 0}%"></span>
          {#each chapters as chapter}{#if duration && chapter.positionMs > 0}<span class="tick" style:left="{(chapter.positionMs / 1000 / duration) * 100}%"></span>{/if}{/each}
          {#if edition?.introEndMs != null && duration}<span class="intro-zone" style:left="{((edition.introStartMs ?? 0) / 1000 / duration) * 100}%" style:width="{((edition.introEndMs - (edition.introStartMs ?? 0)) / 1000 / duration) * 100}%"></span>{/if}
          <span class="knob" style:left="{duration ? (current / duration) * 100 : 0}%"></span>
        </div>
        {#if hover}
          <div class="preview" style:left="clamp(90px, {hover.x}%, calc(100% - 90px))">
            {#if preview}<span class="sprite" style:width="{preview.w}px" style:height="{preview.h}px" style:background-image="url({preview.url})" style:background-position="-{preview.x}px -{preview.y}px"></span>{/if}
            {#if hoverChapter?.title}<small>{hoverChapter.title}</small>{/if}
            <strong class="tabular">{timecode(hover.time * 1000)}</strong>
          </div>
        {/if}
      </div>
    {/if}

    <div class="controls">
      {#if !isGuest}
        <Tip portal={root} text={paused ? "Play" : "Pause"} hint="k">{#snippet children(props)}<button {...props} class="ctl big" aria-label={paused ? "Play" : "Pause"} onclick={toggle}>{#if paused}<Play size={24} weight="fill" />{:else}<Pause size={24} weight="fill" />{/if}</button>{/snippet}</Tip>
        {#if !live}
          <Tip portal={root} text="Back 10s" hint="j">{#snippet children(props)}<button {...props} class="ctl" aria-label="Back 10 seconds" onclick={() => skip(-10)}><ArrowCounterClockwise size={22} /></button>{/snippet}</Tip>
          <Tip portal={root} text="Forward 10s" hint="l">{#snippet children(props)}<button {...props} class="ctl" aria-label="Forward 10 seconds" onclick={() => skip(10)}><ArrowClockwise size={22} /></button>{/snippet}</Tip>
        {/if}
        {#if next}<Tip portal={root} text="Next: {next.title}" hint="n">{#snippet children(props)}<button {...props} class="ctl" aria-label="Next" onclick={playNext}><SkipForward size={20} /></button>{/snippet}</Tip>{/if}
      {/if}
      <div class="volume">
        <button class="ctl" aria-label={muted ? "Unmute" : "Mute"} onclick={() => (muted = !muted)}>{#if muted || volume === 0}<SpeakerSlash size={21} />{:else if volume < 0.5}<SpeakerLow size={21} />{:else}<SpeakerHigh size={21} />{/if}</button>
        <Slider.Root type="single" bind:value={() => (muted ? 0 : volume), (next) => { volume = next; muted = next === 0; }} min={0} max={1} step={0.01} class="vol-slider" aria-label="Volume">
          <span class="vol-track"><Slider.Range class="vol-range" /></span>
          <Slider.Thumb index={0} class="vol-thumb" />
        </Slider.Root>
      </div>
      <span class="time tabular">{#if live}<span class="live-badge"><span class="dot live"></span>LIVE</span>{:else}{timecode(current * 1000)} <span>/ {timecode(duration * 1000)}</span>{/if}</span>
      <span class="spacer"></span>
      {#if chapters.length}
        <Menu portal={root} side="top" width={260}>
          {#snippet trigger(props)}<button {...props} class="ctl" aria-label="Chapters"><Stack size={20} /></button>{/snippet}
          <div class="nf-menu-label">Chapters</div>
          {#each chapters as chapter, index}<MenuItem onSelect={() => seekTo(chapter.positionMs / 1000)} hint={timecode(chapter.positionMs)}>{chapter.title || `Chapter ${index + 1}`}</MenuItem>{/each}
        </Menu>
      {/if}
      {#if audioTracks.length > 1}
        <Menu portal={root} side="top" width={220}>
          {#snippet trigger(props)}<button {...props} class="ctl" aria-label="Audio language"><Translate size={20} /></button>{/snippet}
          <div class="nf-menu-label">Audio</div>
          {#each audioTracks as track (track.id)}<MenuItem icon={audioTrack === track.id ? Check : null} onSelect={() => { if (hls) hls.audioTrack = track.id; }}>{track.label}</MenuItem>{/each}
        </Menu>
      {/if}
      {#if (delivered.length || burnable.length) && !isGuest}
        <Menu portal={root} side="top" width={260}>
          {#snippet trigger(props)}<button {...props} class="ctl" class:on={subtitleIndex != null} aria-label="Subtitles"><Subtitles size={20} /></button>{/snippet}
          <div class="nf-menu-label">Subtitles</div>
          <MenuItem icon={subtitleIndex == null ? Check : null} onSelect={() => chooseSubtitle(null)}>Off</MenuItem>
          {#each delivered as track (track.index)}<MenuItem icon={subtitleIndex === track.index && burned == null ? Check : null} onSelect={() => chooseSubtitle(track.index)}>{trackLabel(track)}</MenuItem>{/each}
          {#each burnable as track (track.index)}<MenuItem icon={burned === track.index ? Check : null} hint={delivered.some((item) => item.index === track.index) ? "styled" : "burn in"} onSelect={() => chooseSubtitle(track.index, true)}>{trackLabel(track)}</MenuItem>{/each}
        </Menu>
      {/if}
      <Menu portal={root} side="top" width={220}>
        {#snippet trigger(props)}<button {...props} class="ctl" aria-label="Settings"><Gear size={20} /></button>{/snippet}
        <MenuItem icon={infoOpen ? Check : Info} hint="i" onSelect={() => (infoOpen = !infoOpen)}>Playback info</MenuItem>
      </Menu>
      {#if document.pictureInPictureEnabled}<Tip portal={root} text="Picture in picture" hint="p">{#snippet children(props)}<button {...props} class="ctl" aria-label="Picture in picture" onclick={togglePip}><PictureInPicture size={20} /></button>{/snippet}</Tip>{/if}
      <Tip portal={root} text={fullscreen ? "Exit fullscreen" : "Fullscreen"} hint="f">{#snippet children(props)}<button {...props} class="ctl" aria-label="Fullscreen" onclick={toggleFullscreen}>{#if fullscreen}<CornersIn size={20} />{:else}<CornersOut size={20} />{/if}</button>{/snippet}</Tip>
    </div>
  </footer>
</div>

<style>
  .player { position: fixed; inset: 0; z-index: 70; background: #000; color: #fff; outline: none; animation: player-in 420ms var(--ease-out); overflow: hidden; }
  @keyframes player-in { from { opacity: 0; transform: scale(1.02); } }
  .player.idle { cursor: none; }
  .poster-bg { position: absolute; inset: 0; opacity: 0.25; filter: blur(30px) saturate(1.2); transform: scale(1.1); }
  .poster-bg :global(.art) { position: absolute; inset: 0; aspect-ratio: auto; height: 100%; }
  video { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: transparent; }
  .chrome { position: absolute; left: 0; right: 0; z-index: 2; transition: opacity 400ms var(--ease-out), transform 400ms var(--ease-out); }
  .idle .chrome { opacity: 0; pointer-events: none; }
  .idle .top { transform: translateY(-8px); }
  .idle .bottom { transform: translateY(8px); }
  .top { top: 0; display: flex; align-items: center; gap: 16px; padding: 18px clamp(16px, 3vw, 36px) 60px; background: linear-gradient(rgb(0 0 0 / 0.7), transparent); }
  .title { flex: 1; display: grid; min-width: 0; }
  .title strong { font-size: 1.1rem; font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .title small { display: flex; align-items: center; gap: 8px; color: rgb(255 255 255 / 0.65); font-size: 0.84rem; }
  .route:hover, .route[aria-pressed="true"] { color: #fff; border-color: rgb(255 255 255 / 0.45); }
  .info { position: absolute; z-index: 3; top: 76px; left: clamp(16px, 3vw, 36px); width: min(420px, calc(100vw - 32px)); max-height: calc(100% - 220px); overflow: auto; padding: 12px 16px 14px; border-radius: 14px; background: rgb(14 15 19 / 0.82); backdrop-filter: blur(18px); border: 1px solid rgb(255 255 255 / 0.14); box-shadow: var(--shadow-3); font-size: 0.82rem; animation: pop-in 160ms var(--ease-out); }
  .info-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  .ctl.small { width: 30px; height: 30px; }
  .info dl { display: grid; grid-template-columns: max-content 1fr; gap: 5px 14px; margin: 0; }
  .info dt { color: rgb(255 255 255 / 0.55); }
  .info dd { margin: 0; overflow-wrap: anywhere; }
  .info ul { margin: 0; padding-left: 1.1em; }
  .route { font-size: 0.72rem; font-weight: 650; letter-spacing: 0.08em; text-transform: uppercase; color: rgb(255 255 255 / 0.5); padding: 4px 10px; border-radius: 99px; border: 1px solid rgb(255 255 255 / 0.18); }
  .bottom { bottom: 0; padding: 70px clamp(16px, 3vw, 36px) max(18px, env(safe-area-inset-bottom)); background: linear-gradient(transparent, rgb(0 0 0 / 0.82)); }
  .center { position: absolute; inset: 0; z-index: 1; display: grid; place-items: center; pointer-events: none; }
  .loader { width: 54px; height: 54px; border-radius: 50%; border: 3px solid rgb(255 255 255 / 0.15); border-top-color: #fff; animation: spin 0.9s linear infinite; }
  .flash span { display: grid; place-items: center; width: 84px; height: 84px; border-radius: 50%; background: rgb(0 0 0 / 0.45); backdrop-filter: blur(8px); animation: flash 600ms var(--ease-out) forwards; }
  @keyframes flash { 0% { opacity: 0; transform: scale(0.7); } 25% { opacity: 1; transform: scale(1); } 100% { opacity: 0; transform: scale(1.25); } }
  .error { pointer-events: auto; align-content: center; gap: 14px; text-align: center; background: rgb(0 0 0 / 0.6); }
  .error .row { justify-content: center; }

  .scrub { position: relative; height: 22px; display: flex; align-items: center; cursor: pointer; touch-action: none; margin-bottom: 6px; outline: none; }
  .rail { position: relative; width: 100%; height: 4px; border-radius: 9px; background: rgb(255 255 255 / 0.22); transition: height 200ms var(--ease-out); }
  .scrub.active .rail, .scrub:focus-visible .rail { height: 7px; }
  .buffer, .played, .intro-zone { position: absolute; top: 0; bottom: 0; left: 0; border-radius: inherit; }
  .buffer { background: rgb(255 255 255 / 0.3); }
  .played { background: var(--accent); }
  .intro-zone { background: rgb(255 255 255 / 0.18); border-radius: 0; }
  .tick { position: absolute; top: 0; bottom: 0; width: 3px; margin-left: -1.5px; background: rgb(0 0 0 / 0.65); }
  .knob { position: absolute; top: 50%; width: 15px; height: 15px; margin: -7.5px 0 0 -7.5px; border-radius: 50%; background: #fff; box-shadow: 0 2px 8px rgb(0 0 0 / 0.5); transform: scale(0); transition: transform 200ms var(--ease-spring); }
  .scrub.active .knob, .scrub:focus-visible .knob { transform: scale(1); }
  .preview { position: absolute; bottom: 30px; translate: -50% 0; display: grid; justify-items: center; gap: 6px; pointer-events: none; animation: pop-in 140ms var(--ease-out); }
  .sprite { display: block; border-radius: 10px; background-repeat: no-repeat; box-shadow: 0 0 0 2px rgb(255 255 255 / 0.9), var(--shadow-3); }
  .preview small { font-size: 0.78rem; color: rgb(255 255 255 / 0.8); max-width: 200px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .preview strong { font-size: 0.86rem; padding: 3px 9px; border-radius: 7px; background: rgb(0 0 0 / 0.7); }

  .controls { display: flex; align-items: center; gap: 4px; }
  .ctl { display: grid; place-items: center; width: 44px; height: 44px; border-radius: 50%; color: rgb(255 255 255 / 0.88); transition: background var(--t), color var(--t), transform var(--t-fast) var(--ease-out); }
  .ctl:hover { background: rgb(255 255 255 / 0.12); color: #fff; }
  .ctl:active { transform: scale(0.9); }
  .ctl.big { width: 50px; height: 50px; }
  .ctl.on { color: var(--accent); }
  .volume { display: flex; align-items: center; }
  .volume :global(.vol-slider) { position: relative; display: flex; align-items: center; width: 0; height: 30px; opacity: 0; overflow: hidden; transition: width 300ms var(--ease-out), opacity 300ms, margin 300ms; touch-action: none; }
  .volume:hover :global(.vol-slider), .volume:focus-within :global(.vol-slider) { width: 96px; opacity: 1; margin: 0 10px 0 2px; overflow: visible; }
  .vol-track { position: relative; width: 100%; height: 4px; border-radius: 9px; background: rgb(255 255 255 / 0.25); }
  .volume :global(.vol-range) { position: absolute; height: 100%; border-radius: inherit; background: #fff; }
  .volume :global(.vol-thumb) { display: block; width: 13px; height: 13px; border-radius: 50%; background: #fff; box-shadow: var(--shadow-1); outline: none; }
  .time { margin-left: 8px; font-size: 0.9rem; font-weight: 550; }
  .time span { color: rgb(255 255 255 / 0.55); }
  .live-badge { display: inline-flex; align-items: center; gap: 8px; color: #fff !important; font-weight: 700; letter-spacing: 0.08em; font-size: 0.78rem; }

  .pill { position: absolute; z-index: 3; right: clamp(16px, 3vw, 36px); bottom: 130px; display: inline-flex; align-items: center; gap: 9px; height: 46px; padding: 0 20px; border-radius: 12px; background: rgb(20 20 24 / 0.7); backdrop-filter: blur(16px); border: 1px solid rgb(255 255 255 / 0.25); font-weight: 650; animation: slide-in 420ms var(--ease-spring); transition: background var(--t); }
  .pill:hover { background: #fff; color: #000; }
  @keyframes slide-in { from { opacity: 0; transform: translateX(24px); } }
  .up-next { position: absolute; z-index: 3; right: clamp(16px, 3vw, 36px); bottom: 130px; display: grid; grid-template-columns: 150px 1fr; gap: 10px 16px; align-items: center; width: min(420px, calc(100vw - 32px)); padding: 14px; border-radius: 18px; background: rgb(18 19 24 / 0.82); backdrop-filter: blur(20px); border: 1px solid rgb(255 255 255 / 0.14); box-shadow: var(--shadow-3); animation: slide-in 460ms var(--ease-spring); }
  .up-next .thumb { position: relative; }
  .up-next svg { position: absolute; right: 8px; bottom: 8px; width: 30px; height: 30px; transform: rotate(-90deg); }
  .up-next circle { fill: rgb(0 0 0 / 0.5); stroke: #fff; stroke-width: 3; stroke-dasharray: 100; transition: stroke-dashoffset 1s linear; }
  .up-next .text { display: grid; gap: 2px; }
  .up-next small { color: rgb(255 255 255 / 0.6); font-size: 0.8rem; }
  .up-next .row { grid-column: 1 / -1; }
  @media (max-width: 600px) { .volume, .time span { display: none; } .up-next { grid-template-columns: 100px 1fr; } }
</style>
