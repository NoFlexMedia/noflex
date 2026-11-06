// SPDX-License-Identifier: AGPL-3.0-only
//
// A title's page: what it is, how to play it, and what's inside it. Everything an
// administrator would change about it stays in the web client.

import { useNavigate, useParams } from "@solidjs/router";
import { Column, Row } from "@solidtv/solid/primitives";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, untrack } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import { audioFormat, bytes, codecName, dynamicRange, levelName, profileName, runtime, techBadges, timecode, titleCase, versionLabel } from "$lib/format.js";
import { MARK_FOR_SMART_TRANSCODING, MARK_PLAYED, MARK_UNPLAYED, MEDIA_FILES, SET_FAVORITE, SMART_MARKS_FOR_WORK, UNMARK_FOR_SMART_TRANSCODING, WORK_CRUMB, WORK_DETAIL } from "$lib/graphql/documents.js";
import { createQuery, request } from "$lib/graphql.js";
import { latest } from "$lib/indexing.js";
import { pickImage } from "$lib/images.js";
import { fetchChildren, isPlayable, player, playWork, subtitleFor } from "$lib/playback.js";
import { revealInRow } from "$lib/rows.js";
import { app, thisDeviceId } from "$lib/session.js";
import { audioChoices, preferredAudio, preferredSubtitle, subtitleChoices } from "$lib/tracks.js";
import type { Edition, MediaFile, Work, SmartMarkState } from "$lib/types.js";
import { attempt, toast, ui } from "$lib/ui.js";
import { chapterStops } from "$lib/player/trickplay.js";
import { setUserData, userDataOf } from "$lib/watch.js";
import { color, motion, PAGE_WIDTH, PAGE_X, radius, SCREEN, type, weight } from "../theme.js";
import Icon from "$components/Icon.js";
import Button, { labelWidth } from "$components/Button.js";
import ChapterRail, { CHAPTER_HEIGHT } from "$components/player/ChapterRail.js";
import Rail, { railHeight } from "$components/Rail.js";
import Sheet from "$components/Sheet.js";
import Skeleton from "$components/Skeleton.js";
import State from "$components/State.js";
import Artwork from "$components/Artwork.js";

const HERO_HEIGHT = 760;
const POSTER_WIDTH = 340;
const INFO_X = POSTER_WIDTH + 60;
const INFO_WIDTH = PAGE_WIDTH - INFO_X - 64;
const PROGRESS_WIDTH = 220;
const FACE = 150;
const CARD = 230;
const CAST_HEIGHT = FACE + 90;
const SECTION_HEAD = 72;
const FACT_ROW = 88;
const FACT_WIDTH = (PAGE_WIDTH - 64) / 3;
const FACE_TINTS = [0x5a688cff, 0x7a5a8cff, 0x5a8c7aff, 0x8c7a5aff, 0x8c5a68ff, 0x5a7a8cff];
const CREW_ROLES = ["Director", "Writer", "Producer", "Composer"];

const initials = (name: string) => name.split(/\s+/).filter(Boolean).slice(0, 2).map((part) => part.charAt(0)).join("").toUpperCase();
const tintOf = (name: string) => FACE_TINTS[name.split("").reduce((sum, char) => sum + char.charCodeAt(0), 0) % FACE_TINTS.length];

function Badge(props: { label: string }) {
  return (
    <view width={labelWidth(props.label, 20) + 24} height={34} borderRadius={radius.xs} color={color.clear} border={{ width: 2, color: color.lineStrong }}>
      <text width={labelWidth(props.label, 20) + 24} height={34} lineHeight={34} contain="both" textAlign="center" fontSize={20} fontWeight={weight.semibold} color={color.text2}>{props.label}</text>
    </view>
  );
}

