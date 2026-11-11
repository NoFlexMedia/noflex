// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex's Cast receiver plays what a sender loads, draws our player and reports
// device health. The sender owns sessions, resume and quality; an optional scoped
// grant lets this screen report progress too, when the server is reachable.
// `customData.protocol` says which dialect a sender speaks.

const SUPPORTED_PROTOCOL = 1;
/** Where a sender asks this device what it decodes, and hears how playback goes. */
const NAMESPACE = "urn:x-cast:dev.noflex.cast";
const HEALTH_MS = 5000;
const CHROME_MS = 4000;

const context = cast.framework.CastReceiverContext.getInstance();
const player = context.getPlayerManager();
const messages = cast.framework.messages;
const EventType = cast.framework.events.EventType;

const $ = (id) => document.getElementById(id);
const body = document.body;

/** What the sender loaded, beyond what the media message itself says. */
let loaded = null;

// Phosphor's play and pause, filled.
const PLAY = "M240,128a15.74,15.74,0,0,1-7.6,13.51L88.32,229.65a16,16,0,0,1-16.2.3A15.86,15.86,0,0,1,64,216.13V39.87a15.86,15.86,0,0,1,8.12-13.82,16,16,0,0,1,16.2.3L232.4,114.49A15.74,15.74,0,0,1,240,128Z";
const PAUSE = "M216,48V208a16,16,0,0,1-16,16H160a16,16,0,0,1-16-16V48a16,16,0,0,1,16-16h40A16,16,0,0,1,216,48ZM96,32H56A16,16,0,0,0,40,48V208a16,16,0,0,0,16,16H96a16,16,0,0,0,16-16V48A16,16,0,0,0,96,32Z";

// ---- what the device decodes ------------------------------------------------

function capabilities() {
  return context.getDeviceCapabilities() || {};
}

/**
 * One question of the web client's own probe (`MediaQuestion` in
 * lib/ts/codecSupport.ts), put to this device's `canDisplayType`.
 */
function answer(question) {
  const caps = capabilities();
  const codecs = question.codecs || "";
  if (question.hdr === "pq-dynamic") return false;
  if (question.hdr && caps.is_hdr_supported === false) return false;
  if (/^(dvh1|dvhe|dav1|dvav)\./.test(codecs) && caps.is_dv_supported === false) return false;
  // canDisplayType has no channel count: Dolby is passed to the TV whole, the rest decode to 5.1 at most.
  if (question.channels && question.channels > (codecs === "ec-3" ? 8 : 6)) return false;
  try {
    return context.canDisplayType(question.mime, codecs || undefined, question.width, question.height, question.framerate);
  } catch {
    return false;
  }
}

// ---- drawing ------------------------------------------------------------------

