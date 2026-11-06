// SPDX-License-Identifier: AGPL-3.0-only
//
// The transport, over a video that lives behind the canvas. Timing rules are the
// web player's (web/src/components/Player.svelte): the timeline is in source time,
// a transcode's stream time zero being `startPositionMs` into it.

import { Row } from "@solidtv/solid/primitives";
import { activeElement, renderer, type ElementNode } from "@solidtv/solid";
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, untrack } from "solid-js";
import { timecode } from "$lib/format.js";
import { MARK_PLAYED, REPORT_PROGRESS } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import { host } from "$lib/host.js";
import { pickImage, sized } from "$lib/images.js";
import { closePlayer, isAudioOnly, nextSibling, type Playback, playWork, restartSession, stopSession } from "$lib/playback.js";
import type { AudioTrack, VideoPlayer } from "$lib/player/types.js";
import { cueAt, parseVtt, type Cue } from "$lib/player/vtt.js";
import { mediaUrl } from "$lib/tokens.js";
import { audioChoices, isImageSubtitle, subtitleChoices } from "$lib/tracks.js";
import type { Work } from "$lib/types.js";
import { toast } from "$lib/ui.js";
import { setUserData } from "$lib/watch.js";
import { color, gradient, GUTTER, motion, radius, SCREEN, toward, type, weight } from "../../theme.js";
import Button from "$components/Button.js";
import Icon from "$components/Icon.js";
import Sheet from "$components/Sheet.js";
import Spinner from "$components/Spinner.js";

const CHROME_MS = 4000;
const HEARTBEAT_MS = 15_000;
const SKIP = 10;
// Within what the transcoder has produced (plus a little it will reach soon) a
// seek is native; further, a new session starting at the target beats waiting.
const TRANSCODE_REACH = 30;
const SCRUB_COMMIT_MS = 600;
const BAR_WIDTH = SCREEN.width - GUTTER * 2;
const UP_NEXT_SECONDS = 10;
const THUMB_WIDTH = 384;
const CAPTION_SIZE = 44;
const CAPTION_LINE = 56;
const CAPTION_PAD = 10;