function CastCard(props: NodeProps & { name: string; detail?: string }) {
  const [focused, setFocused] = createSignal(false);
  let node: ElementNode | undefined;
  let revealing: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(revealing));
  return (
    <view
      {...props}
      ref={(element: ElementNode) => { node = element; }}
      width={CARD}
      height={CAST_HEIGHT}
      scale={focused() ? 1.06 : 1}
      zIndex={focused() ? 5 : 1}
      transition={{ scale: motion.spring }}
      onFocusChanged={(has: boolean) => {
        setFocused(has);
        clearTimeout(revealing);
        if (has) revealing = setTimeout(() => { if (node) revealInRow(node); }, 0);
      }}
    >
      <view x={(CARD - FACE) / 2} width={FACE} height={FACE} borderRadius={FACE / 2} color={tintOf(props.name)} border={{ width: 4, color: focused() ? color.focus : color.clear }}>
        <text width={FACE} height={FACE} lineHeight={FACE} contain="both" textAlign="center" fontSize={type.title} fontWeight={weight.bold}>{initials(props.name)}</text>
      </view>
      <text y={FACE + 14} width={CARD} contain="width" maxLines={1} textAlign="center" fontSize={type.caption} fontWeight={weight.semibold} color={focused() ? color.text : color.text2}>{props.name}</text>
      <Show when={props.detail}>
        <text y={FACE + 48} width={CARD} contain="width" maxLines={1} textAlign="center" fontSize={21} color={color.text3}>{props.detail}</text>
      </Show>
    </view>
  );
}
type SheetKind = "version" | "edition" | "audio" | "subtitle";

function Children(props: { work: Work; onUpNext: (next: { work: Work; resume?: boolean; rewatch?: boolean } | null) => void }) {
  const navigate = useNavigate();
  const [top, setTop] = createSignal<Work[]>([]);
  const [seasonId, setSeasonId] = createSignal<string | null>(null);
  const [episodes, setEpisodes] = createSignal<Work[]>([]);

  createEffect(on(() => props.work.id, () => {
    let stale = false;
    onCleanup(() => { stale = true; });
    (async () => {
      const children = await fetchChildren(props.work.libraryId, props.work.id);
      if (stale) return;
      setTop(children);
      const seasons = children.filter((child) => child.kind === "SEASON");
      setSeasonId(seasons[0]?.id ?? null);
      if (props.work.kind !== "SERIES" && props.work.kind !== "SEASON") return;
      // The episode to start from: in progress first, then the first unwatched, else the very first.
      const groups = seasons.length ? await Promise.all(seasons.map((season) => fetchChildren(props.work.libraryId, season.id))) : [children];
      if (stale) return;
      const all = groups.reduce((sum, group) => sum.concat(group), [] as Work[]);
      const started = all.find((item) => !item.userData?.played && (item.userData?.positionMs ?? 0) > 0);
      const next = started ?? all.find((item) => !item.userData?.played);
      props.onUpNext(next ? { work: next, resume: !!started } : all[0] ? { work: all[0], rewatch: true } : null);
    })().catch(() => {});
  }));

  createEffect(on(seasonId, (id) => {
    if (!id) { setEpisodes([]); return; }
    let stale = false;
    onCleanup(() => { stale = true; });
    fetchChildren(props.work.libraryId, id).then((list) => { if (!stale) setEpisodes(list); }).catch(() => {});
  }));

  const seasons = () => top().filter((child) => child.kind === "SEASON");
  const flat = () => top().filter((child) => child.kind !== "SEASON");
  const flatIsEpisodes = () => flat().some((child) => child.kind === "EPISODE" || child.kind === "TRACK");
  const flatTitle = () => (props.work.kind === "ARTIST" ? "Albums" : props.work.kind === "ALBUM" ? "Tracks" : "Episodes");
  const pickSeason = (season: Work) => {
    if (season.id === seasonId()) navigate(`/works/${season.id}`);
    else setSeasonId(season.id);
  };

  return (
    <>
      <Show when={seasons().length}>
        <Rail title="Seasons" items={seasons()} height={railHeight()} onChosen={pickSeason} />
      </Show>
      <Show when={episodes().length}>
        <Rail title={seasons().find((season) => season.id === seasonId())?.title ?? "Episodes"} items={episodes()} wide height={railHeight(true)} />
      </Show>
      <Show when={flat().length}>
        <Rail title={flatTitle()} items={flat()} wide={flatIsEpisodes()} height={railHeight(flatIsEpisodes())} />
      </Show>
    </>
  );
}