function timecode(seconds) {
  const total = Math.max(0, Math.floor(seconds || 0));
  const h = Math.floor(total / 3600), m = Math.floor((total % 3600) / 60), s = total % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}` : `${m}:${String(s).padStart(2, "0")}`;
}

/** Source time: a linear transcode's zero is `offsetMs` into the title. */
const offset = () => (loaded && loaded.offsetMs ? loaded.offsetMs / 1000 : 0);
const length = () => (loaded && loaded.durationMs ? loaded.durationMs / 1000 : player.getDurationSec() || 0);

const CODEC = { H264: "H.264", H265: "HEVC", AV1: "AV1", VP9: "VP9", AAC: "AAC", AC3: "AC-3", EAC3: "E-AC-3", OPUS: "Opus", FLAC: "FLAC", TRUEHD: "TrueHD", DTS: "DTS" };
const RANGE = { HDR10: "HDR10", HDR10_PLUS: "HDR10+", HLG: "HLG", DOVI: "Dolby Vision", DOVI_WITH_HDR10: "Dolby Vision", DOVI_WITH_SDR: "Dolby Vision", DOVI_WITH_HLG: "Dolby Vision" };

function frame(width, height) {
  if (!width || !height) return null;
  const frames = [["360p", 640, 360], ["480p", 854, 480], ["720p", 1280, 720], ["1080p", 1920, 1080], ["1440p", 2560, 1440], ["4K", 3840, 2160]];
  const found = frames.find(([, w, h]) => width * 20 <= w * 21 && height * 20 <= h * 21);
  return found ? found[0] : "4K";
}

/** The chips under the title: how it's reaching the screen, and what it looks like. */
function badges() {
  const stream = loaded && loaded.stream;
  if (!stream) return [];
  const target = stream.target || {};
  const chips = [];
  if (stream.adaptive) chips.push({ text: `Auto${loaded.level ? ` · ${loaded.level}` : ""}`, accent: true });
  else if (stream.encodes) chips.push({ text: stream.quality ? `Transcoding · ${stream.quality}` : "Transcoding" });
  else chips.push({ text: stream.route === "OPTIMIZED" ? "Optimized" : "Original", accent: true });
  const size = frame(target.width, target.height);
  if (size && !stream.adaptive) chips.push({ text: size });
  if (RANGE[target.videoRange]) chips.push({ text: RANGE[target.videoRange] });
  if (target.videoCodec) chips.push({ text: CODEC[target.videoCodec] || target.videoCodec });
  if (target.audioCodec) chips.push({ text: [CODEC[target.audioCodec] || target.audioCodec, target.audioChannels > 2 ? `${target.audioChannels === 8 ? "7.1" : target.audioChannels === 6 ? "5.1" : `${target.audioChannels} ch`}` : null].filter(Boolean).join(" ") });
  return chips;
}

function drawNowPlaying(media = player.getMediaInformation()) {
  const metadata = (media && media.metadata) || {};
  $("title").textContent = metadata.title || "";
  $("subtitle").textContent = metadata.subtitle || "";
  const poster = metadata.images && metadata.images[0] && metadata.images[0].url;
  if (poster) $("poster").src = poster; else $("poster").removeAttribute("src");
  const backdrop = (loaded && loaded.backdrop) || poster;
  if (backdrop) $("backdrop").src = backdrop; else $("backdrop").removeAttribute("src");
  $("badges").replaceChildren(...badges().map((chip) => {
    const item = document.createElement("li");
    item.textContent = chip.text;
    if (chip.accent) item.className = "accent";
    return item;
  }));
}

function drawTime() {
  const now = offset() + (player.getCurrentTimeSec() || 0);
  const total = length();
  $("position").textContent = timecode(now);
  $("duration").textContent = timecode(total);
  $("played").style.width = total ? `${Math.min(100, (now / total) * 100)}%` : "0";
  const video = $("media");
  const ranges = video.buffered;
  const end = ranges.length ? offset() + ranges.end(ranges.length - 1) : 0;
  $("buffered").style.width = total ? `${Math.min(100, (end / total) * 100)}%` : "0";
}

let chromeTimer;
/** Bring the controls up; while playing they leave again after a moment. */
function showChrome() {
  body.classList.add("chrome-on");
  showProgressWarning();
  clearTimeout(chromeTimer);
  if (player.getPlayerState() === messages.PlayerState.PLAYING) {
    chromeTimer = setTimeout(() => body.classList.remove("chrome-on"), CHROME_MS);
  }
}

function pulse(path) {
  $("pulse-glyph").setAttribute("d", path);
  const element = document.querySelector(".pulse");
  element.classList.remove("on");
  void element.offsetWidth;
  element.classList.add("on");
}

let noticeTimer;
let noticeOwner = null;
const warnedWorks = new Set();
function notice(text, owner = "sender") {
  if (owner === "sender" && noticeOwner && noticeOwner !== "sender") return;
  const element = $("notice");
  element.textContent = text;
  element.classList.add("on");
  element.classList.toggle("warning", owner !== "sender");
  noticeOwner = owner;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => {
    element.classList.remove("on", "warning");
    noticeOwner = null;
    showProgressWarning();
  }, 5000);
}

function clearProgressWarning(load) {
  load.warningPending = false;
  if (noticeOwner !== load) return;
  clearTimeout(noticeTimer);
  $("notice").classList.remove("on", "warning");
  $("notice").textContent = "";
  noticeOwner = null;
}

function showProgressWarning() {
  const state = player.getPlayerState();
  if (!loaded || !loaded.warningPending || !loaded.reportingStarted || !loaded.started || buffering ||
      !body.classList.contains("chrome-on") ||
      (state !== messages.PlayerState.PLAYING && state !== messages.PlayerState.PAUSED)) return;
  loaded.warningPending = false;
  if (loaded.warningShown || (loaded.workId && warnedWorks.has(loaded.workId))) return;
  loaded.warningShown = true;
  if (loaded.workId) warnedWorks.add(loaded.workId);
  notice("This screen can’t save your place on its own. Keep NoFlex open on your phone or computer.", loaded);
}

const rate = (bps) => (bps ? `${(bps / 1e6).toFixed(bps >= 1e7 ? 0 : 1)} Mb/s` : "—");

function drawStats() {
  if (!body.classList.contains("stats-on")) return;
  const stream = (loaded && loaded.stream) || {};
  const stats = player.getStats() || {};
  const source = stream.source || {};
  const target = stream.target || {};
  const video = (s) => [CODEC[s.videoCodec] || s.videoCodec, s.width && `${s.width}×${s.height}`, RANGE[s.videoRange]].filter(Boolean).join(" · ") || "—";
  const audio = (s) => [CODEC[s.audioCodec] || s.audioCodec, s.audioChannels && `${s.audioChannels} ch`].filter(Boolean).join(" · ") || "—";
  const rows = [
    ["Mode", stream.mode || "—"],
    ...(stream.reasons && stream.reasons.length ? [["Why", stream.reasons.join("; ")]] : []),
    ["Video", video(source) === video(target) ? video(target) : `${video(source)} → ${video(target)}`],
    ["Audio", audio(source) === audio(target) ? audio(target) : `${audio(source)} → ${audio(target)}`],
    ...(stream.encoder ? [["Encoder", `${stream.encoder}${stream.hardware ? " (hardware)" : ""}`]] : []),
    ["Playing", stats.width ? `${stats.width}×${stats.height} at ${rate(stats.streamBandwidth)}` : "—"],
    ["Bandwidth", rate(stats.estimatedBandwidth)],
    ["Dropped", stats.decodedFrames ? `${stats.droppedFrames || 0} / ${stats.decodedFrames}` : "—"],
    ["Buffering", `${(stats.bufferingTime || 0).toFixed(1)} s`],
  ];
  $("stats").replaceChildren(...rows.flatMap(([term, value]) => {
    const dt = document.createElement("dt");
    dt.textContent = term;
    const dd = document.createElement("dd");
    dd.textContent = value;
    return [dt, dd];
  }));
}

function toggleStats(show) {
  body.classList.toggle("stats-on", show ?? !body.classList.contains("stats-on"));
  drawStats();
}

function drawState() {
  const state = player.getPlayerState();
  const idle = state === messages.PlayerState.IDLE && !loaded;
  if (loaded && noticeOwner === loaded && state === messages.PlayerState.BUFFERING) clearProgressWarning(loaded);
  body.classList.toggle("idle", idle);
  body.classList.toggle("loading", !!loaded && !loaded.started);
  body.classList.toggle("waiting", state === messages.PlayerState.BUFFERING || (!!loaded && state === messages.PlayerState.IDLE));
  $("toggle-glyph").setAttribute("d", state === messages.PlayerState.PAUSED ? PLAY : PAUSE);
  $("toggle").setAttribute("aria-label", state === messages.PlayerState.PAUSED ? "Play" : "Pause");
  if (state === messages.PlayerState.PAUSED || state === messages.PlayerState.BUFFERING) showChrome();
  else if (state === messages.PlayerState.PLAYING) showChrome();
  else if (idle) body.classList.remove("chrome-on");
}

// ---- telling the sender how it's going -------------------------------------

let buffering = false;
function health(kind) {
  const stats = player.getStats() || {};
  context.sendCustomMessage(NAMESPACE, undefined, {
    type: "health",
    kind,
    buffering,
    seeking: body.classList.contains("seeking"),
    estimatedBandwidthBps: stats.estimatedBandwidth || null,
    streamBandwidthBps: stats.streamBandwidth || null,
    width: stats.width || null,
    height: stats.height || null,
    droppedFrames: stats.droppedFrames || 0,
    decodedFrames: stats.decodedFrames || 0,
  });
}

// ---- best-effort source progress --------------------------------------------

let progressReports = Promise.resolve();

function progressGrant(value) {
  if (!value || typeof value.token !== "string" || !value.token || typeof value.url !== "string") return null;
  try {
    const url = new URL(value.url);
    if (!/^https?:$/.test(url.protocol) || url.username || url.password) return null;
    return { url: url.href, token: value.token, pending: null, busy: false };
  } catch {
    return null;
  }
}

function captureProgress(load, allowZero = false) {
  if (!load || load !== loaded || !load.reportingStarted || load.ended) return;
  const state = player.getPlayerState();
  if (state !== messages.PlayerState.PLAYING && state !== messages.PlayerState.PAUSED) return;
  const seconds = player.getCurrentTimeSec();
  // CAF can briefly report zero before resuming or while tearing down media.
  if (!Number.isFinite(seconds) || seconds < 0 || (seconds === 0 && !allowZero)) return;
  const position = Math.round((load.offsetMs || 0) + seconds * 1000);
  if (Number.isFinite(position) && position >= 0) load.lastPositionMs = Math.min(2147483647, position);
  const duration = load.durationMs || (load.offsetMs || 0) + player.getDurationSec() * 1000;
  if (Number.isFinite(duration) && duration > 0) load.lastDurationMs = duration;
}

function reportProgress(load, state) {
  const grant = load && load.progress;
  if (!grant || !load.reportingStarted || load.lastPositionMs === null) return;
  grant.pending = { positionMs: load.lastPositionMs, state };
  if (!grant.busy) {
    grant.busy = true;
    // Keep replacement reports behind the old title's final attempt.
    progressReports = progressReports.then(() => sendProgress(load, grant));
  }
}

async function sendProgress(load, grant) {
  while (grant.pending) {
    const sample = grant.pending;
    grant.pending = null;
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 8000);
    let reporting = false;
    try {
      const response = await fetch(grant.url, {
        method: "POST",
        mode: "cors",
        credentials: "omit",
        referrerPolicy: "no-referrer",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          query: "mutation ReceiverProgress($token:String!,$positionMs:Int!,$state:PlaybackState!){reportPlaybackProgress(token:$token,positionMs:$positionMs,state:$state){positionMs}}",
          variables: { token: grant.token, ...sample },
        }),
        signal: controller.signal,
        keepalive: true,
      });
      const result = await response.json();
      reporting = response.ok && !result.errors?.length && Number.isFinite(result.data?.reportPlaybackProgress?.positionMs);
    } catch {
      // Mixed content, local-network policy and CORS may all leave reporting to the sender.
    } finally {
      clearTimeout(timeout);
    }
    if (loaded === load && !load.ended) {
      if (reporting) clearProgressWarning(load);
      else {
        load.warningPending = true;
        showProgressWarning();
      }
      try { context.sendCustomMessage(NAMESPACE, undefined, { type: "progress", reporting }); } catch {}
    }
  }
  grant.busy = false;
  if (load.ended) {
    grant.token = null;
    grant.url = null;
  }
}

function playbackProgress(event) {
  if (!loaded || !loaded.started || loaded.ended) return;
  const state = event?.type === EventType.PAUSE ? messages.PlayerState.PAUSED
    : event?.type === EventType.PLAYING ? messages.PlayerState.PLAYING : player.getPlayerState();
  if (state !== messages.PlayerState.PLAYING && state !== messages.PlayerState.PAUSED) return;
  loaded.reportingStarted = true;
  captureProgress(loaded);
  const progressState = state === messages.PlayerState.PAUSED ? "PAUSED" : "PLAYING";
  if (loaded.progressState !== progressState) {
    loaded.progressState = progressState;
    reportProgress(loaded, progressState);
  }
  showProgressWarning();
}

function finishProgress(load, natural = false) {
  if (!load || load.ended) return;
  if (natural && load.reportingStarted && load.lastDurationMs > 0) {
    load.lastPositionMs = Math.min(2147483647, Math.round(load.lastDurationMs));
  }
  reportProgress(load, "STOPPED");
  load.ended = true;
  clearProgressWarning(load);
  const grant = load.progress;
  load.progress = null;
  if (grant && !grant.busy) {
    grant.token = null;
    grant.url = null;
  }
}

// ---- steering from the device itself ----------------------------------------

function seekBy(seconds) {
  const now = player.getCurrentTimeSec() || 0;
  const target = Math.max(0, now + seconds);
  if (loaded) loaded.zeroSeek = target === 0;
  player.seek(target);
  pulse(seconds > 0 ? $("forward").querySelector("path").getAttribute("d") : $("back").querySelector("path").getAttribute("d"));
  showChrome();
}

function togglePlay() {
  if (player.getPlayerState() === messages.PlayerState.PAUSED) { player.play(); pulse(PLAY); }
  else { player.pause(); pulse(PAUSE); }
  showChrome();
}

$("toggle").addEventListener("click", togglePlay);
$("back").addEventListener("click", () => seekBy(-10));
$("forward").addEventListener("click", () => seekBy(10));
document.addEventListener("pointerdown", (event) => { if (!event.target.closest("button")) showChrome(); });
// A TV with Cast built in can hand its remote's D-pad to the page.
document.addEventListener("keydown", (event) => {
  if (!loaded) return;
  const handled = {
    Enter: togglePlay, " ": togglePlay, MediaPlayPause: togglePlay,
    ArrowLeft: () => seekBy(-10), ArrowRight: () => seekBy(10),
    ArrowUp: () => toggleStats(), ArrowDown: showChrome,
  }[event.key];
  if (!handled) return;
  event.preventDefault();
  handled();
});

// ---- the player -------------------------------------------------------------

player.setMessageInterceptor(messages.MessageType.LOAD, (request) => {
  const media = request.media;
  if (!media) return request;
  const data = media.customData || {};
  if (data.protocol !== undefined && data.protocol > SUPPORTED_PROTOCOL) {
    // A newer sender than this receiver: play what it asked, as the media message alone says.
    console.warn("sender speaks protocol", data.protocol, "; this receiver speaks", SUPPORTED_PROTOCOL);
  }
  captureProgress(loaded);
  finishProgress(loaded);
  const offsetMs = Number.isFinite(data.offsetMs) ? Math.max(0, data.offsetMs) : 0;
  const resumeMs = Number.isFinite(request.currentTime) ? Math.max(0, request.currentTime * 1000) : 0;
  loaded = {
    offsetMs, durationMs: data.durationMs || null, backdrop: data.backdrop || null,
    stream: data.stream || null, level: null, started: false, workId: data.workId || null,
    progress: progressGrant(data.progress), reportingStarted: false,
    lastPositionMs: Math.min(2147483647, Math.round(offsetMs + resumeMs)),
    lastDurationMs: data.durationMs || null, ended: false, warningPending: false, warningShown: false,
  };
  // A growing transcode is still a film, not a live stream: seeking and the timeline must work.
  media.streamType = messages.StreamType.BUFFERED;
  if (/mpegurl/i.test(media.contentType || "") && data.stream) {
    const fmp4 = data.stream.container === "fmp4";
    media.hlsSegmentFormat = fmp4 ? messages.HlsSegmentFormat.FMP4 : messages.HlsSegmentFormat.TS;
    media.hlsVideoSegmentFormat = fmp4 ? messages.HlsVideoSegmentFormat.FMP4 : messages.HlsVideoSegmentFormat.MPEG2_TS;
  }
  drawNowPlaying(media);
  drawState();
  return request;
});

player.setMessageInterceptor(messages.MessageType.SEEK, (request) => {
  if (loaded) loaded.zeroSeek = request.currentTime === 0;
  return request;
});

player.addEventListener(EventType.PLAYER_LOAD_COMPLETE, () => {
  if (loaded) loaded.started = true;
  playbackProgress();
  drawNowPlaying();
  drawState();
  health("loaded");
});
player.addEventListener(EventType.BUFFERING, (event) => {
  buffering = event.isBuffering;
  drawState();
  health(buffering ? "stalled" : "resumed");
});
player.addEventListener(EventType.SEEKING, () => { body.classList.add("seeking"); showChrome(); });
player.addEventListener(EventType.SEEKED, () => {
  body.classList.remove("seeking");
  captureProgress(loaded, loaded?.zeroSeek === true);
  if (loaded) {
    loaded.zeroSeek = false;
    reportProgress(loaded, player.getPlayerState() === messages.PlayerState.PAUSED ? "PAUSED" : "PLAYING");
  }
  drawTime();
});
player.addEventListener(EventType.BITRATE_CHANGED, (event) => {
  if (!loaded) return;
  const stats = player.getStats() || {};
  loaded.level = frame(stats.width, stats.height);
  drawNowPlaying();
  health("switched");
  void event;
});
player.addEventListener([EventType.PLAY, EventType.PAUSE, EventType.PLAYING, EventType.ENDED, EventType.MEDIA_FINISHED], drawState);
player.addEventListener([EventType.PLAYING, EventType.PAUSE], playbackProgress);
player.addEventListener(EventType.MEDIA_FINISHED, (event) => {
  // LOAD installs the replacement before CAF finishes interrupting the old media.
  if (loaded && !loaded.started && event.endedReason === cast.framework.events.EndedReason.INTERRUPTED) return;
  if (loaded && loaded.reportingStarted && Number.isFinite(event.currentMediaTime) && event.currentMediaTime > 0) {
    loaded.lastPositionMs = Math.min(2147483647, Math.round(loaded.offsetMs + event.currentMediaTime * 1000));
  }
  finishProgress(loaded, event.endedReason === cast.framework.events.EndedReason.END_OF_STREAM);
  loaded = null;
  drawState();
});

setInterval(() => {
  if (!loaded) return;
  captureProgress(loaded);
  drawTime();
  drawStats();
}, 1000);
setInterval(() => {
  if (loaded && player.getPlayerState() === messages.PlayerState.PLAYING) health("tick");
}, HEALTH_MS);
setInterval(() => {
  const state = player.getPlayerState();
  if (loaded && (state === messages.PlayerState.PLAYING || state === messages.PlayerState.PAUSED)) {
    captureProgress(loaded);
    reportProgress(loaded, state === messages.PlayerState.PAUSED ? "PAUSED" : "PLAYING");
  }
}, 15000);

function flushProgress() {
  captureProgress(loaded);
  finishProgress(loaded);
}
window.addEventListener("pagehide", flushProgress);
window.addEventListener("beforeunload", flushProgress);
context.addEventListener(cast.framework.system.EventType.SHUTDOWN, flushProgress);

context.addCustomMessageListener(NAMESPACE, (event) => {
  const message = event.data || {};
  if (message.type === "capabilities") {
    // The web client's own walk, run here: one message for any sender, rather than a round trip per step.
    NoFlexCodecs.probeElsewhere(async (questions) => questions.map(answer), capabilities().is_hdr_supported === true).then(
      (profile) => context.sendCustomMessage(NAMESPACE, event.senderId, { type: "capabilities", id: message.id, capabilities: profile }),
      (cause) => context.sendCustomMessage(NAMESPACE, event.senderId, { type: "capabilities", id: message.id, error: String(cause) }),
    );
  } else if (message.type === "stats") {
    toggleStats(message.show);
  } else if (message.type === "notice" && message.text) {
    notice(String(message.text));
  }
});

context.addEventListener(cast.framework.system.EventType.READY, () => {
  body.classList.toggle("touchscreen", capabilities().touch_input_supported === true);
});

const options = new cast.framework.CastReceiverOptions();
options.customNamespaces = { [NAMESPACE]: cast.framework.system.MessageType.JSON };
// Our own player draws the controls, so a smart display mustn't expect Google's.
options.uiConfig = { touchScreenOptimizedApp: true };
// Shaka adapts along a ladder and estimates bandwidth, which Auto needs.
options.useShakaForHls = true;
options.playbackConfig = new cast.framework.PlaybackConfig();
// A transcode starts a little behind the sender's request, so give it room before it pauses.
options.playbackConfig.autoResumeDuration = 5;
// The server encodes a segment when it's asked for, and holds the request for up to 45 s.
options.playbackConfig.shakaConfig = {
  streaming: { retryParameters: { timeout: 50000, maxAttempts: 4 } },
  abr: { defaultBandwidthEstimate: 20000000 },
};
options.disableIdleTimeout = false;
options.maxInactivity = 3600;
context.start(options);