export default function Player(props: { playback: Playback }) {
  const playback = () => props.playback;
  let video: VideoPlayer | null = null;
  let root: ElementNode | undefined;
  let controls: ElementNode | undefined;
  let chromeTimer: ReturnType<typeof setTimeout> | undefined;
  let heartbeat: ReturnType<typeof setInterval> | undefined;
  let countdownTimer: ReturnType<typeof setInterval> | undefined;
  let restartTimer: ReturnType<typeof setTimeout> | undefined;
  let scrubTimer: ReturnType<typeof setTimeout> | undefined;
  let finalized = false;
  let closing = false;
  let cueIndex = 0;

  const [position, setPosition] = createSignal(0);
  const [length, setLength] = createSignal(0);
  const [paused, setPaused] = createSignal(false);
  const [waiting, setWaiting] = createSignal(true);
  const [chrome, setChrome] = createSignal(true);
  const [error, setError] = createSignal("");
  const [scrub, setScrub] = createSignal<number | null>(null);
  const [restarting, setRestarting] = createSignal<number | null>(null);
  const [next, setNext] = createSignal<Work | null>(null);
  const [countdown, setCountdown] = createSignal<number | null>(null);
  const [tracks, setTracks] = createSignal<AudioTrack[]>([]);
  const [audioTrack, setAudioTrack] = createSignal(-1);
  const [sheet, setSheet] = createSignal<"audio" | "subtitle" | null>(null);
  const [subtitleIndex, setSubtitleIndex] = createSignal<number | null>(untrack(() => props.playback.subtitleIndex ?? null));
  const [cues, setCues] = createSignal<Cue[]>([]);
  const [caption, setCaption] = createSignal("");

  const live = () => !!playback().live || !playback().workId;
  const edition = () => playback().edition;
  const transcoding = () => playback().route === "TRANSCODE";
  const offset = () => (transcoding() ? (playback().startPositionMs ?? 0) / 1000 : 0);
  const sessionId = () => playback().sessionId ?? playback().jobId;
  const mediaTime = () => restarting() ?? offset() + (video?.position() ?? 0);
  const positionMs = () => Math.max(0, Math.round(mediaTime() * 1000));
  // The growing HLS duration would make the timeline creep; the runtime is the real length.
  const timeline = () => {
    const own = video?.duration() ?? 0;
    if (!transcoding()) return own;
    return (edition()?.runtimeMs ?? playback().work?.runtimeMs ?? 0) / 1000 || offset() + own;
  };
  const shown = () => scrub() ?? position();
  const fraction = () => (length() > 0 ? Math.min(1, shown() / length()) : 0);
  const inIntro = () => {
    const end = edition()?.introEndMs;
    return end != null && position() * 1000 >= (edition()?.introStartMs ?? 0) && position() * 1000 < end - 1500;
  };
  const inCredits = () => edition()?.creditsStartMs != null && length() > 0 && position() * 1000 >= edition()!.creditsStartMs!;
  const delivered = () => playback().subtitles ?? [];
  // Measured ahead of the renderer, which would answer a frame late; Inter averages about half an em.
  const captionBox = () => {
    const lines = caption().split("\n");
    const longest = lines.reduce((most, line) => Math.max(most, line.length), 0);
    return { width: Math.min(SCREEN.width - GUTTER * 2, longest * CAPTION_SIZE * 0.52 + 48), height: lines.length * CAPTION_LINE + CAPTION_PAD * 2 };
  };

  function showChrome(): void {
    setChrome(true);
    clearTimeout(chromeTimer);
    chromeTimer = setTimeout(() => { if (!video?.paused() && scrub() == null && !sheet()) setChrome(false); }, CHROME_MS);
  }

  async function report(state: "PLAYING" | "PAUSED" | "STOPPED"): Promise<void> {
    const workId = playback().workId;
    if (!workId || live() || restarting() != null) return;
    try {
      const data = await request(REPORT_PROGRESS, { id: workId, position: positionMs(), state });
      setUserData(workId, data.reportProgress);
    } catch { /* best effort */ }
  }

  async function finish(): Promise<void> {
    if (finalized) return;
    finalized = true;
    clearInterval(heartbeat);
    await report("STOPPED");
    await stopSession(sessionId());
  }

  async function close(): Promise<void> {
    if (closing) return;
    closing = true;
    await finish();
    closePlayer();
  }

  async function playNext(): Promise<void> {
    const target = next();
    cancelCountdown();
    if (!target) return close();
    await finish();
    if (!(await playWork(target, { fromStart: true }))) closePlayer();
  }

  function startCountdown(): void {
    setCountdown(UP_NEXT_SECONDS);
    showChrome();
    countdownTimer = setInterval(() => {
      const left = (countdown() ?? 1) - 1;
      setCountdown(left);
      if (left <= 0) void playNext();
    }, 1000);
  }
  function cancelCountdown(): void {
    clearInterval(countdownTimer);
    countdownTimer = undefined;
    setCountdown(null);
  }

  // ---- stream lifecycle -----------------------------------------------------
  createEffect(on(() => playback().url, (url) => {
    finalized = false;
    setError("");
    setWaiting(true);
    let disposed = false;
    let current: VideoPlayer | null = null;
    onCleanup(() => {
      disposed = true;
      clearInterval(heartbeat);
      current?.destroy();
      if (video === current) video = null;
    });
    void host.createPlayer().then((created) => {
      if (disposed) { created.destroy(); return; }
      current = created;
      video = created;
      created.on((event) => {
        switch (event.type) {
          case "ready":
            setLength(timeline());
            { // A transcode already starts at the resume point; a file seeks to it.
              const resume = (playback().resumeMs ?? 0) / 1000 - offset();
              if (resume > 1 && created.position() < 1) created.seek(resume);
            }
            break;
          case "playing":
            setRestarting(null);
            setWaiting(false);
            setPaused(false);
            clearInterval(heartbeat);
            void report("PLAYING");
            heartbeat = setInterval(() => void report("PLAYING"), HEARTBEAT_MS);
            break;
          case "paused":
            if (restarting() != null) break;
            setPaused(true);
            clearInterval(heartbeat);
            if (!finalized) void report("PAUSED");
            showChrome();
            break;
          case "waiting":
            setWaiting(true);
            break;
          case "time":
            if (restarting() != null) break;
            setPosition(mediaTime());
            setLength(timeline());
            if (cues().length) {
              const found = cueAt(cues(), created.position(), cueIndex);
              cueIndex = found.index;
              setCaption(found.text);
            }
            if (inCredits() && next() && countdown() == null && !countdownTimer && !created.paused()) startCountdown();
            break;
          case "ended":
            // The server infers completion from progress, but only with a known runtime.
            if (playback().workId && !live()) {
              const workId = playback().workId!;
              request(MARK_PLAYED, { id: workId }).then((data) => setUserData(workId, data.markPlayed), () => {});
            }
            if (next()) void playNext(); else void close();
            break;
          case "tracks":
            setTracks(event.tracks);
            setAudioTrack(event.current);
            break;
          case "error":
            setError(event.message);
            setWaiting(false);
            break;
        }
      });
      return created.load({ url, hls: playback().route !== "DIRECT", live: live(), audioStream: playback().audioStream });
    }).catch(() => { if (!disposed) setError("This device has no way to play this stream."); });
  }));

  // The next episode: this component stays mounted, and starts over.
  createEffect(on(() => playback().mediaFileId, () => {
    setSubtitleIndex(playback().subtitleIndex ?? null);
    setPosition((playback().resumeMs ?? 0) / 1000);
    setTracks([]);
    cancelCountdown();
  }, { defer: true }));

  createEffect(on(() => playback().work, (work) => {
    setNext(null);
    if (!work || untrack(live)) return;
    let stale = false;
    onCleanup(() => { stale = true; });
    void nextSibling(work).then((item) => { if (!stale) setNext(item); });
  }));

  // Captions draw on the canvas, from the file the server converted. A burned-in track has none to fetch.
  createEffect(() => {
    const index = subtitleIndex();
    const burned = playback().burnedSubtitle;
    setCues([]);
    setCaption("");
    cueIndex = 0;
    if (index == null || burned != null) return;
    const track = delivered().find((item) => item.index === index);
    if (!track) return;
    let stale = false;
    onCleanup(() => { stale = true; });
    fetch(track.url).then((response) => response.text()).then((text) => { if (!stale) setCues(parseVtt(text)); }).catch(() => { if (!stale) toast("Those subtitles couldn’t be loaded.", "error"); });
  });

  // Paused or not: a film left paused still shouldn't wake to a screensaver, and leaving ends it.
  const returnTo = untrack(activeElement) as ElementNode | undefined;
  onMount(() => {
    root?.setFocus();
    showChrome();
    host.keepAwake?.(true);
  });
  onCleanup(() => {
    host.keepAwake?.(false);
    if (returnTo && !returnTo.destroyed) returnTo.setFocus();
    clearTimeout(chromeTimer);
    clearTimeout(restartTimer);
    clearTimeout(scrubTimer);
    clearInterval(countdownTimer);
    clearInterval(heartbeat);
    // Whatever closed us, the server should stop encoding.
    void finish();
  });

  // ---- controls -------------------------------------------------------------
  function toggle(): void {
    if (!video) return;
    if (video.paused()) video.play(); else video.pause();
    showChrome();
  }

  function restartAt(target: number): void {
    setRestarting(target);
    setWaiting(true);
    clearTimeout(restartTimer);
    // Debounced so a run of skips opens one session, not several.
    restartTimer = setTimeout(async () => {
      clearInterval(heartbeat);
      if (!(await restartSession(playback().burnedSubtitle ?? null, Math.round(target * 1000)))) {
        setRestarting(null);
        setWaiting(false);
      }
    }, 350);
  }

  function seekTo(seconds: number): void {
    if (!video || live()) return;
    const target = Math.max(0, Math.min(length() || seconds, seconds));
    setPosition(target);
    const local = target - offset();
    if (!transcoding() || (local >= 0 && local <= video.seekableEnd() + TRANSCODE_REACH)) {
      clearTimeout(restartTimer);
      if (restarting() == null) video.seek(local);
      else restartAt(target);
      return;
    }
    restartAt(target);
  }

  // A held key walks the scrub head; the seek lands when it stops, so the decoder isn't asked for every step.
  function nudge(direction: number, repeat: boolean): boolean {
    if (live()) { showChrome(); return true; }
    const step = repeat ? 30 : SKIP;
    setScrub(Math.max(0, Math.min(length() || Infinity, (scrub() ?? position()) + direction * step)));
    showChrome();
    clearTimeout(scrubTimer);
    scrubTimer = setTimeout(() => {
      const target = scrub();
      setScrub(null);
      if (target != null) seekTo(target);
      showChrome();
    }, SCRUB_COMMIT_MS);
    return true;
  }

  const onBack = () => {
    if (sheet()) { setSheet(null); return true; }
    if (scrub() != null) { clearTimeout(scrubTimer); setScrub(null); showChrome(); return true; }
    if (countdown() != null) { cancelCountdown(); return true; }
    if (chrome() && !paused() && controlsFocused()) { root?.setFocus(); setChrome(false); return true; }
    void close();
    return true;
  };

  const [controlsFocused, setControlsFocused] = createSignal(false);
  const focusControls = () => { showChrome(); controls?.setFocus(); return true; };

  // ---- trickplay ------------------------------------------------------------
  const trick = () => edition()?.trickplay;
  const preview = createMemo(() => {
    const time = scrub();
    const data = trick();
    if (time == null || !data?.sheets?.length) return null;
    const perSheet = data.tileColumns * data.tileRows;
    const index = Math.floor((time * 1000) / data.intervalMs);
    const page = data.sheets.find((item) => item.index === Math.floor(index / perSheet));
    if (!page) return null;
    const within = index % perSheet;
    return {
      texture: renderer.createTexture("SubTexture", {
        texture: renderer.createTexture("ImageTexture", { src: mediaUrl(page.url) }),
        x: (within % data.tileColumns) * data.thumbWidth,
        y: Math.floor(within / data.tileColumns) * data.thumbHeight,
        w: data.thumbWidth,
        h: data.thumbHeight,
      }),
      height: THUMB_WIDTH * (data.thumbHeight / data.thumbWidth),
    };
  });
  const headX = () => GUTTER + BAR_WIDTH * fraction();
  const thumbX = () => Math.max(GUTTER, Math.min(SCREEN.width - GUTTER - THUMB_WIDTH, headX() - THUMB_WIDTH / 2));

  // ---- track pickers --------------------------------------------------------
  const audioOptions = () => (tracks().length > 1 ? tracks().map((track) => ({ value: String(track.id), label: track.label })) : audioChoices(playback().audioStreams));
  const pickAudio = (value: string) => {
    if (tracks().length > 1) { video?.setAudioTrack(Number(value)); return; }
    // A transcode carries one track; another means a new session.
    void restartSession(playback().burnedSubtitle ?? null, positionMs(), Number(value));
  };
  const pickSubtitle = (value: string) => {
    const index = value === "off" ? null : Number(value);
    setSubtitleIndex(index);
    const track = index == null ? null : playback().subtitleTracks?.[index];
    const needsBurn = index != null && (isImageSubtitle(track) || !delivered().some((item) => item.index === index));
    // Text tracks switch in place; entering or leaving burn-in needs a new session.
    if (needsBurn || playback().burnedSubtitle != null) void restartSession(needsBurn ? index : null, positionMs());
  };
  const canPickAudio = () => !live() && audioOptions().length > 1;
  const canPickSubtitles = () => !live() && (playback().subtitleTracks?.length ?? 0) > 0;
  const art = () => (isAudioOnly(playback().work) ? pickImage(playback().work?.images, "poster") : null);

  return (
    <view
      ref={(node: ElementNode) => { root = node; }}
      width={SCREEN.width}
      height={SCREEN.height}
      zIndex={120}
      onBack={onBack}
      onEnter={() => { if (!chrome()) showChrome(); else toggle(); return true; }}
      onLeft={(event: KeyboardEvent) => nudge(-1, !!event?.repeat)}
      onRight={(event: KeyboardEvent) => nudge(1, !!event?.repeat)}
      onUp={focusControls}
      onDown={focusControls}
      onPlayPause={() => { toggle(); return true; }}
      onPlay={() => { video?.play(); showChrome(); return true; }}
      onPause={() => { video?.pause(); return true; }}
      onStop={() => { void close(); return true; }}
      onFastForward={(event: KeyboardEvent) => nudge(1, !!event?.repeat)}
      onRewind={(event: KeyboardEvent) => nudge(-1, !!event?.repeat)}
    >
      <Show when={art()}>
        <view width={SCREEN.width} height={SCREEN.height} color={color.bg} skipFocus />
        <view src={sized(art()!, 640, 640)} x={(SCREEN.width - 640) / 2} y={120} width={640} height={640} borderRadius={radius.l} skipFocus />
      </Show>

      <Show when={caption()}>
        <view x={(SCREEN.width - captionBox().width) / 2} y={(chrome() ? 800 : 1000) - captionBox().height} width={captionBox().width} height={captionBox().height} borderRadius={radius.s} color={0x000000b3} transition={{ y: motion.out }} skipFocus>
          <text y={CAPTION_PAD} width={captionBox().width} contain="width" textAlign="center" lineHeight={CAPTION_LINE} fontSize={CAPTION_SIZE} fontWeight={weight.semibold}>{caption()}</text>
        </view>
      </Show>

      <Show when={waiting() && !error()}>
        <Spinner x={SCREEN.width / 2 - 36} y={SCREEN.height / 2 - 36} />
      </Show>

      <Show when={error()}>
        <view x={SCREEN.width / 2 - 460} y={SCREEN.height / 2 - 120} width={920} height={240} borderRadius={radius.l} color={0x16181ef2} skipFocus>
          <Icon name="warning-circle" size={56} x={432} y={32} color={color.danger} />
          <text y={104} width={920} contain="width" textAlign="center" fontSize={type.lead} fontWeight={weight.semibold}>{error()}</text>
          <text y={164} width={920} contain="width" textAlign="center" fontSize={type.caption} color={color.text2}>Press Back to return.</text>
        </view>
      </Show>

      {/* Left and Right off either end of the buttons mustn't reach the scrub handlers above. */}
      <view width={SCREEN.width} height={SCREEN.height} alpha={chrome() ? 1 : 0} transition={{ alpha: motion.out }} forwardFocus={3} onLeft={controlsFocused} onRight={controlsFocused}>
        <view width={SCREEN.width} height={240} linearGradient={gradient([0x000000cc, 0x00000000], toward.down)} skipFocus />
        <view y={SCREEN.height - 400} width={SCREEN.width} height={400} linearGradient={gradient([0x00000000, 0x000000e6], toward.down)} skipFocus />
        <view skipFocus>
          <text x={GUTTER} y={52} width={1400} contain="width" maxLines={1} fontSize={type.title} fontWeight={weight.bold}>{playback().title}</text>
          <text x={GUTTER} y={108} fontSize={type.caption} color={color.text2}>{playback().subtitle ?? (live() ? "Live" : "")}</text>
          <Show when={live()}>
            <view x={GUTTER} y={150} width={16} height={16} borderRadius={8} color={color.live} />
          </Show>
          <Show when={!live()}>
            <view x={GUTTER} y={SCREEN.height - 200} width={BAR_WIDTH} height={10} borderRadius={5} color={0xffffff38}>
              <view width={BAR_WIDTH * fraction()} height={10} borderRadius={5} color={color.accent} />
            </view>
            <view x={headX() - 14} y={SCREEN.height - 209} width={28} height={28} borderRadius={14} color={color.light} alpha={scrub() != null ? 1 : 0} />
            <text x={GUTTER} y={SCREEN.height - 180} fontSize={type.caption} fontWeight={weight.semibold}>{timecode(shown() * 1000)}</text>
            <text x={SCREEN.width - GUTTER - 240} y={SCREEN.height - 180} width={240} contain="width" textAlign="right" fontSize={type.caption} color={color.text2}>{`-${timecode((length() - shown()) * 1000)}`}</text>
          </Show>
          <Show when={preview()}>
            {(thumb) => (
              <view x={thumbX()} y={SCREEN.height - 240 - thumb().height} width={THUMB_WIDTH} height={thumb().height} borderRadius={radius.s} texture={thumb().texture} />
            )}
          </Show>
        </view>
        <Row
          ref={(node: ElementNode) => { controls = node; }}
          x={GUTTER}
          y={SCREEN.height - 124}
          width={BAR_WIDTH}
          height={80}
          gap={18}
          scroll="none"
          onFocusChanged={setControlsFocused}
          onLeft={() => { showChrome(); return false; }}
          onRight={() => { showChrome(); return false; }}
        >
          <Button icon={paused() ? "play" : "pause"} kind="light" size="lg" onEnter={() => { toggle(); return true; }} />
          <Show when={!live()}>
            <Button icon="arrow-counter-clockwise" size="lg" onEnter={() => { seekTo(position() - SKIP); showChrome(); return true; }} />
            <Button icon="arrow-clockwise" size="lg" onEnter={() => { seekTo(position() + SKIP); showChrome(); return true; }} />
          </Show>
          <Show when={inIntro()}>
            <Button icon="skip-forward" size="lg" label="Skip intro" onEnter={() => { seekTo(edition()!.introEndMs! / 1000); showChrome(); return true; }} />
          </Show>
          <Show when={next()}>
            <Button icon="skip-forward" size="lg" label={countdown() != null ? `Next in ${countdown()}` : "Next"} kind={countdown() != null ? "primary" : "glass"} onEnter={() => { void playNext(); return true; }} />
          </Show>
          <Show when={canPickAudio()}>
            <Button icon="translate" size="lg" label="Audio" onEnter={() => { setSheet("audio"); return true; }} />
          </Show>
          <Show when={canPickSubtitles()}>
            <Button icon="subtitles" size="lg" label="Subtitles" onEnter={() => { setSheet("subtitle"); return true; }} />
          </Show>
        </Row>
      </view>

      <Show when={sheet() === "audio"}>
        <Sheet title="Audio" choices={audioOptions()} value={tracks().length > 1 ? String(audioTrack()) : String(playback().audioStream ?? 0)} onPick={(value) => { pickAudio(value); setSheet(null); showChrome(); }} onClose={() => setSheet(null)} />
      </Show>
      <Show when={sheet() === "subtitle"}>
        <Sheet title="Subtitles" choices={subtitleChoices(playback().subtitleTracks)} value={subtitleIndex() == null ? "off" : String(subtitleIndex())} onPick={(value) => { pickSubtitle(value); setSheet(null); showChrome(); }} onClose={() => setSheet(null)} />
      </Show>
    </view>
  );
}