export default function Title() {
  const params = useParams();
  const navigate = useNavigate();
  const detail = createQuery<{ work: Work; editions: Edition[] }>(WORK_DETAIL, () => ({ id: params.id }));
  const work = () => detail.data()?.work;
  const editions = () => detail.data()?.editions ?? [];

  const [editionId, setEditionId] = createSignal<string | null>(null);
  createEffect(() => { if (!editions().some((edition) => edition.id === editionId())) setEditionId(editions()[0]?.id ?? null); });
  const edition = () => editions().find((item) => item.id === editionId());

  const media = createQuery<{ mediaFiles: MediaFile[] }>(MEDIA_FILES, () => (editionId() ? { edition: editionId() } : null));
  const files = () => media.data()?.mediaFiles ?? [];
  const [mediaId, setMediaId] = createSignal<string | null>(null);
  createEffect(() => { if (!files().some((file) => file.id === mediaId())) setMediaId(files()[0]?.id ?? null); });
  const file = () => files().find((item) => item.id === mediaId());

  const audioStreams = () => file()?.probe?.audio ?? [];
  const subtitleTracks = () => file()?.subtitleTracks ?? [];
  const [audio, setAudio] = createSignal<number | null>(null);
  const [subtitle, setSubtitle] = createSignal<number | null>(null);
  createEffect(on(mediaId, () => {
    setAudio(preferredAudio(audioStreams(), app.playbackPrefs));
    setSubtitle(preferredSubtitle(subtitleTracks(), app.playbackPrefs));
  }));

  const [upNext, setUpNext] = createSignal<{ work: Work; resume?: boolean; rewatch?: boolean } | null>(null);
  createEffect(on(() => params.id, () => setUpNext(null)));
  const [sheet, setSheet] = createSignal<SheetKind | null>(null);

  // A season or episode without art of its own wears its series'.
  // A match brings cast and tags as well as the card's fields.
  let wasIndexing = false;
  createEffect(() => {
    const current = work();
    const indexing = !!(current && latest(current).indexing);
    if (wasIndexing && !indexing) void detail.refetch();
    wasIndexing = indexing;
  });
  const parent = createQuery<{ work: Work }>(WORK_CRUMB, () => (work()?.parentId ? { id: work()!.parentId } : null));
  const heroArt = () => pickImage(work()?.images, "backdrop") ?? pickImage(parent.data()?.work.images, "backdrop");
  createEffect(() => { ui.backdrop = heroArt(); });
  onMount(() => { ui.backdropDim = true; });
  onCleanup(() => { ui.backdropDim = false; });

  const known = createMemo(() => (work() ? userDataOf(work()!) : null));
  const resumeMs = () => (known() && !known()!.played ? known()!.positionMs ?? 0 : 0);
  const canPlay = () => isPlayable(work());
  const genres = () => work()?.tags?.filter((tag) => tag.namespace === "genre").map((tag) => tag.display).slice(0, 4) ?? [];
  const parental = () => work()?.tags?.find((tag) => tag.namespace === "parental")?.display;
  const metaParts = () => [subtitleFor(work()) !== String(work()?.year) ? subtitleFor(work()) : "", work()?.year, runtime(edition()?.runtimeMs ?? work()?.runtimeMs), parental()].filter(Boolean).join("  ·  ");
  const chosenAudio = () => audioStreams()[audio() ?? 0];
  const badges = () => (canPlay() ? techBadges(file()?.probe, chosenAudio()) : []);
  const credits = () => work()?.credits ?? [];
  const crew = () => CREW_ROLES.map((role) => [role, credits().filter((credit) => credit.role === role).map((credit) => credit.name).join(", ")] as const).filter(([, names]) => names);
  const castCards = () => credits().slice(0, 30);
  const resumePercent = () => {
    const length = edition()?.runtimeMs ?? work()?.runtimeMs ?? 0;
    return length ? Math.min(1, resumeMs() / length) : 0;
  };
  const stops = () => {
    const current = edition();
    const chapters = current?.trickplay?.chapterCues?.length ? current.trickplay.chapterCues : current?.chapters;
    return chapters?.length ? chapterStops(chapters, current?.trickplay, current?.runtimeMs) : [];
  };
  const facts = () => {
    const video = file()?.probe?.video;
    if (!video) return [];
    const fps = video.frame_rate ? ` @ ${Number(video.frame_rate).toFixed(3).replace(/\.?0+$/, "")} fps` : "";
    const rows: [string, string | null | undefined][] = [
      ["Video", [codecName(video.codec), profileName(video.profile), levelName(video.codec, video.level)].filter(Boolean).join(" · ")],
      ["Resolution", video.width && video.height ? `${video.width} × ${video.height}${fps}` : null],
      ["Dynamic range", dynamicRange(video) || "SDR"],
      ["Bit depth", video.bit_depth ? `${video.bit_depth}-bit${video.pixel_format ? ` · ${String(video.pixel_format).toUpperCase()}` : ""}` : null],
      ["Audio", chosenAudio() ? audioFormat(chosenAudio()) : null],
      ["Container", file()?.container?.toUpperCase()],
      ["File size", bytes(file()?.sizeBytes)],
      ["Community score", work()?.communityRating ? `${work()!.communityRating!.toFixed(1)} / 10` : null],
    ];
    return rows.filter(([, value]) => value) as [string, string][];
  };
  const [tip, setTip] = createSignal("");
  const tipFor = (text: string) => (has: boolean) => setTip((shown) => (has ? text : shown === text ? "" : shown));

  const options = () => ({ editionId: editionId() ?? undefined, mediaFileId: mediaId() ?? undefined, audioStream: audio(), subtitleIndex: subtitle() });
  const start = (fromStart: boolean) => {
    const current = work();
    if (current) void playWork(current, { ...options(), fromStart });
    return true;
  };

  const toggleFavorite = () => {
    const current = work();
    if (!current) return true;
    void attempt(async () => {
      const data = await request(SET_FAVORITE, { id: current.id, favorite: !known()?.favorite });
      setUserData(current.id, data.setFavorite);
    });
    return true;
  };
  const toggleWatched = () => {
    const current = work();
    if (!current) return true;
    void attempt(async () => {
      if (known()?.played) setUserData(current.id, (await request(MARK_UNPLAYED, { id: current.id })).markUnplayed);
      else setUserData(current.id, (await request(MARK_PLAYED, { id: current.id })).markPlayed);
    });
    return true;
  };

  const MARK_STATE: Record<SmartMarkState, string> = { WAITING: "in the queue", PREPARING: "preparing", READY: "ready", NOT_NEEDED: "plays as it is", TOO_LARGE: "too large for this server", NO_DEVICES: "no device has it on", NO_SPACE: "waiting for space", FAILED: "failed", DISABLED: "off on this server" };
  const markState = (mark?: { state: SmartMarkState; progress: number }) => (!mark ? "" : mark.state === "PREPARING" ? `preparing ${Math.round(mark.progress * 100)}%` : MARK_STATE[mark.state]);
  // A mark for this TV alone, or for every device with smart transcoding on.
  const [marks, setMarks] = createSignal<{ device: boolean; here: boolean; anyDeviceOn: boolean; deviceState: string; hereState: string }>({ device: false, here: false, anyDeviceOn: true, deviceState: "", hereState: "" });
  const canMark = () => ["MOVIE", "SERIES", "SEASON", "EPISODE"].indexOf(work()?.kind ?? "") >= 0;
  const me = thisDeviceId();
  const loadMarks = (id: string) => request<{ mySmartTranscodeMarks: { device?: { id: string } | null; state: SmartMarkState; progress: number }[]; myDevices: { smartTranscode: boolean }[] }>(SMART_MARKS_FOR_WORK, { work: id }).then((data) => {
    setMarks({
      device: data.mySmartTranscodeMarks.some((mark) => !mark.device),
      here: data.mySmartTranscodeMarks.some((mark) => mark.device?.id === me),
      anyDeviceOn: data.myDevices.some((device) => device.smartTranscode),
      deviceState: markState(data.mySmartTranscodeMarks.find((mark) => !mark.device)),
      hereState: markState(data.mySmartTranscodeMarks.find((mark) => mark.device?.id === me)),
    });
  }, () => {});
  createEffect(on(() => params.id, (id) => { if (id) void loadMarks(id); }));
  const markTip = (thisTv: boolean) => (thisTv
    ? marks().here ? `For this TV: ${marks().hereState}. OK unmarks it` : "Smart transcode for this TV"
    : marks().device ? `For your devices: ${marks().deviceState}. OK unmarks it` : "Mark for smart transcoding");
  const toggleMark = (thisTv: boolean) => {
    const current = work();
    if (!current) return true;
    const marked = thisTv ? marks().here : marks().device;
    void attempt(async () => {
      await request(marked ? UNMARK_FOR_SMART_TRANSCODING : MARK_FOR_SMART_TRANSCODING, { work: current.id, device: thisTv ? me : null });
      await loadMarks(current.id);
      setTip(markTip(thisTv));
      if (marked) toast("Unmarked", "success");
      else if (thisTv) toast("It’ll be ready for this TV next", "success");
      else if (marks().anyDeviceOn) toast("Marked. It’ll be ready next on your devices with smart transcoding on", "success");
      else toast("Marked, but none of your devices has smart transcoding on. Turn it on in Settings", "info");
    });
    return true;
  };

  // Read once when opened: the values it picks would otherwise rebuild it mid-pick.
  const sheetFor = (kind: SheetKind) => {
    switch (kind) {
      case "edition": return { title: "Edition", choices: editions().map((item) => ({ value: item.id, label: item.name || "Standard" })), value: editionId() ?? "", pick: (value: string) => setEditionId(value) };
      case "version": return { title: "Version", choices: files().map((item) => ({ value: item.id, label: versionLabel(item.probe) ?? item.path.split("/").pop() ?? "File" })), value: mediaId() ?? "", pick: (value: string) => setMediaId(value) };
      case "audio": return { title: "Audio", choices: audioChoices(audioStreams()), value: String(audio() ?? 0), pick: (value: string) => setAudio(Number(value)) };
      case "subtitle": return { title: "Subtitles", choices: subtitleChoices(subtitleTracks()), value: subtitle() == null ? "off" : String(subtitle()), pick: (value: string) => setSubtitle(value === "off" ? null : Number(value)) };
    }
  };

  return (
    <view x={PAGE_X} width={PAGE_WIDTH} height={SCREEN.height} forwardFocus={0}>
      <Show when={work()} keyed fallback={
        <Show when={detail.error()} fallback={<Skeleton y={140} />}>
          <State areaWidth={PAGE_WIDTH} autofocus kind="error" title="This title is out of reach" message={detail.error()} action={() => void detail.refetch()} />
        </Show>
      }>
        {(current) => (
          <>
            <Column autofocus width={PAGE_WIDTH} height={SCREEN.height * 2} scroll="always" gap={8}>
              <view width={PAGE_WIDTH} height={HERO_HEIGHT} forwardFocus={0}>
                <Row x={INFO_X} y={596} width={INFO_WIDTH} height={80} gap={18} scroll="auto">
                  <Show when={canPlay()}>
                    <Button
                      size="lg"
                      kind="primary"
                      icon="play"
                      busy={player.preparing === current.id}
                      label={resumeMs() > 30_000 ? `Resume from ${timecode(resumeMs())}` : "Play"}
                      onEnter={() => start(false)}
                    />
                    <Show when={resumeMs() > 30_000}>
                      <Button size="lg" icon="clock-counter-clockwise" onFocused={tipFor("Play from the beginning")} onEnter={() => start(true)} />
                    </Show>
                  </Show>
                  {/* There from the start, so focus doesn't jump when the next episode is worked out. */}
                  <Show when={current.kind === "SERIES" || current.kind === "SEASON"}>
                    <Button
                      size="lg"
                      kind="primary"
                      icon="play"
                      busy={!upNext()}
                      label={upNext() ? `${upNext()!.resume ? "Resume" : upNext()!.rewatch ? "Watch again" : "Play"} ${subtitleFor(upNext()!.work)}` : "Play"}
                      onEnter={() => { const next = upNext(); if (next) void playWork(next.work); return true; }}
                    />
                  </Show>
                  <Button size="lg" icon={known()?.favorite ? "heart-fill" : "heart"} onFocused={tipFor(known()?.favorite ? "Remove from favorites" : "Add to favorites")} onEnter={toggleFavorite} />
                  <Button size="lg" icon={known()?.played ? "check-circle" : "check"} onFocused={tipFor(known()?.played ? "Mark as unwatched" : "Mark as watched")} onEnter={toggleWatched} />
                  <Show when={canMark()}>
                    <Button size="lg" icon="sparkle" kind={marks().device ? "primary" : "glass"} onFocused={tipFor(markTip(false))} onEnter={() => toggleMark(false)} />
                    <Button size="lg" icon="television" kind={marks().here ? "primary" : "glass"} onFocused={tipFor(markTip(true))} onEnter={() => toggleMark(true)} />
                  </Show>
                  <Show when={editions().length > 1}>
                    <Button size="lg" icon="stack" label={edition()?.name || "Edition"} onEnter={() => { setSheet("edition"); return true; }} />
                  </Show>
                  <Show when={canPlay() && files().length > 1}>
                    <Button size="lg" icon="film-strip" label="Version" onEnter={() => { setSheet("version"); return true; }} />
                  </Show>
                  <Show when={canPlay() && audioStreams().length > 1}>
                    <Button size="lg" icon="translate" label="Audio" onEnter={() => { setSheet("audio"); return true; }} />
                  </Show>
                  <Show when={canPlay() && subtitleTracks().length > 0}>
                    <Button size="lg" icon="subtitles" label="Subtitles" onEnter={() => { setSheet("subtitle"); return true; }} />
                  </Show>
                </Row>
                <Artwork work={current} kind={current.kind === "EPISODE" ? "thumb" : "poster"} y={72} width={POSTER_WIDTH} height={current.kind === "EPISODE" ? 191 : 510} skipFocus />
                <view x={INFO_X} y={72} width={INFO_WIDTH} height={510} display="flex" flexDirection="column" justifyContent="flexEnd" gap={14} skipFocus>
                  <text fontSize={22} fontWeight={weight.semibold} letterSpacing={3} color={color.text3}>{titleCase(current.kind).toUpperCase()}</text>
                  <text width={INFO_WIDTH} contain="width" maxLines={2} lineHeight={84} fontSize={72} fontWeight={weight.bold}>{current.title}</text>
                  <view width={INFO_WIDTH} height={36} display="flex" gap={14} alignItems="center">
                    <Show when={current.communityRating}>
                      <Icon name="star" size={28} color={color.warning} />
                      <text fontSize={type.caption} fontWeight={weight.semibold} color={color.warning}>{current.communityRating?.toFixed(1)}</text>
                    </Show>
                    <text fontSize={type.caption} fontWeight={weight.semibold} color={color.text2}>{metaParts()}</text>
                  </view>
                  <Show when={genres().length}>
                    <text width={INFO_WIDTH} contain="width" maxLines={1} fontSize={type.caption} color={color.text3}>{genres().join(" · ")}</text>
                  </Show>
                  <Show when={badges().length}>
                    <view width={INFO_WIDTH} height={34} display="flex" gap={10}>
                      <For each={badges()}>{(badge) => <Badge label={badge} />}</For>
                    </view>
                  </Show>
                  <Show when={current.tagline}>
                    <text width={INFO_WIDTH} contain="width" maxLines={1} fontSize={type.body} fontWeight={weight.semibold} color={color.text}>{current.tagline ?? ""}</text>
                  </Show>
                  <text width={1180} contain="width" maxLines={current.tagline ? 2 : 3} lineHeight={40} fontSize={type.body} color={color.text2}>{current.overview ?? ""}</text>
                </view>
                <Show when={canPlay() && resumePercent() > 0}>
                  <view x={INFO_X + 28} y={684} width={PROGRESS_WIDTH} height={6} borderRadius={3} color={color.surface3} skipFocus>
                    <view width={PROGRESS_WIDTH * resumePercent()} height={6} borderRadius={3} color={color.accent} />
                  </view>
                </Show>
                <text x={INFO_X} y={704} fontSize={22} color={color.text2}>{tip()}</text>
              </view>
              <Children work={current} onUpNext={setUpNext} />
              <Show when={stops().length}>
                <view width={PAGE_WIDTH} height={SECTION_HEAD + CHAPTER_HEIGHT + 40} forwardFocus={1}>
                  <text height={56} lineHeight={56} fontSize={type.lead} fontWeight={weight.semibold}>Chapters</text>
                  <ChapterRail y={SECTION_HEAD} width={PAGE_WIDTH - 64} stops={stops()} trick={edition()?.trickplay} onChosen={(stop) => { if (work()) void playWork(work()!, { ...options(), positionMs: stop.positionMs }); }} />
                </view>
              </Show>
              <Show when={credits().length}>
                <view width={PAGE_WIDTH} height={SECTION_HEAD + (crew().length ? 90 : 0) + CAST_HEIGHT + 40} forwardFocus={crew().length ? 2 : 1}>
                  <text height={56} lineHeight={56} fontSize={type.lead} fontWeight={weight.semibold}>Cast & Crew</text>
                  <Show when={crew().length}>
                    <view y={SECTION_HEAD - 6} width={PAGE_WIDTH - 64} height={70} display="flex" gap={56} skipFocus>
                      <For each={crew()}>
                        {([role, names]) => (
                          <view width={(PAGE_WIDTH - 64 - 56 * (crew().length - 1)) / crew().length} height={70}>
                            <text fontSize={20} fontWeight={weight.semibold} letterSpacing={2} color={color.text3}>{role.toUpperCase()}</text>
                            <text y={32} width={(PAGE_WIDTH - 64 - 56 * (crew().length - 1)) / crew().length} contain="width" maxLines={1} fontSize={type.caption}>{names}</text>
                          </view>
                        )}
                      </For>
                    </view>
                  </Show>
                  <Row y={SECTION_HEAD + (crew().length ? 90 : 0)} width={PAGE_WIDTH - 64} height={CAST_HEIGHT} gap={12} scroll="auto">
                    <For each={castCards()}>
                      {(person) => <CastCard name={person.name} detail={person.character ?? person.role ?? undefined} onEnter={() => { navigate(`/search?q=${encodeURIComponent(person.name)}`); return true; }} />}
                    </For>
                  </Row>
                </view>
              </Show>
              {/* Focusable with nothing to do: the column only scrolls to what can take focus. */}
              <Show when={facts().length}>
                <view width={PAGE_WIDTH} height={SECTION_HEAD + Math.ceil(facts().length / 3) * FACT_ROW + 200} onEnter={() => true}>
                  <text height={56} lineHeight={56} fontSize={type.lead} fontWeight={weight.semibold}>Technical details</text>
                  <For each={facts()}>
                    {([term, value], position) => (
                      <view x={(position() % 3) * FACT_WIDTH} y={SECTION_HEAD + Math.floor(position() / 3) * FACT_ROW} width={FACT_WIDTH - 32} height={FACT_ROW} skipFocus>
                        <text fontSize={20} fontWeight={weight.semibold} letterSpacing={2} color={color.text3}>{term.toUpperCase()}</text>
                        <text y={30} width={FACT_WIDTH - 32} contain="width" maxLines={1} fontSize={type.caption}>{value}</text>
                      </view>
                    )}
                  </For>
                </view>
              </Show>
            </Column>
            <Show when={current.kind === "PHOTO"}>
              <Artwork work={current} kind="poster" x={PAGE_WIDTH - 900} y={140} width={820} height={800} skipFocus />
            </Show>
          </>
        )}
      </Show>
      <Show when={sheet()} keyed>
        {(kind) => {
          const open = untrack(() => sheetFor(kind));
          return (
            <view x={-PAGE_X} width={SCREEN.width} height={SCREEN.height} forwardFocus={0}>
              <Sheet title={open.title} choices={open.choices} value={open.value} onPick={(value) => { open.pick(value); setSheet(null); }} onClose={() => setSheet(null)} />
            </view>
          );
        }}
      </Show>
    </view>
  );
}

